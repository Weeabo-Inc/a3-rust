//! The model [`RenderFeature`]: GPU resources, the asset cache and per-frame culling, LOD
//! selection and instanced drawing.

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use a3_paa::AlphaFlags;
use a3_render::{ColorSpace, GpuTexture, Phase, PrepareContext, RenderFeature, TextureData};
use a3_vfs::Vfs;
use bytemuck::{Pod, Zeroable};
use glam::{Affine3A, DAffine3, DMat3, DVec3, Vec3};
use wgpu::util::DeviceExt;

use crate::batch::InstanceBatcher;
use crate::cull::{Frustum, ObjectGrid};
use crate::loader::{Job, Loaded, Loader, TextureKey, TextureUse};
use crate::lod::{LodMetrics, LodSelector, ObjectBounds, ViewScale, Visibility, object_size};
use crate::material::{AlphaMode, MaterialDesc, Slot};
use crate::prepare::{ModelVertex, PreparedModel};
use crate::shader::ShaderFamily;
use crate::texture::TextureOptions;

/// Handle of a model registered with [`ModelRenderer::model`].
pub type ModelId = u32;

/// Tuning of the model renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelSettings {
    /// Objects farther than this (metres, horizontally) are not drawn.
    pub view_distance: f32,
    pub lod: LodSelector,
    /// Bytes of mesh and texture data uploaded per frame at most (at least one item).
    pub upload_budget: usize,
    /// Model loads submitted per frame at most, nearest first.
    pub loads_per_frame: usize,
    /// How deep proxies of proxies are followed.
    pub max_proxy_depth: u32,
    /// Grid cell edge for placed objects, metres.
    pub grid_cell: f64,
    /// Objects within this distance cast sun shadows (match the renderer's shadow distance).
    pub shadow_distance: f32,
    /// Draw this Resolution LOD index (clamped to the model's) instead of choosing by size;
    /// for inspecting models.
    pub force_lod: Option<usize>,
}

impl Default for ModelSettings {
    fn default() -> Self {
        ModelSettings {
            view_distance: 1600.0,
            lod: LodSelector::default(),
            upload_budget: 48 << 20,
            loads_per_frame: 256,
            max_proxy_depth: 2,
            grid_cell: 100.0,
            shadow_distance: 150.0,
            force_lod: None,
        }
    }
}

/// What the last frame did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelStats {
    /// Placed objects looked at by the grid query.
    pub candidates: u32,
    /// Instances drawn (objects and proxies).
    pub instances: u32,
    /// Draw calls issued.
    pub draw_calls: u32,
    /// Models uploaded and drawable.
    pub models_ready: u32,
    /// Models requested and not yet drawable.
    pub models_pending: u32,
    /// Models that failed to load.
    pub models_failed: u32,
    /// Textures on the GPU.
    pub textures: u32,
    /// Textures that failed to load (drawn with defaults).
    pub textures_failed: u32,
    /// Distinct materials (bind groups).
    pub materials: u32,
}

/// One placed object: a model and its world transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedObject {
    pub model: ModelId,
    pub transform: DAffine3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct InstanceRaw {
    rows: [[f32; 4]; 3],
    /// x: fade (1 = fully drawn, less = dithered in), yzw: unused.
    params: [f32; 4],
}

impl InstanceRaw {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4
    ];

    fn new(transform: &DAffine3, camera: DVec3, fade: f32) -> InstanceRaw {
        let m = transform.matrix3;
        let t = (transform.translation - camera).as_vec3();
        let row = |i: usize| {
            [
                m.x_axis[i] as f32,
                m.y_axis[i] as f32,
                m.z_axis[i] as f32,
                t[i],
            ]
        };
        InstanceRaw {
            rows: [row(0), row(1), row(2)],
            params: [fade, 0.0, 0.0, 0.0],
        }
    }
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x2, 4 => Float32x4
];

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    ambient: [f32; 4],
    diffuse: [f32; 4],
    emissive: [f32; 4],
    specular: [f32; 4],
    params: [u32; 4],
    uv: [[f32; 4]; 2 * Slot::COUNT],
}

