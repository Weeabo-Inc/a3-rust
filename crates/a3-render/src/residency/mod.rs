//! Streamed textures: ref-counted handles, asynchronous loading, mip streaming and LRU eviction
//! under a GPU memory budget.
//!
//! Renderers ask a [`TextureResidency`] for textures by key (a VFS path) and get a
//! [`TextureHandle`]; holding a clone of the handle keeps the texture referenced. The manager
//! first loads the low-detail tail of the mip chain (mips up to
//! [`ResidencyConfig::tail_size`] pixels) on worker threads, then refines towards the finest mip
//! anyone [wants](TextureHandle::want_mip) this frame. Unreferenced textures stay cached until
//! the [budget](ResidencyConfig::budget_bytes) needs their memory (least recently used first);
//! referenced ones holding finer mips than they currently need are coarsened next.
//!
//! Uploads happen in [`TextureResidency::update`], once per frame, limited to
//! [`ResidencyConfig::upload_bytes_per_frame`] so arrivals never stall a frame. Refinements copy
//! the already resident mips on the GPU instead of reloading them.
//!
//! Textures are produced by a [`TextureSource`]; [`PaaSource`] reads PAA files.
//! [`MeshDraw`](crate::MeshDraw)s use streamed textures through
//! [`TextureHandle::texture_id`]; the renderer then also computes their screen-space mip need.
//! Custom features resolve views with [`TextureResidency::view`] in `prepare`.

mod paa;
mod plan;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

pub use paa::{FileReader, PaaSource};
pub use plan::{MipRequest, TextureInfo, required_mip};

use crate::draw::TextureId;
use crate::texture::{ColorSpace, MipLayout};

/// Mips loaded by a [`TextureSource`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedMips {
    pub info: TextureInfo,
    /// Level of `mips[0]`.
    pub first_mip: u32,
    /// Consecutive mips from `first_mip`, finest first, in the layout of
    /// [`TextureData`](crate::TextureData) mips.
    pub mips: Vec<Vec<u8>>,
}

impl LoadedMips {
    fn byte_len(&self) -> u64 {
        self.mips.iter().map(|m| m.len() as u64).sum()
    }
}

/// Produces texture mips; called on loader threads.
pub trait TextureSource: Send + Sync + 'static {
    /// Load the mips `request` asks for. For [`MipRequest::Tail`] the source picks the first mip
    /// with [`TextureInfo::tail_start`]; for [`MipRequest::Range`] it returns exactly
    /// `first..end`.
    fn load(&self, key: &str, request: MipRequest) -> Result<LoadedMips, String>;
}

/// Residency tuning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResidencyConfig {
    /// GPU bytes all streamed textures may occupy.
    pub budget_bytes: u64,
    /// Largest edge of the first, low-detail load.
    pub tail_size: u32,
    /// Most loads in flight.
    pub max_pending: usize,
    /// Upload limit per [`update`](TextureResidency::update); at least one arrival is always
    /// applied.
    pub upload_bytes_per_frame: u64,
    /// Loader threads.
    pub loader_threads: usize,
}

impl Default for ResidencyConfig {
    fn default() -> Self {
        ResidencyConfig {
            budget_bytes: 1 << 30,
            tail_size: 64,
            max_pending: 64,
            upload_bytes_per_frame: 32 << 20,
            loader_threads: 2,
        }
    }
}

/// Counters for overlays and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResidencyStats {
    /// Textures known to the manager (referenced or cached).
    pub textures: usize,
    /// Textures with at least one handle.
    pub referenced: usize,
    pub resident_bytes: u64,
    pub pending: usize,
    pub failed: usize,
    /// Textures evicted since creation.
    pub evictions: u64,
    /// Bytes uploaded by the last update.
    pub uploaded_bytes: u64,
}

struct Shared {
    id: u32,
    generation: u32,
    /// Finest mip wanted since the last update (`u32::MAX`: none).
    wanted: AtomicU32,
    used: AtomicBool,
}

