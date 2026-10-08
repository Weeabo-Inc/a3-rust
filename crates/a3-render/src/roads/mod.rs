//! Road rendering: textured strips along the road centre lines, draped on the terrain and
//! alpha-blended over it in the [`Phase::Alpha`] pass.
//!
//! The caller supplies the road curves ([`RoadSegment`], cubic Beziers as the engine builds them
//! from the roads shapefile), a terrain height function and one [`RoadMaterial`] per road type
//! (its straight and end textures). [`build_road_meshes`] tessellates them on the CPU into
//! chunks; [`RoadFeature`] uploads the chunks and draws the ones near the camera.
//!
//! Against z-fighting the strips are lifted a few centimetres and drawn with a depth bias
//! towards the camera (reversed-Z: a positive bias).

mod mesh;

use std::mem::size_of;

use glam::Vec3;
use wgpu::util::DeviceExt;

use crate::feature::{Phase, PrepareContext, RenderFeature};
use crate::renderer::{Renderer, pipeline_layout, shader};
use crate::texture::{ColorSpace, GpuTexture, TextureData, TextureError};

pub use mesh::{
    RoadBatch, RoadChunk, RoadMeshSettings, RoadSegment, RoadVertex, build_road_meshes,
};

/// The textures of one road type (RoadsLib `mainStrTex` and `mainTerTex`).
pub struct RoadMaterial {
    /// Texture of straight road (`_ca`, with alpha at the edges).
    pub straight: TextureData,
    /// Texture of road ends.
    pub end: TextureData,
}

struct GpuBatch {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    material: usize,
    end: bool,
}

struct GpuChunk {
    origin: glam::DVec3,
    centre: Vec3,
    radius: f32,
    batches: Vec<GpuBatch>,
}

/// The road [`RenderFeature`].
pub struct RoadFeature {
    pipeline: wgpu::RenderPipeline,
    /// Per material: (straight, end) bind groups.
    materials: Vec<(wgpu::BindGroup, wgpu::BindGroup)>,
    chunks: Vec<GpuChunk>,
    /// One chunk offset (origin minus camera) per chunk, rewritten each frame.
    offsets: wgpu::Buffer,
    /// Chunks drawn this frame.
    visible: Vec<usize>,
    /// Chunks farther than this from the camera are skipped (metres).
    pub draw_distance: f32,
    _textures: Vec<GpuTexture>,
}

/// Instance data: the chunk origin relative to the camera.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ChunkOffset {
    offset: [f32; 3],
}

impl RoadFeature {
    /// Uploads road `chunks` and `materials` for `renderer`'s device and frame layout.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &Renderer,
        materials: &[RoadMaterial],
        chunks: &[RoadChunk],
    ) -> Result<Self, TextureError> {
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("road texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("road sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let mut textures = Vec::new();
        let mut bind = |data: &TextureData| -> Result<wgpu::BindGroup, TextureError> {
            let texture = GpuTexture::upload(device, queue, data, ColorSpace::Srgb, Some("road"))?;
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("road material"),
                layout: &texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });
            textures.push(texture);
            Ok(group)
        };
        let materials = materials
            .iter()
            .map(|m| Ok((bind(&m.straight)?, bind(&m.end)?)))
            .collect::<Result<Vec<_>, TextureError>>()?;

        let module = shader(device, "roads", include_str!("../../shaders/roads.wgsl"));
        let layout = pipeline_layout(device, "roads", &[renderer.frame_layout(), &texture_layout]);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("roads"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<RoadVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<ChunkOffset>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![3 => Float32x3],
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: Renderer::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                // Reversed-Z: a positive bias moves the road towards the camera.
                bias: wgpu::DepthBiasState {
                    constant: 8,
                    slope_scale: 1.5,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: Renderer::SCENE_COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let chunks: Vec<GpuChunk> = chunks
            .iter()
            .map(|c| GpuChunk {
                origin: c.origin,
                centre: c.centre,
                radius: c.radius,
                batches: c
                    .batches
                    .iter()
                    .filter(|b| !b.indices.is_empty())
                    .map(|b| GpuBatch {
                        vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("road vertices"),
                            contents: bytemuck::cast_slice(&b.vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        }),
                        indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("road indices"),
                            contents: bytemuck::cast_slice(&b.indices),
                            usage: wgpu::BufferUsages::INDEX,
                        }),
                        count: b.indices.len() as u32,
                        material: b.material as usize,
                        end: b.end,
                    })
                    .collect(),
            })
            .collect();
        let offsets = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("road chunk offsets"),
            size: (chunks.len().max(1) * size_of::<ChunkOffset>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pipeline,
            materials,
            chunks,
            offsets,
            visible: Vec::new(),
            draw_distance: 4000.0,
            _textures: textures,
        })
    }

    /// Number of chunks drawn in the last prepared frame.
    pub fn visible_chunks(&self) -> usize {
        self.visible.len()
    }
}

impl RenderFeature for RoadFeature {
    fn prepare(&mut self, cx: &PrepareContext<'_>) {
        let camera = cx.camera.position;
        let mut offsets = Vec::with_capacity(self.chunks.len());
        self.visible.clear();
        for (i, chunk) in self.chunks.iter().enumerate() {
            let rel = chunk.origin - camera;
            let offset = Vec3::new(rel.x as f32, rel.y as f32, rel.z as f32);
            offsets.push(ChunkOffset {
                offset: offset.to_array(),
            });
            let centre_distance = (offset + chunk.centre).length();
            if centre_distance - chunk.radius <= self.draw_distance {
                self.visible.push(i);
            }
        }
        if !offsets.is_empty() {
            cx.queue
                .write_buffer(&self.offsets, 0, bytemuck::cast_slice(&offsets));
        }
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>) {
        if phase != Phase::Alpha || self.visible.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(1, self.offsets.slice(..));
        // Straight textures first, end caps over them.
        for end in [false, true] {
            for &i in &self.visible {
                let chunk = &self.chunks[i];
                for batch in chunk.batches.iter().filter(|b| b.end == end) {
                    let Some((straight, end_group)) = self.materials.get(batch.material) else {
                        continue;
                    };
                    pass.set_bind_group(1, if end { end_group } else { straight }, &[]);
                    pass.set_vertex_buffer(0, batch.vertices.slice(..));
                    pass.set_index_buffer(batch.indices.slice(..), wgpu::IndexFormat::Uint32);
                    let instance = i as u32;
                    pass.draw_indexed(0..batch.count, 0, instance..instance + 1);
                }
            }
        }
    }
}