struct GpuSection {
    indices: Range<u32>,
    material: u32,
    alpha: AlphaMode,
}

struct GpuLod {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    sections: Vec<GpuSection>,
    proxies: Vec<(ModelId, Affine3A)>,
}

struct GpuModel {
    lods: Vec<GpuLod>,
    metrics: Vec<LodMetrics>,
    density: f32,
    radius: f32,
    /// Bounding box of the drawn geometry, model space.
    bbox: (Vec3, Vec3),
}

enum ModelState {
    Unrequested,
    Loading,
    Prepared(Box<PreparedModel>),
    Ready(GpuModel),
    Failed,
}

struct ModelEntry {
    path: String,
    state: ModelState,
}

enum TextureState {
    Pending,
    Ready(GpuTexture, Option<AlphaFlags>),
    Failed,
}

struct GpuMaterial {
    bind_group: wgpu::BindGroup,
}

#[derive(Debug, Clone)]
struct DrawCmd {
    model: ModelId,
    lod: u16,
    section: u16,
    instances: Range<u32>,
}

/// Renders ODOL models: owns the GPU resources, the model and texture cache and the placed
/// objects. Wrap it in a [`ModelFeature`] to register it with the renderer.
pub struct ModelRenderer {
    pub settings: ModelSettings,
    material_layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::RenderPipeline; 3],
    shadow_pipelines: [wgpu::RenderPipeline; 2],
    sampler: wgpu::Sampler,
    /// 1x1 stand-ins for empty slots, by texel and sRGB-ness.
    defaults: HashMap<([u8; 4], bool), GpuTexture>,

    loader: Loader,
    models: Vec<ModelEntry>,
    model_ids: HashMap<String, ModelId>,
    textures: HashMap<TextureKey, TextureState>,
    materials: Vec<GpuMaterial>,
    material_ids: HashMap<MaterialDesc, u32>,
    waiting: Vec<ModelId>,
    results: VecDeque<Loaded>,

    objects: Vec<PlacedObject>,
    grid: ObjectGrid,
    dynamic: Vec<PlacedObject>,

    batcher: InstanceBatcher<(Pass, ModelId, u16), InstanceRaw>,
    load_requests: Vec<(f64, ModelId)>,
    instance_buffer: wgpu::Buffer,
    draws: [Vec<DrawCmd>; 3],
    shadow_draws: [Vec<DrawCmd>; 2],
    stats: ModelStats,
}