/// A reference to a streamed texture. Clones share the reference; the texture becomes
/// evictable when the last handle is dropped.
#[derive(Clone)]
pub struct TextureHandle(Arc<Shared>);

impl std::fmt::Debug for TextureHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextureHandle({}#{})", self.0.id, self.0.generation)
    }
}

impl TextureHandle {
    /// Ask for mip `level` (0 = full detail) or finer this frame. Calls within one frame keep
    /// the finest level. Also marks the texture as used.
    pub fn want_mip(&self, level: u32) {
        self.0.wanted.fetch_min(level, Ordering::Relaxed);
        self.0.used.store(true, Ordering::Relaxed);
    }

    /// Mark the texture as used this frame without asking for more detail.
    pub fn touch(&self) {
        self.0.used.store(true, Ordering::Relaxed);
    }

    /// The id to put in a [`MeshDraw`](crate::MeshDraw).
    pub fn texture_id(&self) -> TextureId {
        TextureId::streamed(self.0.id, self.0.generation)
    }
}

struct GpuTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Level of the texture's mip 0 in the full chain.
    top: u32,
    material: Option<wgpu::BindGroup>,
}

struct Slot {
    shared: Arc<Shared>,
    key: Arc<str>,
    color_space: ColorSpace,
    info: Option<TextureInfo>,
    gpu: Option<GpuTexture>,
    pending: bool,
    failed: bool,
    last_used: u64,
}

struct Job {
    id: u32,
    generation: u32,
    key: Arc<str>,
    request: MipRequest,
}

struct JobResult {
    id: u32,
    generation: u32,
    request: MipRequest,
    result: Result<LoadedMips, String>,
}

/// The streamed-texture manager. See the [module docs](self).
pub struct TextureResidency {
    config: ResidencyConfig,
    slots: Vec<Option<Slot>>,
    generations: Vec<u32>,
    free: Vec<u32>,
    by_key: HashMap<Arc<str>, u32>,
    jobs: Option<Sender<Job>>,
    results: Receiver<JobResult>,
    ready: VecDeque<JobResult>,
    frame: u64,
    evictions: u64,
    uploaded_bytes: u64,
}

impl TextureResidency {
    /// Start a manager with loader threads reading from `source`.
    pub fn new(source: Arc<dyn TextureSource>, config: ResidencyConfig) -> TextureResidency {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (result_tx, result_rx) = mpsc::channel();
        let job_rx = Arc::new(Mutex::new(job_rx));
        for n in 0..config.loader_threads.max(1) {
            let (jobs, results, source) = (job_rx.clone(), result_tx.clone(), source.clone());
            std::thread::Builder::new()
                .name(format!("texture loader {n}"))
                .spawn(move || {
                    loop {
                        let next = jobs.lock().map(|j| j.recv());
                        let Ok(Ok(job)) = next else { break };
                        let result = source.load(&job.key, job.request);
                        let done = JobResult {
                            id: job.id,
                            generation: job.generation,
                            request: job.request,
                            result,
                        };
                        if results.send(done).is_err() {
                            break;
                        }
                    }
                })
                .expect("spawning a texture loader thread");
        }
        TextureResidency {
            config,
            slots: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
            by_key: HashMap::new(),
            jobs: Some(job_tx),
            results: result_rx,
            ready: VecDeque::new(),
            frame: 0,
            evictions: 0,
            uploaded_bytes: 0,
        }
    }

    pub fn config(&self) -> &ResidencyConfig {
        &self.config
    }

    /// Change the budget and limits; takes effect at the next update.
    pub fn set_config(&mut self, config: ResidencyConfig) {
        self.config = config;
    }

