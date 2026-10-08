//! The frame: uniforms, built-in pipelines (meshes, debug lines, text), the post chain and
//! phase order.

use std::sync::mpsc;

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Vec3};

use crate::camera::Camera;
use crate::draw::{DrawList, MeshId, TextureId, relative_model};
use crate::feature::{Phase, PrepareContext, RenderFeature};
use crate::font;
use crate::gpu::{Gpu, RenderError};
use crate::mesh::{Mesh, MeshData, Vertex};
use crate::post::{HdrSettings, PostChain};
use crate::shadow::{
    self, MAX_CASCADES, SHADOW_FORMAT, ShadowMaps, ShadowSettings, ShadowUniforms,
};
use crate::texture::{ColorSpace, GpuTexture, TextureData, TextureError};

/// Eye adaptation state read back from the GPU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExposureReadout {
    /// Luminance the eye is adapted to (RV's aperture analogue).
    pub adapted_luminance: f32,
    /// Multiplier applied to scene colour before tonemapping.
    pub exposure: f32,
    /// Average scene luminance measured this frame.
    pub average_luminance: f32,
}

/// Lighting and atmosphere parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderSettings {
    /// Unit vector towards the sun.
    pub sun_direction: Vec3,
    /// Linear sun colour and intensity.
    pub sun_color: Vec3,
    /// Ambient light level (scalar, for shaders without hemisphere ambient).
    pub ambient: f32,
    /// Hemisphere ambient: light from straight above (RV `AE`).
    pub ambient_sky: Vec3,
    /// Hemisphere ambient: light from the horizon (RV `AmbientMid`).
    pub ambient_mid: Vec3,
    /// Hemisphere ambient: light from below (RV `GE`, ground reflection).
    pub ambient_ground: Vec3,
    pub sky_zenith: Vec3,
    /// Horizon and fog colour.
    pub sky_horizon: Vec3,
    /// Fog extinction per metre at sea level (world height 0).
    pub fog_density: f32,
    /// Height decay of the fog per metre: extinction at height `h` is
    /// `fog_density * e^(-fog_decay * h)`; 0 for uniform fog.
    pub fog_decay: f32,
    /// Distance haze extinction per metre (RGB), on top of the fog.
    pub haze: Vec3,
    /// Classic linear fog on top (RV `fogStart`/`fogEnd`, tied to the view distance): full
    /// fog from `fog_end` metres; infinite to disable.
    pub fog_start: f32,
    pub fog_end: f32,
    /// Draw the built-in gradient sky where nothing was drawn. Turn off when a feature (such
    /// as [`SkyFeature`](crate::sky::SkyFeature)) draws the sky.
    pub procedural_sky: bool,
    /// Eye adaptation, tonemapping and anti-aliasing.
    pub hdr: HdrSettings,
    /// Cascaded sun shadows.
    pub shadows: ShadowSettings,
}

impl Default for RenderSettings {
    fn default() -> Self {
        RenderSettings {
            sun_direction: Vec3::new(0.45, 0.6, -0.65).normalize(),
            sun_color: Vec3::new(1.0, 0.95, 0.85),
            ambient: 0.3,
            ambient_sky: Vec3::splat(0.3),
            ambient_mid: Vec3::splat(0.3),
            ambient_ground: Vec3::splat(0.3),
            sky_zenith: Vec3::new(0.12, 0.28, 0.65),
            sky_horizon: Vec3::new(0.62, 0.72, 0.85),
            fog_density: 0.000_08,
            fog_decay: 0.0,
            haze: Vec3::ZERO,
            fog_start: f32::INFINITY,
            fog_end: f32::INFINITY,
            procedural_sky: true,
            hdr: HdrSettings::default(),
            shadows: ShadowSettings::default(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct FrameUniforms {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    viewport: [f32; 4],
    params: [f32; 4],
    // Appended fields: shaders that declare only the fields above keep working.
    ambient_sky: [f32; 4],
    ambient_mid: [f32; 4],
    ambient_ground: [f32; 4],
    // x: fog extinction at sea level, y: fog height decay, z: camera world height,
    // w: 1 when the built-in sky is drawn.
    fog: [f32; 4],
    haze: [f32; 4],
    // x: linear fog end, y: 1 / (end - start); y = 0 disables.
    linear_fog: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct InstanceRaw {
    model: [[f32; 4]; 4],
    color: [f32; 4],
}

impl InstanceRaw {
    const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4
    ];
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct LineVertex {
    position: [f32; 3],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct GlyphRaw {
    position: [f32; 2],
    glyph: u32,
    scale: f32,
    color: [f32; 4],
}

/// A vertex/uniform buffer that grows to fit each frame's data.
struct DynamicBuffer {
    buffer: wgpu::Buffer,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl DynamicBuffer {
    fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        DynamicBuffer {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 1024,
                usage,
                mapped_at_creation: false,
            }),
            usage,
            label,
        }
    }

    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) {
        if bytes.len() as u64 > self.buffer.size() {
            let size = (bytes.len() as u64).next_power_of_two();
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size,
                usage: self.usage,
                mapped_at_creation: false,
            });
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytes);
        }
    }
}