impl ModelRenderer {
    /// Create the renderer's pipelines (group 0 is the renderer's `frame_layout`) and start
    /// `loader_threads` workers reading from `vfs`.
    pub fn new(
        device: &wgpu::Device,
        frame_layout: &wgpu::BindGroupLayout,
        vfs: Vfs,
        texture_options: TextureOptions,
        loader_threads: usize,
    ) -> ModelRenderer {
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }];
        entries.extend((1..=Slot::COUNT as u32).map(texture_entry));
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: Slot::COUNT as u32 + 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model material layout"),
            entries: &entries,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("model"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/model.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("model"),
            bind_group_layouts: &[Some(frame_layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let pipeline = |alpha: AlphaMode| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(match alpha {
                    AlphaMode::Opaque => "model opaque",
                    AlphaMode::Test => "model alpha test",
                    AlphaMode::Blend => "model blend",
                }),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<ModelVertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &VERTEX_ATTRIBUTES,
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<InstanceRaw>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &InstanceRaw::ATTRIBUTES,
                        }),
                    ],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode: (alpha == AlphaMode::Opaque).then_some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: a3_render::Renderer::DEPTH_FORMAT,
                    depth_write_enabled: Some(alpha != AlphaMode::Blend),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: a3_render::Renderer::SCENE_COLOR_FORMAT,
                        blend: (alpha == AlphaMode::Blend)
                            .then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipelines = [
            pipeline(AlphaMode::Opaque),
            pipeline(AlphaMode::Test),
            pipeline(AlphaMode::Blend),
        ];
        let shadow_pipeline = |alpha: AlphaMode| {
            let alpha_test = alpha == AlphaMode::Test;
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if alpha_test {
                    "model shadow alpha test"
                } else {
                    "model shadow"
                }),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_shadow"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<ModelVertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &VERTEX_ATTRIBUTES,
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<InstanceRaw>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &InstanceRaw::ATTRIBUTES,
                        }),
                    ],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode: (!alpha_test).then_some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: a3_render::shadow::SHADOW_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant: 2,
                        slope_scale: 2.0,
                        clamp: 0.0,
                    },
                }),
                multisample: Default::default(),
                fragment: alpha_test.then(|| wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_shadow_alpha_test"),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let shadow_pipelines = [
            shadow_pipeline(AlphaMode::Opaque),
            shadow_pipeline(AlphaMode::Test),
        ];
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("model material sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        ModelRenderer {
            settings: ModelSettings::default(),
            material_layout,
            pipelines,
            shadow_pipelines,
            sampler,
            defaults: HashMap::new(),
            loader: Loader::new(vfs, loader_threads, texture_options),
            models: Vec::new(),
            model_ids: HashMap::new(),
            textures: HashMap::new(),
            materials: Vec::new(),
            material_ids: HashMap::new(),
            waiting: Vec::new(),
            results: VecDeque::new(),
            objects: Vec::new(),
            grid: ObjectGrid::build(100.0, std::iter::empty()),
            dynamic: Vec::new(),
            batcher: InstanceBatcher::default(),
            load_requests: Vec::new(),
            instance_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("model instances"),
                size: 1 << 16,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            draws: Default::default(),
            shadow_draws: Default::default(),
            stats: ModelStats::default(),
        }
    }

    /// The id of the model at VFS `path` (`.p3d` added when missing), registering it. Nothing
    /// loads until the model is drawn.
    pub fn model(&mut self, path: &str) -> ModelId {
        let mut key = path
            .trim_start_matches(['\\', '/'])
            .replace('/', "\\")
            .to_ascii_lowercase();
        if !key.ends_with(".p3d") {
            key.push_str(".p3d");
        }
        if let Some(&id) = self.model_ids.get(&key) {
            return id;
        }
        let id = self.models.len() as ModelId;
        self.models.push(ModelEntry {
            path: key.clone(),
            state: ModelState::Unrequested,
        });
        self.model_ids.insert(key, id);
        id
    }

    /// Register model data built on the CPU (procedural or test content) under `path`,
    /// replacing whatever was loaded there. It uploads like a loaded file, textures included.
    pub fn insert_prepared(&mut self, path: &str, model: PreparedModel) -> ModelId {
        let id = self.model(path);
        self.models[id as usize].state = ModelState::Loading;
        self.results.push_back(Loaded::Model {
            id,
            result: Ok(Box::new(model)),
        });
        id
    }

    /// Replace the static placed objects (for example a terrain's objects).
    pub fn set_static_objects(&mut self, objects: Vec<PlacedObject>) {
        // Model radii are unknown until loaded; cells are padded by a typical building size.
        const GUESS_RADIUS: f32 = 30.0;
        self.grid = ObjectGrid::build(
            self.settings.grid_cell,
            objects
                .iter()
                .enumerate()
                .map(|(i, o)| (i as u32, o.transform.translation, GUESS_RADIUS)),
        );
        self.objects = objects;
    }

    /// Number of static placed objects.
    pub fn static_object_count(&self) -> usize {
        self.objects.len()
    }

    /// Objects drawn this frame only, in addition to the static ones; cleared with
    /// [`clear_dynamic`](Self::clear_dynamic).
    pub fn add_dynamic(&mut self, object: PlacedObject) {
        self.dynamic.push(object);
    }

    pub fn clear_dynamic(&mut self) {
        self.dynamic.clear();
    }

    /// Request a model now, without waiting for it to become visible.
    pub fn preload(&mut self, model: ModelId) {
        self.request(model);
    }

    /// Bounding radius around the model origin and the number of Resolution LODs, once the
    /// model is drawable.
    pub fn model_bounds(&self, model: ModelId) -> Option<(f32, usize)> {
        match &self.models.get(model as usize)?.state {
            ModelState::Ready(m) => Some((m.radius, m.lods.len())),
            _ => None,
        }
    }

    /// Whether the model failed to load.
    pub fn model_failed(&self, model: ModelId) -> bool {
        matches!(
            self.models.get(model as usize).map(|m| &m.state),
            Some(ModelState::Failed)
        )
    }

    /// No loads are queued, running or waiting for upload.
    pub fn is_idle(&self) -> bool {
        self.loader.in_flight() == 0
            && self.waiting.is_empty()
            && self.results.is_empty()
            && self.load_requests.is_empty()
    }

    pub fn stats(&self) -> ModelStats {
        self.stats
    }

    fn request(&mut self, model: ModelId) {
        let entry = &mut self.models[model as usize];
        if matches!(entry.state, ModelState::Unrequested) {
            entry.state = ModelState::Loading;
            self.loader.submit(Job::Model {
                id: model,
                path: entry.path.clone(),
            });
        }
    }

    /// Per-frame work: integrate finished loads, cull, select LODs, batch and upload instances.
    pub fn prepare(&mut self, cx: &PrepareContext<'_>) {
        self.integrate_loads(cx.device, cx.queue);
        self.cull_and_batch(cx);
        self.stats.models_ready = 0;
        self.stats.models_pending = 0;
        self.stats.models_failed = 0;
        for m in &self.models {
            match m.state {
                ModelState::Ready(_) => self.stats.models_ready += 1,
                ModelState::Loading | ModelState::Prepared(_) => self.stats.models_pending += 1,
                ModelState::Failed => self.stats.models_failed += 1,
                ModelState::Unrequested => {}
            }
        }
        self.stats.materials = self.materials.len() as u32;
    }

    fn integrate_loads(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        while let Some(loaded) = self.loader.try_recv() {
            self.results.push_back(loaded);
        }
        let mut uploaded = 0usize;
        while uploaded < self.settings.upload_budget {
            let Some(loaded) = self.results.pop_front() else {
                break;
            };
            match loaded {
                Loaded::Model { id, result } => match result {
                    Ok(prepared) => {
                        for key in texture_keys(&prepared) {
                            if !self.textures.contains_key(&key) {
                                self.textures.insert(key.clone(), TextureState::Pending);
                                self.loader.submit(Job::Texture(key));
                            }
                        }
                        self.models[id as usize].state = ModelState::Prepared(prepared);
                        self.waiting.push(id);
                    }
                    Err(e) => {
                        log::warn!("model {}: {e}", self.models[id as usize].path);
                        self.models[id as usize].state = ModelState::Failed;
                    }
                },
                Loaded::Texture { key, result } => {
                    let state = match result.map(|t| {
                        let space = match key.usage {
                            TextureUse::Color => ColorSpace::Srgb,
                            _ => ColorSpace::Linear,
                        };
                        uploaded += t.data.mips.iter().map(Vec::len).sum::<usize>();
                        GpuTexture::upload(device, queue, &t.data, space, None)
                            .map(|g| (g, t.alpha))
                            .map_err(|e| e.to_string())
                    }) {
                        Ok(Ok((gpu, alpha))) => {
                            self.stats.textures += 1;
                            TextureState::Ready(gpu, alpha)
                        }
                        Ok(Err(e)) | Err(e) => {
                            log::debug!("texture {}: {e}", key.path);
                            self.stats.textures_failed += 1;
                            TextureState::Failed
                        }
                    };
                    self.textures.insert(key, state);
                }
            }
        }

        // Models whose textures have all arrived become drawable.
        let mut i = 0;
        while i < self.waiting.len() && uploaded < self.settings.upload_budget {
            let id = self.waiting[i];
            let ModelState::Prepared(prepared) = &self.models[id as usize].state else {
                self.waiting.swap_remove(i);
                continue;
            };
            let ready = texture_keys(prepared)
                .iter()
                .all(|k| !matches!(self.textures.get(k), Some(TextureState::Pending)));
            if !ready {
                i += 1;
                continue;
            }
            let ModelState::Prepared(prepared) =
                std::mem::replace(&mut self.models[id as usize].state, ModelState::Loading)
            else {
                unreachable!("checked above");
            };
            let (model, bytes) = self.upload_model(device, queue, &prepared);
            uploaded += bytes;
            self.models[id as usize].state = ModelState::Ready(model);
            self.waiting.swap_remove(i);
        }
    }

    fn upload_model(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        prepared: &PreparedModel,
    ) -> (GpuModel, usize) {
        let mut bytes = 0;
        let mut lods = Vec::with_capacity(prepared.lods.len());
        for lod in &prepared.lods {
            let sections = lod
                .sections
                .iter()
                .map(|s| {
                    let (material, alpha) = self.material(device, queue, &s.material);
                    GpuSection {
                        indices: s.indices.clone(),
                        material,
                        alpha,
                    }
                })
                .collect();
            let proxies = lod
                .proxies
                .iter()
                .filter(|p| renders_proxy(&p.model))
                .map(|p| (self.model(&p.model), p.transform))
                .collect();
            let vertices = if lod.vertices.is_empty() {
                &[ModelVertex::zeroed()][..]
            } else {
                &lod.vertices[..]
            };
            let indices = if lod.indices.is_empty() {
                &[0u32, 0, 0][..]
            } else {
                &lod.indices[..]
            };
            bytes += std::mem::size_of_val(vertices) + std::mem::size_of_val(indices);
            lods.push(GpuLod {
                vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("model vertices"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("model indices"),
                    contents: bytemuck::cast_slice(indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                sections,
                proxies,
            });
        }
        let model = GpuModel {
            lods,
            metrics: prepared.lod_metrics(),
            density: prepared.lod_density_coef,
            radius: prepared.radius,
            bbox: prepared.bbox,
        };
        (model, bytes)
    }

    /// The bind group index and alpha mode of a material, creating it on first use.
    fn material(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        desc: &MaterialDesc,
    ) -> (u32, AlphaMode) {
        let diffuse_alpha = desc.texture(Slot::Diffuse).and_then(|t| {
            match self
                .textures
                .get(&TextureKey::new(&t.path, TextureUse::Color))
            {
                Some(TextureState::Ready(_, alpha)) => *alpha,
                _ => None,
            }
        });
        let mut desc = desc.clone();
        desc.alpha = AlphaMode::decide(desc.family, diffuse_alpha);
        if let Some(&id) = self.material_ids.get(&desc) {
            return (id, desc.alpha);
        }
        let mut uniform = MaterialUniform {
            ambient: desc.ambient,
            diffuse: desc.diffuse,
            emissive: desc.emissive,
            specular: [
                desc.specular[0],
                desc.specular[1],
                desc.specular[2],
                desc.specular_power,
            ],
            params: [family_code(desc.family), desc.alpha as u32, 0, 0],
            uv: [[1.0, 0.0, 0.0, 0.0]; 2 * Slot::COUNT],
        };
        for i in 0..Slot::COUNT {
            uniform.uv[2 * i + 1] = [0.0, 1.0, 0.0, 0.0];
        }
        let mut views: Vec<wgpu::TextureView> = (0..Slot::COUNT)
            .map(|i| {
                let slot = Slot::at(i, desc.family);
                let key = (slot.default_texel(desc.family), slot.is_color());
                let texture = self.defaults.entry(key).or_insert_with(|| {
                    let space = if key.1 {
                        ColorSpace::Srgb
                    } else {
                        ColorSpace::Linear
                    };
                    GpuTexture::upload(
                        device,
                        queue,
                        &TextureData::solid_rgba8(key.0),
                        space,
                        Some("model default texture"),
                    )
                    .expect("a 1x1 RGBA8 texture is always valid")
                });
                texture.view.clone()
            })
            .collect();
        for (slot, texture) in desc.textures() {
            let i = slot.binding();
            let [r0, r1] = texture.uv.rows;
            uniform.uv[2 * i] = [r0[0], r0[1], r0[2], texture.uv.uv_set as f32];
            uniform.uv[2 * i + 1] = [r1[0], r1[1], r1[2], 0.0];
            let key = TextureKey::new(&texture.path, texture_use(slot));
            if let Some(TextureState::Ready(gpu, _)) = self.textures.get(&key) {
                views[i] = gpu.view.clone();
            }
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("model material"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }];
        for (i, view) in views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: i as u32 + 1,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: Slot::COUNT as u32 + 1,
            resource: wgpu::BindingResource::Sampler(&self.sampler),
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("model material"),
            layout: &self.material_layout,
            entries: &entries,
        });
        let id = self.materials.len() as u32;
        self.materials.push(GpuMaterial { bind_group });
        self.material_ids.insert(desc.clone(), id);
        (id, desc.alpha)
    }

    fn cull_and_batch(&mut self, cx: &PrepareContext<'_>) {
        let camera = cx.camera.position;
        let frustum = Frustum::from_view_projection(cx.view_projection);
        let aspect = cx.viewport.0.max(1) as f32 / cx.viewport.1.max(1) as f32;
        let view = CullView {
            camera,
            scale: ViewScale::new(cx.camera.fov.top, aspect),
        };
        let view_distance = f64::from(self.settings.view_distance);
        let shadow_distance = f64::from(
            self.settings
                .shadow_distance
                .min(self.settings.view_distance),
        );

        // Visible objects.
        let mut candidates = Vec::new();
        self.grid.query(camera, view_distance, Some(&frustum), |i| {
            candidates.push(i)
        });
        self.stats.candidates = candidates.len() as u32;
        let mut roots: Vec<PlacedObject> = candidates
            .iter()
            .map(|&i| self.objects[i as usize])
            .collect();
        roots.extend(self.dynamic.iter().copied());
        self.gather(Pass::View, roots, &view, Some(&frustum), view_distance);

        // Shadow casters: near objects in every direction (a caster behind the camera can
        // shadow what is in view).
        if shadow_distance > 0.0 {
            candidates.clear();
            self.grid
                .query(camera, shadow_distance, None, |i| candidates.push(i));
            let mut roots: Vec<PlacedObject> = candidates
                .iter()
                .map(|&i| self.objects[i as usize])
                .collect();
            roots.extend(self.dynamic.iter().copied());
            self.gather(Pass::Shadow, roots, &view, None, shadow_distance);
        }

        // Nearest missing models first.
        self.load_requests.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.load_requests.dedup_by_key(|r| r.1);
        let n = self.load_requests.len().min(self.settings.loads_per_frame);
        let requests: Vec<ModelId> = self.load_requests.drain(..n).map(|r| r.1).collect();
        self.load_requests.clear();
        for id in requests {
            self.request(id);
        }

        let (instances, batches) = self.batcher.finish();
        self.stats.instances = 0;
        let bytes: &[u8] = bytemuck::cast_slice(&instances);
        if bytes.len() as u64 > self.instance_buffer.size() {
            self.instance_buffer = cx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("model instances"),
                size: (bytes.len() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !bytes.is_empty() {
            cx.queue.write_buffer(&self.instance_buffer, 0, bytes);
        }
        for list in self.draws.iter_mut().chain(&mut self.shadow_draws) {
            list.clear();
        }
        for batch in batches {
            let (pass, model, lod) = batch.key;
            let ModelState::Ready(m) = &self.models[model as usize].state else {
                continue;
            };
            if pass == Pass::View {
                self.stats.instances += batch.instances.len() as u32;
            }
            for (s, section) in m.lods[lod as usize].sections.iter().enumerate() {
                let cmd = DrawCmd {
                    model,
                    lod,
                    section: s as u16,
                    instances: batch.instances.clone(),
                };
                match (pass, section.alpha) {
                    (Pass::View, alpha) => self.draws[alpha as usize].push(cmd),
                    (Pass::Shadow, AlphaMode::Blend) => {}
                    (Pass::Shadow, alpha) => self.shadow_draws[alpha as usize].push(cmd),
                }
            }
        }
        // Fewer state changes: group by material within each pipeline.
        let models = &self.models;
        let material_of = |d: &DrawCmd| match &models[d.model as usize].state {
            ModelState::Ready(m) => m.lods[d.lod as usize].sections[d.section as usize].material,
            _ => 0,
        };
        for list in self.draws[..2]
            .iter_mut()
            .chain(&mut self.shadow_draws[1..])
        {
            list.sort_by_key(material_of);
        }
        self.stats.draw_calls = self.draws.iter().map(|l| l.len() as u32).sum();
    }

    /// Select LODs for `roots` and their proxies and queue them as instances of `pass`.
    fn gather(
        &mut self,
        pass: Pass,
        roots: Vec<PlacedObject>,
        view: &CullView,
        frustum: Option<&Frustum>,
        max_distance: f64,
    ) {
        let mut stack: Vec<(PlacedObject, u32)> = roots.into_iter().map(|o| (o, 0)).collect();
        while let Some((object, depth)) = stack.pop() {
            let relative = object.transform.translation - view.camera;
            let distance = relative.length();
            let horizontal = (relative.x * relative.x + relative.z * relative.z).sqrt();
            let model = match &self.models[object.model as usize].state {
                ModelState::Ready(m) => m,
                ModelState::Unrequested => {
                    if pass == Pass::View && horizontal <= max_distance {
                        self.load_requests.push((distance, object.model));
                    }
                    continue;
                }
                _ => continue,
            };
            let scale = max_scale(&object.transform.matrix3);
            let radius = model.radius * scale;
            if frustum.is_some_and(|f| !f.intersects_sphere(relative.as_vec3(), radius)) {
                continue;
            }
            if depth == 0 && horizontal - f64::from(radius) > max_distance {
                continue;
            }
            let bounds = ObjectBounds {
                size: object_size(model.bbox.0, model.bbox.1, scale),
                density: model.density,
            };
            let lod_settings = &self.settings.lod;
            if pass == Pass::Shadow
                && !lod_settings.casts_shadow(bounds, distance as f32, view.scale)
            {
                continue;
            }
            let selected = match self.settings.force_lod {
                Some(lod) if !model.lods.is_empty() => {
                    Some((lod.min(model.lods.len() - 1), Visibility::Full))
                }
                _ => lod_settings.select(&model.metrics, bounds, distance as f32, view.scale),
            };
            let Some((lod, visibility)) = selected else {
                continue;
            };
            let fade = match visibility {
                Visibility::Fade(f) if pass == Pass::View => f,
                _ => 1.0,
            };
            self.batcher.push(
                (pass, object.model, lod as u16),
                InstanceRaw::new(&object.transform, view.camera, fade),
            );
            if depth < self.settings.max_proxy_depth {
                for (proxy, transform) in &model.lods[lod].proxies {
                    let local = DAffine3 {
                        matrix3: transform.matrix3.as_dmat3(),
                        translation: transform.translation.as_dvec3(),
                    };
                    stack.push((
                        PlacedObject {
                            model: *proxy,
                            transform: object.transform * local,
                        },
                        depth + 1,
                    ));
                }
            }
        }
    }

    /// Record this frame's draws for `phase`.
    pub fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>) {
        let lists: [(&wgpu::RenderPipeline, &[DrawCmd]); 2] = match phase {
            Phase::Opaque => [
                (&self.pipelines[0], &self.draws[0]),
                (&self.pipelines[1], &self.draws[1]),
            ],
            Phase::Alpha => [
                (&self.pipelines[2], &self.draws[2]),
                (&self.pipelines[2], &[]),
            ],
            Phase::Shadow { .. } => [
                (&self.shadow_pipelines[0], &self.shadow_draws[0]),
                (&self.shadow_pipelines[1], &self.shadow_draws[1]),
            ],
            Phase::Ui => return,
        };
        for (pipeline, draws) in lists {
            if draws.is_empty() {
                continue;
            }
            pass.set_pipeline(pipeline);
            pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            let mut bound_material = u32::MAX;
            let mut bound_lod = (u32::MAX, u16::MAX);
            for d in draws {
                let ModelState::Ready(m) = &self.models[d.model as usize].state else {
                    continue;
                };
                let lod = &m.lods[d.lod as usize];
                let section = &lod.sections[d.section as usize];
                if section.material != bound_material {
                    bound_material = section.material;
                    pass.set_bind_group(
                        1,
                        &self.materials[section.material as usize].bind_group,
                        &[],
                    );
                }
                if bound_lod != (d.model, d.lod) {
                    bound_lod = (d.model, d.lod);
                    pass.set_vertex_buffer(0, lod.vertices.slice(..));
                    pass.set_index_buffer(lod.indices.slice(..), wgpu::IndexFormat::Uint32);
                }
                pass.draw_indexed(section.indices.clone(), 0, d.instances.clone());
            }
        }
    }
}