    /// A handle to the texture at `key` (loading starts at the next update). The colour space
    /// of the first acquisition sticks.
    pub fn acquire(&mut self, key: &str, color_space: ColorSpace) -> TextureHandle {
        if let Some(&id) = self.by_key.get(key) {
            let slot = self.slots[id as usize]
                .as_ref()
                .expect("indexed slot exists");
            slot.shared.used.store(true, Ordering::Relaxed);
            return TextureHandle(slot.shared.clone());
        }
        let id = self.free.pop().unwrap_or_else(|| {
            self.slots.push(None);
            self.generations.push(0);
            self.slots.len() as u32 - 1
        });
        let shared = Arc::new(Shared {
            id,
            generation: self.generations[id as usize],
            wanted: AtomicU32::new(u32::MAX),
            used: AtomicBool::new(true),
        });
        let key: Arc<str> = key.into();
        self.by_key.insert(key.clone(), id);
        self.slots[id as usize] = Some(Slot {
            shared: shared.clone(),
            key,
            color_space,
            info: None,
            gpu: None,
            pending: false,
            failed: false,
            last_used: self.frame,
        });
        TextureHandle(shared)
    }

    fn slot(&self, id: u32, generation: u32) -> Option<&Slot> {
        self.slots
            .get(id as usize)?
            .as_ref()
            .filter(|s| s.shared.generation == generation)
    }

    /// The texture's view, or `None` while nothing is resident.
    pub fn view(&self, handle: &TextureHandle) -> Option<&wgpu::TextureView> {
        self.slot(handle.0.id, handle.0.generation)?
            .gpu
            .as_ref()
            .map(|g| &g.view)
    }

    /// Finest resident mip level, `None` while nothing is resident.
    pub fn resident_mip(&self, handle: &TextureHandle) -> Option<u32> {
        self.slot(handle.0.id, handle.0.generation)?
            .gpu
            .as_ref()
            .map(|g| g.top)
    }

    /// The texture's shape, once known.
    pub fn info(&self, handle: &TextureHandle) -> Option<TextureInfo> {
        self.slot(handle.0.id, handle.0.generation)?.info
    }

    /// Whether loading this texture failed.
    pub fn failed(&self, handle: &TextureHandle) -> bool {
        self.slot(handle.0.id, handle.0.generation)
            .is_some_and(|s| s.failed)
    }

    /// Material bind group of a streamed [`TextureId`] (renderer internal).
    pub(crate) fn material(&self, id: u32, generation: u32) -> Option<&wgpu::BindGroup> {
        self.slot(id, generation)?.gpu.as_ref()?.material.as_ref()
    }

    /// Ask for enough detail to cover `screen_size` pixels (renderer internal).
    pub(crate) fn want_screen_size(&self, id: u32, generation: u32, screen_size: f32) {
        let Some(slot) = self.slot(id, generation) else {
            return;
        };
        slot.shared.used.store(true, Ordering::Relaxed);
        if let Some(info) = slot.info {
            let level = required_mip(info.width.max(info.height), screen_size);
            slot.shared.wanted.fetch_min(level, Ordering::Relaxed);
        }
    }

    pub fn stats(&self) -> ResidencyStats {
        let mut stats = ResidencyStats {
            evictions: self.evictions,
            uploaded_bytes: self.uploaded_bytes,
            ..ResidencyStats::default()
        };
        for slot in self.slots.iter().flatten() {
            stats.textures += 1;
            stats.referenced += usize::from(Arc::strong_count(&slot.shared) > 1);
            stats.pending += usize::from(slot.pending);
            stats.failed += usize::from(slot.failed);
            if let (Some(info), Some(gpu)) = (slot.info, &slot.gpu) {
                stats.resident_bytes += info.bytes_from(gpu.top);
            }
        }
        stats
    }