struct MeshBatch {
    mesh: MeshId,
    texture: TextureId,
    instances: std::ops::Range<u32>,
    transparent: bool,
}

struct Targets {
    size: (u32, u32),
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

/// The renderer: owns uploaded meshes and textures, built-in pipelines and registered features.
pub struct Renderer {
    output_format: wgpu::TextureFormat,
    pub settings: RenderSettings,

    frame_layout: wgpu::BindGroupLayout,
    frame_buffer: wgpu::Buffer,
    frame_bind_group: wgpu::BindGroup,
    shadow_maps: ShadowMaps,
    cascade_buffers: Vec<wgpu::Buffer>,
    cascade_groups: Vec<wgpu::BindGroup>,
    active_cascades: u32,
    mesh_shadow: wgpu::RenderPipeline,

    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    meshes: Vec<Mesh>,
    textures: Vec<(GpuTexture, wgpu::BindGroup)>,
    white: TextureId,

    mesh_opaque: wgpu::RenderPipeline,
    mesh_alpha: wgpu::RenderPipeline,
    instances: DynamicBuffer,
    batches: Vec<MeshBatch>,

    line_pipeline: wgpu::RenderPipeline,
    lines: DynamicBuffer,
    line_vertex_count: u32,

    text_pipeline: wgpu::RenderPipeline,
    font_bind_group: wgpu::BindGroup,
    glyphs: DynamicBuffer,
    glyph_count: u32,

    post: PostChain,
    targets: Option<Targets>,

    features: Vec<Box<dyn RenderFeature>>,
}

impl Renderer {
    /// HDR colour target of the scene pass (opaque + alpha phases).
    pub const SCENE_COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
    /// Reversed-Z depth buffer: cleared to 0, nearer is greater.
    pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

    /// Create a renderer drawing into targets of `output_format` (the surface format, or
    /// `Rgba8UnormSrgb` for offscreen rendering).
    pub fn new(gpu: &Gpu, output_format: wgpu::TextureFormat) -> Renderer {
        let device = &gpu.device;
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame layout"),
            entries: &{
                let [map, sampler, cascades] = shadow::frame_layout_entries();
                [
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    map,
                    sampler,
                    cascades,
                ]
            },
        });
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame uniforms"),
            size: std::mem::size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_maps = ShadowMaps::new(device, 1, 16);
        let frame_bind_group = frame_group(
            device,
            &frame_layout,
            &frame_buffer,
            &shadow_maps,
            &shadow_maps.array_view,
        );
        let cascade_buffers: Vec<wgpu::Buffer> = (0..MAX_CASCADES)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("cascade frame uniforms"),
                    size: std::mem::size_of::<FrameUniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let cascade_groups = cascade_buffers
            .iter()
            .map(|b| {
                frame_group(
                    device,
                    &frame_layout,
                    b,
                    &shadow_maps,
                    &shadow_maps.dummy_view,
                )
            })
            .collect();

        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material texture layout"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Float { filterable: true }),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });

        let mesh_shader = shader(device, "mesh", include_str!("../shaders/mesh.wgsl"));
        let mesh_layout = pipeline_layout(device, "mesh", &[&frame_layout, &texture_layout]);
        let mesh_pipeline = |transparent: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if transparent {
                    "mesh alpha"
                } else {
                    "mesh opaque"
                }),
                layout: Some(&mesh_layout),
                vertex: wgpu::VertexState {
                    module: &mesh_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(Vertex::layout()),
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
                    cull_mode: if transparent {
                        None
                    } else {
                        Some(wgpu::Face::Back)
                    },
                    ..Default::default()
                },
                depth_stencil: Some(depth_state(!transparent, wgpu::CompareFunction::Greater)),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &mesh_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: Self::SCENE_COLOR_FORMAT,
                        blend: transparent.then_some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let mesh_shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh shadow"),
            layout: Some(&mesh_layout),
            vertex: wgpu::VertexState {
                module: &mesh_shader,
                entry_point: Some("vs_shadow"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(Vertex::layout()),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<InstanceRaw>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &InstanceRaw::ATTRIBUTES,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                front_face: wgpu::FrontFace::Cw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
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
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let mesh_opaque = mesh_pipeline(false);
        let mesh_alpha = mesh_pipeline(true);

        let line_shader = shader(
            device,
            "debug lines",
            include_str!("../shaders/debug_lines.wgsl"),
        );
        let line_layout = pipeline_layout(device, "debug lines", &[&frame_layout]);
        let line_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("debug lines"),
            layout: Some(&line_layout),
            vertex: wgpu::VertexState {
                module: &line_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<LineVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &line_attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(depth_state(true, wgpu::CompareFunction::GreaterEqual)),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &line_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(Self::SCENE_COLOR_FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });

        let font_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("font layout"),
            entries: &[texture_entry(
                0,
                wgpu::TextureSampleType::Float { filterable: false },
            )],
        });
        let font_bind_group = create_font(gpu, &font_layout);
        let text_shader = shader(device, "text", include_str!("../shaders/text.wgsl"));
        let text_layout = pipeline_layout(device, "text", &[&frame_layout, &font_layout]);
        let glyph_attributes =
            wgpu::vertex_attr_array![0 => Float32x2, 1 => Uint32, 2 => Float32, 3 => Float32x4];
        let text_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("text"),
            layout: Some(&text_layout),
            vertex: wgpu::VertexState {
                module: &text_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GlyphRaw>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &glyph_attributes,
                })],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &text_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let post = PostChain::new(device, &frame_layout, output_format);

        let mut renderer = Renderer {
            output_format,
            settings: RenderSettings::default(),
            frame_layout,
            frame_buffer,
            frame_bind_group,
            shadow_maps,
            cascade_buffers,
            cascade_groups,
            active_cascades: 0,
            mesh_shadow,
            texture_layout,
            sampler,
            meshes: Vec::new(),
            textures: Vec::new(),
            white: TextureId(0),
            mesh_opaque,
            mesh_alpha,
            instances: DynamicBuffer::new(device, "mesh instances", wgpu::BufferUsages::VERTEX),
            batches: Vec::new(),
            line_pipeline,
            lines: DynamicBuffer::new(device, "debug lines", wgpu::BufferUsages::VERTEX),
            line_vertex_count: 0,
            text_pipeline,
            font_bind_group,
            glyphs: DynamicBuffer::new(device, "glyphs", wgpu::BufferUsages::VERTEX),
            glyph_count: 0,
            post,
            targets: None,
            features: Vec::new(),
        };
        renderer.white = renderer
            .upload_texture(gpu, &TextureData::solid_rgba8([255; 4]), ColorSpace::Srgb)
            .expect("a 1x1 RGBA8 texture is always valid");
        renderer
    }

    /// Format of the targets passed to [`render`](Self::render).
    pub fn output_format(&self) -> wgpu::TextureFormat {
        self.output_format
    }

    /// Bind group layout of the frame uniforms (group 0 of every scene pipeline).
    pub fn frame_layout(&self) -> &wgpu::BindGroupLayout {
        &self.frame_layout
    }

    /// Register a feature; it is prepared and drawn every frame after the built-ins.
    pub fn add_feature(&mut self, feature: Box<dyn RenderFeature>) {
        self.features.push(feature);
    }

    /// Upload a mesh.
    pub fn upload_mesh(&mut self, gpu: &Gpu, data: &MeshData) -> MeshId {
        self.meshes
            .push(Mesh::upload(&gpu.device, data, Some("mesh")));
        MeshId(self.meshes.len() as u32 - 1)
    }

    /// Upload a texture (with all its mips) for use by [`MeshDraw`](crate::MeshDraw)s.
    pub fn upload_texture(
        &mut self,
        gpu: &Gpu,
        data: &TextureData,
        color_space: ColorSpace,
    ) -> Result<TextureId, TextureError> {
        let texture = GpuTexture::upload(&gpu.device, &gpu.queue, data, color_space, None)?;
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material texture"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.textures.push((texture, bind_group));
        Ok(TextureId(self.textures.len() as u32 - 1))
    }

    /// Whether the device can sample BC1-3 textures natively.
    pub fn supports_bc(gpu: &Gpu) -> bool {
        gpu.device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    }

    /// Jump straight to the target exposure on the next frame (after a camera cut).
    pub fn reset_eye_adaptation(&mut self) {
        self.post.reset_adaptation();
    }

    /// Render one frame into `target` (of [`output_format`](Self::output_format)) of `size`.
    /// `dt` is the frame time in seconds, used for eye adaptation.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        target: &wgpu::TextureView,
        size: (u32, u32),
        camera: &Camera,
        draws: &DrawList,
        dt: f32,
    ) {
        let size = (size.0.max(1), size.1.max(1));
        self.ensure_targets(gpu, size);
        let aspect = size.0 as f32 / size.1 as f32;
        let view_projection = camera.view_projection(aspect);
        let frame = self.write_frame_uniforms(gpu, camera, view_projection, size);
        self.prepare_shadows(gpu, camera, aspect, &frame);
        self.prepare_meshes(gpu, camera.position, draws);
        self.prepare_lines(gpu, camera.position, draws);
        self.prepare_text(gpu, draws);

        let cx = PrepareContext {
            device: &gpu.device,
            queue: &gpu.queue,
            camera,
            view_projection,
            viewport: size,
            draws,
        };
        for feature in &mut self.features {
            feature.prepare(&cx);
        }

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        self.encode_shadows(&mut encoder);
        let targets = self.targets.as_ref().expect("ensured above");
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            self.draw_meshes(&mut pass, false);
            self.draw_lines(&mut pass);
            for feature in &self.features {
                pass.set_bind_group(0, &self.frame_bind_group, &[]);
                feature.draw(Phase::Opaque, &mut pass);
            }
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            self.draw_meshes(&mut pass, true);
            for feature in &self.features {
                pass.set_bind_group(0, &self.frame_bind_group, &[]);
                feature.draw(Phase::Alpha, &mut pass);
            }
        }
        self.post.encode(
            &gpu.queue,
            &mut encoder,
            &self.frame_bind_group,
            target,
            &self.settings.hdr,
            dt,
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            for feature in &self.features {
                pass.set_bind_group(0, &self.frame_bind_group, &[]);
                feature.draw(Phase::Ui, &mut pass);
            }
            if self.glyph_count > 0 {
                pass.set_pipeline(&self.text_pipeline);
                pass.set_bind_group(0, &self.frame_bind_group, &[]);
                pass.set_bind_group(1, &self.font_bind_group, &[]);
                pass.set_vertex_buffer(0, self.glyphs.buffer.slice(..));
                pass.draw(0..6, 0..self.glyph_count);
            }
        }
        gpu.queue.submit([encoder.finish()]);
    }

    /// Render one frame offscreen and read it back as tightly packed RGBA8 rows (top row
    /// first). The renderer's output format must be an 8-bit RGBA or BGRA format.
    pub fn render_to_image(
        &mut self,
        gpu: &Gpu,
        width: u32,
        height: u32,
        camera: &Camera,
        draws: &DrawList,
        dt: f32,
    ) -> Result<Vec<u8>, RenderError> {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.output_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        self.render(gpu, &view, (width, height), camera, draws, dt);
        let mut pixels = read_texture(gpu, &texture, width, height)?;
        if matches!(
            self.output_format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            for px in pixels.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        Ok(pixels)
    }

    /// Eye adaptation state after the last submitted frame. Blocks until the GPU is done.
    pub fn read_exposure(&self, gpu: &Gpu) -> Result<ExposureReadout, RenderError> {
        let buffer = self.post.exposure_state();
        let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exposure readback"),
            size: buffer.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, buffer.size());
        gpu.queue.submit([encoder.finish()]);
        let bytes = map_read(gpu, &staging)?;
        let values: &[f32] = bytemuck::cast_slice(&bytes);
        Ok(ExposureReadout {
            adapted_luminance: values[0],
            exposure: values[1],
            average_luminance: values[2],
        })
    }

    fn ensure_targets(&mut self, gpu: &Gpu, size: (u32, u32)) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let make = |label, format| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let color = make("scene color", Self::SCENE_COLOR_FORMAT);
        let depth = make("scene depth", Self::DEPTH_FORMAT);
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        self.post
            .resize(&gpu.device, size, &color_view, &depth_view);
        self.targets = Some(Targets {
            size,
            color_view,
            depth_view,
        });
    }

    fn write_frame_uniforms(
        &self,
        gpu: &Gpu,
        camera: &Camera,
        view_projection: glam::Mat4,
        size: (u32, u32),
    ) -> FrameUniforms {
        let s = &self.settings;
        let (w, h) = (size.0 as f32, size.1 as f32);
        let uniforms = FrameUniforms {
            view_proj: view_projection.to_cols_array_2d(),
            inv_view_proj: view_projection.inverse().to_cols_array_2d(),
            sun_dir: s.sun_direction.normalize_or_zero().extend(0.0).to_array(),
            sun_color: s.sun_color.extend(s.ambient).to_array(),
            sky_zenith: s.sky_zenith.extend(1.0).to_array(),
            sky_horizon: s.sky_horizon.extend(s.fog_density).to_array(),
            viewport: [w, h, 1.0 / w, 1.0 / h],
            params: [camera.near, 0.0, 0.0, 0.0],
            ambient_sky: s.ambient_sky.extend(1.0).to_array(),
            ambient_mid: s.ambient_mid.extend(1.0).to_array(),
            ambient_ground: s.ambient_ground.extend(1.0).to_array(),
            fog: [
                s.fog_density,
                s.fog_decay,
                camera.position.y as f32,
                if s.procedural_sky { 1.0 } else { 0.0 },
            ],
            haze: s.haze.extend(0.0).to_array(),
            linear_fog: if s.fog_end.is_finite() {
                [
                    s.fog_end,
                    1.0 / (s.fog_end - s.fog_start).max(1e-3),
                    0.0,
                    0.0,
                ]
            } else {
                [0.0; 4]
            },
        };
        gpu.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniforms));
        uniforms
    }

    /// Fit the cascades to the camera, (re)create the maps if their shape changed, and upload
    /// the cascade matrices.
    fn prepare_shadows(&mut self, gpu: &Gpu, camera: &Camera, aspect: f32, frame: &FrameUniforms) {
        let settings = self.settings.shadows;
        let wanted = (
            settings.cascades.clamp(1, MAX_CASCADES as u32),
            settings.map_size,
        );
        if settings.enabled && self.shadow_maps.shape != wanted {
            self.shadow_maps = ShadowMaps::new(&gpu.device, wanted.0, wanted.1);
            self.frame_bind_group = frame_group(
                &gpu.device,
                &self.frame_layout,
                &self.frame_buffer,
                &self.shadow_maps,
                &self.shadow_maps.array_view,
            );
            self.cascade_groups = self
                .cascade_buffers
                .iter()
                .map(|b| {
                    frame_group(
                        &gpu.device,
                        &self.frame_layout,
                        b,
                        &self.shadow_maps,
                        &self.shadow_maps.dummy_view,
                    )
                })
                .collect();
        }
        let cascades = if settings.enabled {
            shadow::compute_cascades(camera, aspect, self.settings.sun_direction, &settings)
        } else {
            Vec::new()
        };
        self.active_cascades = cascades.len() as u32;
        let uniforms = ShadowUniforms::new(&cascades, camera, &settings);
        gpu.queue
            .write_buffer(&self.shadow_maps.uniforms, 0, bytemuck::bytes_of(&uniforms));
        for (cascade, buffer) in cascades.iter().zip(&self.cascade_buffers) {
            let light = FrameUniforms {
                view_proj: cascade.view_projection.to_cols_array_2d(),
                ..*frame
            };
            gpu.queue
                .write_buffer(buffer, 0, bytemuck::bytes_of(&light));
        }
    }

    /// One depth pass per cascade: opaque meshes, then features in [`Phase::Shadow`].
    fn encode_shadows(&self, encoder: &mut wgpu::CommandEncoder) {
        for cascade in 0..self.active_cascades {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow cascade"),
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_maps.layer_views[cascade as usize],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            let group = &self.cascade_groups[cascade as usize];
            pass.set_bind_group(0, group, &[]);
            pass.set_pipeline(&self.mesh_shadow);
            pass.set_vertex_buffer(1, self.instances.buffer.slice(..));
            for batch in self.batches.iter().filter(|b| !b.transparent) {
                let mesh = &self.meshes[batch.mesh.0 as usize];
                pass.set_bind_group(1, &self.textures[batch.texture.0 as usize].1, &[]);
                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, batch.instances.clone());
            }
            for feature in &self.features {
                pass.set_bind_group(0, group, &[]);
                feature.draw(Phase::Shadow { cascade }, &mut pass);
            }
        }
    }

    fn prepare_meshes(&mut self, gpu: &Gpu, camera: DVec3, draws: &DrawList) {
        let valid = |id: MeshId| (id.0 as usize) < self.meshes.len();
        let texture_of = |t: Option<TextureId>| match t {
            Some(t) if (t.0 as usize) < self.textures.len() => t,
            _ => self.white,
        };
        let mut opaque: Vec<_> = draws
            .meshes
            .iter()
            .filter(|d| !d.transparent && valid(d.mesh))
            .collect();
        opaque.sort_by_key(|d| (d.mesh, texture_of(d.texture)));
        let mut transparent: Vec<_> = draws
            .meshes
            .iter()
            .filter(|d| d.transparent && valid(d.mesh))
            .map(|d| (d, (d.transform.translation - camera).length_squared()))
            .collect();
        transparent.sort_by(|a, b| b.1.total_cmp(&a.1));

        self.batches.clear();
        let mut raw = Vec::with_capacity(draws.meshes.len());
        let ordered = opaque
            .into_iter()
            .map(|d| (d, false))
            .chain(transparent.into_iter().map(|(d, _)| (d, true)));
        for (draw, is_transparent) in ordered {
            let texture = texture_of(draw.texture);
            let index = raw.len() as u32;
            raw.push(InstanceRaw {
                model: relative_model(&draw.transform, camera),
                color: draw.color,
            });
            match self.batches.last_mut() {
                Some(b)
                    if b.mesh == draw.mesh
                        && b.texture == texture
                        && b.transparent == is_transparent =>
                {
                    b.instances.end = index + 1;
                }
                _ => self.batches.push(MeshBatch {
                    mesh: draw.mesh,
                    texture,
                    instances: index..index + 1,
                    transparent: is_transparent,
                }),
            }
        }
        self.instances
            .write(&gpu.device, &gpu.queue, bytemuck::cast_slice(&raw));
    }

    fn prepare_lines(&mut self, gpu: &Gpu, camera: DVec3, draws: &DrawList) {
        let vertices: Vec<LineVertex> = draws
            .lines
            .segments
            .iter()
            .flat_map(|(a, b, color)| {
                [*a, *b].map(|p| LineVertex {
                    position: (p - camera).as_vec3().to_array(),
                    color: *color,
                })
            })
            .collect();
        self.line_vertex_count = vertices.len() as u32;
        self.lines
            .write(&gpu.device, &gpu.queue, bytemuck::cast_slice(&vertices));
    }

    fn prepare_text(&mut self, gpu: &Gpu, draws: &DrawList) {
        let mut glyphs = Vec::new();
        for run in &draws.text {
            let advance = font::CELL_W as f32 * run.scale;
            let line_height = (font::CELL_H as f32 + 2.0) * run.scale;
            let (mut x, mut y) = (run.x, run.y);
            for c in run.text.chars() {
                if c == '\n' {
                    x = run.x;
                    y += line_height;
                    continue;
                }
                if c != ' ' {
                    glyphs.push(GlyphRaw {
                        position: [x, y],
                        glyph: font::glyph_index(c),
                        scale: run.scale,
                        color: run.color,
                    });
                }
                x += advance;
            }
        }
        self.glyph_count = glyphs.len() as u32;
        self.glyphs
            .write(&gpu.device, &gpu.queue, bytemuck::cast_slice(&glyphs));
    }

    fn draw_meshes(&self, pass: &mut wgpu::RenderPass<'_>, transparent: bool) {
        let pipeline = if transparent {
            &self.mesh_alpha
        } else {
            &self.mesh_opaque
        };
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(1, self.instances.buffer.slice(..));
        for batch in self.batches.iter().filter(|b| b.transparent == transparent) {
            let mesh = &self.meshes[batch.mesh.0 as usize];
            pass.set_bind_group(1, &self.textures[batch.texture.0 as usize].1, &[]);
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, batch.instances.clone());
        }
    }

    fn draw_lines(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.line_vertex_count == 0 {
            return;
        }
        pass.set_pipeline(&self.line_pipeline);
        pass.set_vertex_buffer(0, self.lines.buffer.slice(..));
        pass.draw(0..self.line_vertex_count, 0..1);
    }
}