/// Which pass an instance is batched for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Pass {
    View,
    Shadow,
}

/// What LOD selection needs to know about the camera.
struct CullView {
    camera: DVec3,
    scale: ViewScale,
}

fn family_code(family: ShaderFamily) -> u32 {
    match family {
        ShaderFamily::Basic | ShaderFamily::Unsupported => 0,
        ShaderFamily::Super | ShaderFamily::SuperAlphaTest => 1,
        ShaderFamily::Multi => 2,
        ShaderFamily::Tree => 3,
        ShaderFamily::Glass => 4,
    }
}

fn texture_use(slot: Slot) -> TextureUse {
    match slot {
        Slot::Normal | Slot::LayerNormal(_) => TextureUse::Normal,
        s if s.is_color() => TextureUse::Color,
        _ => TextureUse::Data,
    }
}

fn texture_keys(prepared: &PreparedModel) -> Vec<TextureKey> {
    let mut keys: Vec<TextureKey> = prepared
        .lods
        .iter()
        .flat_map(|l| &l.sections)
        .flat_map(|s| s.material.textures())
        .map(|(slot, t)| TextureKey::new(&t.path, texture_use(slot)))
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

fn max_scale(m: &DMat3) -> f32 {
    m.x_axis
        .length()
        .max(m.y_axis.length())
        .max(m.z_axis.length()) as f32
}

/// Proxies that stand for things placed at run time (crew seats, weapon slots, muzzle flashes)
/// are placeholders, not decoration.
fn renders_proxy(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    !(p.contains("\\proxies\\")
        || p.starts_with("a3\\characters_f\\heads\\")
        || p.contains("volumelight"))
}

/// A cloneable handle registering a [`ModelRenderer`] as a render feature while the game
/// keeps access to it.
#[derive(Clone)]
pub struct ModelFeature(Arc<Mutex<ModelRenderer>>);

impl ModelFeature {
    pub fn new(renderer: ModelRenderer) -> Self {
        ModelFeature(Arc::new(Mutex::new(renderer)))
    }

    /// Exclusive access to the renderer, between frames.
    pub fn lock(&self) -> MutexGuard<'_, ModelRenderer> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl RenderFeature for ModelFeature {
    fn prepare(&mut self, cx: &PrepareContext<'_>) {
        self.lock().prepare(cx);
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>) {
        self.lock().draw(phase, pass);
    }
}