    /// Apply arrived loads (within the upload budget), then evict, coarsen and request loads
    /// for this frame. `material` (layout and sampler) makes bind groups for
    /// [`MeshDraw`](crate::MeshDraw) use.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        material: Option<(&wgpu::BindGroupLayout, &wgpu::Sampler)>,
    ) {
        self.frame += 1;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("texture residency"),
        });
        while let Ok(result) = self.results.try_recv() {
            self.ready.push_back(result);
        }
        self.uploaded_bytes = 0;
        while let Some(result) = self.ready.pop_front() {
            let size = result.result.as_ref().map_or(0, LoadedMips::byte_len);
            if self.uploaded_bytes > 0
                && self.uploaded_bytes + size > self.config.upload_bytes_per_frame
            {
                self.ready.push_front(result);
                break;
            }
            self.uploaded_bytes += size;
            self.apply(device, queue, &mut encoder, result);
        }

        let views: Vec<plan::SlotView> = self
            .slots
            .iter_mut()
            .flatten()
            .map(|slot| {
                if slot.shared.used.swap(false, Ordering::Relaxed) {
                    slot.last_used = self.frame;
                }
                plan::SlotView {
                    id: slot.shared.id,
                    refs: Arc::strong_count(&slot.shared) - 1,
                    info: slot.info,
                    resident_top: slot.gpu.as_ref().map(|g| g.top),
                    pending: slot.pending,
                    failed: slot.failed,
                    last_used: slot.last_used,
                    wanted: slot.shared.wanted.swap(u32::MAX, Ordering::Relaxed),
                }
            })
            .collect();
        let decisions = plan::plan(
            &views,
            &plan::Policy {
                budget_bytes: self.config.budget_bytes,
                tail_size: self.config.tail_size,
                max_pending: self.config.max_pending,
            },
        );
        for id in decisions.evict {
            self.evict(id);
        }
        for (id, top) in decisions.coarsen {
            if let Some(slot) = self.slots[id as usize].as_mut() {
                let info = slot.info.expect("coarsened slots have info");
                if let Some(old) = &slot.gpu {
                    slot.gpu = Some(rebuild(
                        device,
                        &mut encoder,
                        slot.color_space,
                        &info,
                        old,
                        top,
                        None,
                    ));
                }
            }
        }
        for (id, request) in decisions.load {
            let Some(slot) = self.slots[id as usize].as_mut() else {
                continue;
            };
            slot.pending = true;
            let job = Job {
                id,
                generation: slot.shared.generation,
                key: slot.key.clone(),
                request,
            };
            if let Some(jobs) = &self.jobs {
                let _ = jobs.send(job);
            }
        }
        if let Some((layout, sampler)) = material {
            for slot in self.slots.iter_mut().flatten() {
                if let Some(gpu) = slot.gpu.as_mut().filter(|g| g.material.is_none()) {
                    gpu.material = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("streamed material"),
                        layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&gpu.view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(sampler),
                            },
                        ],
                    }));
                }
            }
        }
        queue.submit([encoder.finish()]);
    }

    fn evict(&mut self, id: u32) {
        let Some(slot) = self.slots[id as usize].as_ref() else {
            return;
        };
        if Arc::strong_count(&slot.shared) > 1 || slot.pending {
            return;
        }
        let key = slot.key.clone();
        self.slots[id as usize] = None;
        self.by_key.remove(&key);
        self.generations[id as usize] = self.generations[id as usize].wrapping_add(1);
        self.free.push(id);
        self.evictions += 1;
    }

    fn apply(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        done: JobResult,
    ) {
        let Some(slot) = self
            .slots
            .get_mut(done.id as usize)
            .and_then(Option::as_mut)
            .filter(|s| s.shared.generation == done.generation)
        else {
            return;
        };
        slot.pending = false;
        let loaded = match done.result {
            Ok(loaded) => loaded,
            Err(e) => {
                log::warn!("texture {} failed to load: {e}", slot.key);
                slot.failed = true;
                return;
            }
        };
        if !mips_match(&loaded) {
            log::warn!(
                "texture {} came back with an inconsistent mip chain",
                slot.key
            );
            slot.failed = true;
            return;
        }
        let info = loaded.info;
        match (done.request, &slot.gpu) {
            (MipRequest::Tail { .. }, None) => {
                let gpu = create(device, slot.color_space, &info, loaded.first_mip);
                write_mips(queue, &gpu, &info, &loaded);
                slot.info = Some(info);
                slot.gpu = Some(gpu);
            }
            (MipRequest::Range { first, end }, Some(old))
                if old.top == end && loaded.first_mip == first =>
            {
                let gpu = rebuild(
                    device,
                    encoder,
                    slot.color_space,
                    &info,
                    old,
                    first,
                    Some((queue, &loaded)),
                );
                slot.gpu = Some(gpu);
            }
            // The slot changed while loading (coarsened, or a duplicate tail); the planner
            // asks again if still needed.
            _ => {}
        }
    }
}