/// The frame bind group: uniforms in `uniforms`, shadow maps from `maps` (or a dummy while
/// the maps are being rendered).
fn frame_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    maps: &ShadowMaps,
    shadow_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("frame"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(shadow_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&maps.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: maps.uniforms.as_entire_binding(),
            },
        ],
    })
}

pub(crate) fn shader(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

pub(crate) fn pipeline_layout(
    device: &wgpu::Device,
    label: &str,
    groups: &[&wgpu::BindGroupLayout],
) -> wgpu::PipelineLayout {
    let groups: Vec<Option<&wgpu::BindGroupLayout>> = groups.iter().map(|g| Some(*g)).collect();
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &groups,
        immediate_size: 0,
    })
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn depth_state(write: bool, compare: wgpu::CompareFunction) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: Renderer::DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        depth_compare: Some(compare),
        stencil: Default::default(),
        bias: Default::default(),
    }
}

fn create_font(gpu: &Gpu, layout: &wgpu::BindGroupLayout) -> wgpu::BindGroup {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("debug font"),
        size: wgpu::Extent3d {
            width: font::ATLAS_W,
            height: font::ATLAS_H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &font::atlas(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(font::ATLAS_W),
            rows_per_image: Some(font::ATLAS_H),
        },
        wgpu::Extent3d {
            width: font::ATLAS_W,
            height: font::ATLAS_H,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&Default::default());
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("debug font"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&view),
        }],
    })
}

/// Copy a 4-byte-per-pixel texture back to the CPU, removing row padding.
fn read_texture(
    gpu: &Gpu,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, RenderError> {
    let unpadded = width * 4;
    let padded =
        unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);

    let data = map_read(gpu, &buffer)?;
    let mut pixels = Vec::with_capacity((unpadded * height) as usize);
    for row in data.chunks_exact(padded as usize) {
        pixels.extend_from_slice(&row[..unpadded as usize]);
    }
    Ok(pixels)
}

/// Map a `MAP_READ` buffer after all submitted work and copy its contents out.
fn map_read(gpu: &Gpu, buffer: &wgpu::Buffer) -> Result<Vec<u8>, RenderError> {
    let slice = buffer.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| RenderError::Readback(e.to_string()))?;
    rx.recv()
        .map_err(|e| RenderError::Readback(e.to_string()))?
        .map_err(|e| RenderError::Readback(e.to_string()))?;

    let data = slice
        .get_mapped_range()
        .map_err(|e| RenderError::Readback(e.to_string()))?;
    let bytes = data.to_vec();
    drop(data);
    buffer.unmap();
    Ok(bytes)
}
