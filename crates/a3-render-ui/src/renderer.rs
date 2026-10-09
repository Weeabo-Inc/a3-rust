//! The UI render feature: [a3-ui](a3_ui) draw lists drawn over the finished frame.
//!
//! [`UiRenderer`] turns the [`DrawList`] set with [`set_draw_list`](UiRenderer::set_draw_list)
//! into batched quads (see [`crate::build_geometry`]) and draws them into [`Phase::Ui`], after
//! the renderer's post pass, with no depth test. Wrap it in a [`UiFeature`] to register it:
//!
//! ```no_run
//! # let (gpu, renderer) = todo!();
//! # let mut renderer: a3_render::Renderer = renderer;
//! let ui = a3_render_ui::UiFeature::new(&gpu, &renderer);
//! ui.lock().set_assets(Box::new(a3_render_ui::vfs_with_dir(std::path::Path::new("."))));
//! renderer.add_feature(Box::new(ui.clone()));
//! // each frame, after building the draw list:
//! # let list = a3_ui::DrawList::default();
//! ui.lock().set_draw_list(list);
//! ```

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use a3_render::{Gpu, Phase, PrepareContext, RenderFeature, Renderer};
use a3_ui::DrawList;

use crate::assets::UiAssets;
use crate::cache::{TextureCache, TextureInfo};
use crate::geometry::{Geometry, UiVertex, build_geometry};

/// What the last [`prepare`](RenderFeature::prepare) did, for diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiStats {
    /// Quads in the draw list.
    pub quads: usize,
    /// Vertices written (six per drawn quad).
    pub vertices: u32,
    /// Batches, i.e. draw calls.
    pub batches: u32,
    /// Textures uploaded to the cache, the white texture included.
    pub textures: u32,
    /// Textures decoded and uploaded by the last prepare.
    pub uploads: u32,
    /// Quads dropped because their texture is not loaded.
    pub missing_textures: usize,
    /// Quads dropped because they are empty, transparent or clipped away.
    pub skipped: usize,
}

/// A vertex buffer that grows to the largest frame seen.
struct VertexBuffer {
    buffer: wgpu::Buffer,
    capacity: u64,
}

impl VertexBuffer {
    fn new(device: &wgpu::Device) -> Self {
        // Never zero-sized: draws are skipped while there is nothing to draw.
        let capacity = (size_of::<UiVertex>() * 6) as u64;
        VertexBuffer {
            buffer: Self::allocate(device, capacity),
            capacity,
        }
    }

    fn allocate(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui vertices"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if bytes.len() as u64 > self.capacity {
            self.capacity = (bytes.len() as u64).next_power_of_two();
            self.buffer = Self::allocate(device, self.capacity);
        }
        queue.write_buffer(&self.buffer, 0, bytes);
    }
}

/// Draws the a3-ui draw list in [`Phase::Ui`].
pub struct UiRenderer {
    pipeline: wgpu::RenderPipeline,
    cache: TextureCache,
    vertices: VertexBuffer,
    geometry: Geometry,
    /// The draw list of the frame, kept until the next `set_draw_list`.
    list: DrawList,
    /// The resolution of each texture of `list` by key.
    textures: Vec<Option<TextureInfo>>,
    assets: Option<Box<dyn UiAssets>>,
    stats: UiStats,
}

impl UiRenderer {
    /// Builds the pipeline against `renderer`'s frame layout and output format.
    pub fn new(gpu: &Gpu, renderer: &Renderer) -> Self {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/ui.wgsl").into()),
        });
        let cache = TextureCache::new(device, &gpu.queue, Renderer::supports_bc(gpu));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui pipeline layout"),
            bind_group_layouts: &[Some(renderer.frame_layout()), Some(cache.layout())],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<UiVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4,
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            // The UI pass has no depth attachment.
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: renderer.output_format(),
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        UiRenderer {
            pipeline,
            cache,
            vertices: VertexBuffer::new(device),
            geometry: Geometry::default(),
            list: DrawList::default(),
            textures: Vec::new(),
            assets: None,
            stats: UiStats::default(),
        }
    }

    /// Sets where textures are read from (the game VFS, or any other source).
    pub fn set_assets(&mut self, assets: Box<dyn UiAssets>) {
        self.assets = Some(assets);
    }

    /// Sets this frame's draw list.
    pub fn set_draw_list(&mut self, list: DrawList) {
        self.list = list;
    }

    /// Forgets the draw list; nothing is drawn until the next `set_draw_list`.
    pub fn clear(&mut self) {
        self.list = DrawList::default();
        self.geometry = Geometry::default();
        self.textures.clear();
        self.stats = UiStats::default();
    }

    /// Loads every texture `list` uses into the cache now, so the first frame that draws it
    /// does no decoding. Textures are cached across frames.
    pub fn preload(&mut self, gpu: &Gpu, list: &DrawList) {
        for path in list.textures() {
            self.cache
                .resolve(&gpu.device, &gpu.queue, self.assets.as_deref(), path);
        }
    }

    /// What the last frame rendered.
    pub fn stats(&self) -> UiStats {
        self.stats
    }

    /// The number of textures held in the cache.
    pub fn texture_count(&self) -> usize {
        self.cache.len()
    }
}

impl RenderFeature for UiRenderer {
    fn prepare(&mut self, cx: &PrepareContext<'_>) {
        let before = self.cache.len();
        self.textures.clear();
        for path in self.list.textures() {
            let info = self
                .cache
                .resolve(cx.device, cx.queue, self.assets.as_deref(), path);
            self.textures.push(info);
        }
        self.geometry = build_geometry(&self.list, &self.textures, cx.viewport);
        self.vertices.write(
            cx.device,
            cx.queue,
            bytemuck::cast_slice(&self.geometry.vertices),
        );
        self.stats = UiStats {
            quads: self.list.quads.len(),
            vertices: self.geometry.vertices.len() as u32,
            batches: self.geometry.batches.len() as u32,
            textures: self.cache.len() as u32,
            uploads: (self.cache.len() - before) as u32,
            missing_textures: self.geometry.missing_textures,
            skipped: self.geometry.skipped,
        };
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>) {
        if phase != Phase::Ui || self.geometry.batches.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertices.buffer.slice(..));
        for batch in &self.geometry.batches {
            let Some(group) = self.cache.group(batch.slot) else {
                continue;
            };
            pass.set_bind_group(1, group, &[]);
            let [x, y, w, h] = batch.scissor;
            pass.set_scissor_rect(x, y, w, h);
            pass.draw(batch.start..batch.start + batch.count, 0..1);
        }
    }
}

/// Hands a shared [`UiRenderer`] to the renderer.
///
/// The app keeps a clone to set the draw list each frame; the one registered with
/// [`Renderer::add_feature`] draws it. Like [`ModelFeature`](https://docs.rs/a3-render-models),
/// the lock is only ever held for the duration of one call.
#[derive(Clone)]
pub struct UiFeature(Arc<Mutex<UiRenderer>>);

impl UiFeature {
    /// Creates the renderer and its handle.
    pub fn new(gpu: &Gpu, renderer: &Renderer) -> Self {
        UiFeature(Arc::new(Mutex::new(UiRenderer::new(gpu, renderer))))
    }

    /// Exclusive access to the renderer, between frames.
    pub fn lock(&self) -> MutexGuard<'_, UiRenderer> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl RenderFeature for UiFeature {
    fn prepare(&mut self, cx: &PrepareContext<'_>) {
        self.lock().prepare(cx);
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>) {
        self.lock().draw(phase, pass);
    }
}