impl Drop for TextureResidency {
    fn drop(&mut self) {
        // Closing the job channel stops the loader threads.
        self.jobs = None;
    }
}

/// Whether `loaded` holds a consistent chain: right sizes, within the declared count.
fn mips_match(loaded: &LoadedMips) -> bool {
    let info = &loaded.info;
    if loaded.mips.is_empty() || loaded.first_mip + loaded.mips.len() as u32 > info.mip_count {
        return false;
    }
    let (w, h) = info.mip_size(loaded.first_mip);
    let dim = info.format.block_dim();
    if w % dim != 0 || h % dim != 0 {
        return false;
    }
    loaded.mips.iter().enumerate().all(|(i, data)| {
        let layout = MipLayout::new(
            info.format,
            info.width,
            info.height,
            loaded.first_mip + i as u32,
        );
        data.len() == layout.byte_len()
    })
}

/// An empty texture holding levels `top..mip_count`.
fn create(
    device: &wgpu::Device,
    color_space: ColorSpace,
    info: &TextureInfo,
    top: u32,
) -> GpuTexture {
    let (width, height) = info.mip_size(top);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("streamed texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: info.mip_count - top,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: info.format.wgpu_format(color_space),
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    GpuTexture {
        texture,
        view,
        top,
        material: None,
    }
}

/// Upload the loaded mips into `gpu` (whose top is at or above `loaded.first_mip`).
fn write_mips(queue: &wgpu::Queue, gpu: &GpuTexture, info: &TextureInfo, loaded: &LoadedMips) {
    let dim = info.format.block_dim();
    for (i, data) in loaded.mips.iter().enumerate() {
        let level = loaded.first_mip + i as u32;
        let layout = MipLayout::new(info.format, info.width, info.height, level);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &gpu.texture,
                mip_level: level - gpu.top,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(layout.bytes_per_row),
                rows_per_image: Some(layout.rows),
            },
            physical_extent(&layout, dim),
        );
    }
}

fn physical_extent(layout: &MipLayout, dim: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: layout.width.div_ceil(dim) * dim,
        height: layout.height.div_ceil(dim) * dim,
        depth_or_array_layers: 1,
    }
}

/// A texture with levels `top..`, copying the levels it shares with `old` on the GPU and
/// uploading the finer ones from `loaded` (refinement) or none (coarsening).
fn rebuild(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    color_space: ColorSpace,
    info: &TextureInfo,
    old: &GpuTexture,
    top: u32,
    loaded: Option<(&wgpu::Queue, &LoadedMips)>,
) -> GpuTexture {
    let gpu = create(device, color_space, info, top);
    if let Some((queue, loaded)) = loaded {
        write_mips(queue, &gpu, info, loaded);
    }
    let dim = info.format.block_dim();
    for level in top.max(old.top)..info.mip_count {
        let layout = MipLayout::new(info.format, info.width, info.height, level);
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &old.texture,
                mip_level: level - old.top,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &gpu.texture,
                mip_level: level - top,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            physical_extent(&layout, dim),
        );
    }
    gpu
}
