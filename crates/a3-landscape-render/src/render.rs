//! The terrain [`RenderFeature`]: CDLOD heightmap patches textured with the satellite
//! overview and streamed full-resolution satellite tiles, plus a placeholder sea plane.
//!
//! The sea belongs to the ocean renderer; until it exists, [`TerrainRenderer`] draws a flat
//! placeholder at sea level (`vs_sea`/`fs_sea` in `terrain.wgsl`), shaded by the water depth
//! read from the heightmap. Turn it off with [`TerrainRenderer::without_sea`].

use std::sync::{Arc, Mutex};

use a3_render::wgpu;
use a3_render::wgpu::util::DeviceExt as _;
use a3_render::{
    ColorSpace, Frustum, Gpu, GpuTexture, Phase, PrepareContext, RenderFeature, Renderer,
    TextureData,
};
use bytemuck::{Pod, Zeroable};
use glam::DVec3;

use crate::landscape::Landscape;
use crate::lod::{LodQuadtree, LodSettings, SelectedNode};
use crate::residency::{TileResidency, nearest_tiles};
use crate::satellite::{NO_TILE, SatelliteGrid, Tile};
use crate::stream::{FileReader, TileFormat, TileLoader};

/// Full-resolution satellite tiles kept on the GPU.
pub const TILE_SLOTS: u32 = 128;
/// Tiles whose core is within this distance (m) of the camera are streamed in.
pub const TILE_RADIUS: f32 = 2_600.0;
/// Most tiles uploaded per frame.
const UPLOADS_PER_FRAME: usize = 8;
/// Half-extent of the sea plane around the camera in metres.
const SEA_EXTENT: f32 = 60_000.0;
const MAX_LEVELS: usize = 16;

/// Per-frame numbers for overlays and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TerrainStats {
    /// Areas drawn.
    pub nodes: u32,
    pub triangles: u64,
    /// Full-resolution satellite tiles on the GPU.
    pub resident_tiles: u32,
    /// Tiles requested and not yet uploaded.
    pub pending_tiles: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Params {
    camera: [f32; 4],
    grid: [f32; 4],
    misc: [f32; 4],
    morph: [[f32; 4]; MAX_LEVELS],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct TileGpu {
    u: [f32; 4],
    v: [f32; 4],
    slot: [i32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct NodeInstance {
    rel_origin: [f32; 2],
    grid_origin: [u32; 2],
    step_level: [u32; 2],
}

impl NodeInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![1 => Float32x2, 2 => Uint32x2, 3 => Uint32x2];
}

struct Streaming {
    grid: SatelliteGrid,
    loader: TileLoader,
    residency: TileResidency,
    tiles: Vec<Tile>,
    last_centre: Option<DVec3>,
}

/// Draws the Landscape. Register with [`Renderer::add_feature`].
pub struct TerrainRenderer {
    tree: LodQuadtree,
    cell: f64,
    world_size: f32,
    selected: Vec<SelectedNode>,
    instances: Vec<NodeInstance>,
    full_count: u32,
    quarter_count: u32,

    terrain_pipeline: wgpu::RenderPipeline,
    sea_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    params: Params,
    params_buffer: wgpu::Buffer,
    tiles: Vec<TileGpu>,
    tile_buffer: wgpu::Buffer,
    near_tiles: wgpu::Texture,
    patch_vertices: wgpu::Buffer,
    patch_indices: wgpu::Buffer,
    full_indices: std::ops::Range<u32>,
    quarter_indices: std::ops::Range<u32>,
    instance_buffer: wgpu::Buffer,

    streaming: Option<Streaming>,
    sea: bool,
    stats: Arc<Mutex<TerrainStats>>,
}

impl TerrainRenderer {
    /// Upload the Landscape. `reader` (VFS file access) enables streaming of full-resolution
    /// satellite tiles; without it only the overview is used.
    pub fn new(
        gpu: &Gpu,
        renderer: &Renderer,
        landscape: &Landscape,
        reader: Option<FileReader>,
    ) -> TerrainRenderer {
        let device = &gpu.device;
        let queue = &gpu.queue;
        let tree = LodQuadtree::new(&landscape.heights, LodSettings::default());
        let size = landscape.heights.size;

        let heights = texture_2d(
            gpu,
            "terrain heights",
            (size, size),
            wgpu::TextureFormat::R32Float,
            bytemuck::cast_slice(&landscape.heights.heights),
            4,
        );
        let normals = texture_2d(
            gpu,
            "terrain normals",
            (size, size),
            wgpu::TextureFormat::Rgba8Snorm,
            &landscape.heights.normals_rgba8_snorm(),
            4,
        );
        let overview_data = landscape
            .overview
            .clone()
            .unwrap_or_else(|| TextureData::solid_rgba8(crate::landscape::SEABED_RGBA));
        let overview = GpuTexture::upload(
            device,
            queue,
            &overview_data,
            ColorSpace::Srgb,
            Some("terrain overview"),
        )
        .expect("the overview atlas is a valid RGBA8 texture");
        let cells = landscape.land_cells.max(1);
        let cell_tiles: Vec<u16> =
            if landscape.tiles.cell_tiles.as_slice().len() == (cells * cells) as usize {
                landscape.tiles.cell_tiles.as_slice().to_vec()
            } else {
                vec![NO_TILE; (cells * cells) as usize]
            };
        let cell_tiles = texture_2d(
            gpu,
            "terrain cell tiles",
            (cells, cells),
            wgpu::TextureFormat::R16Uint,
            bytemuck::cast_slice(&cell_tiles),
            2,
        );

        // Full-resolution tile array, shaped like the first satellite tile.
        let first_tile = reader.as_ref().and_then(|read| {
            landscape
                .tiles
                .tiles
                .iter()
                .filter_map(|t| t.satellite.as_ref())
                .find_map(|p| read(p))
                .and_then(|b| TileFormat::probe(&b, Renderer::supports_bc(gpu)))
        });
        let tile_format = first_tile.unwrap_or(TileFormat {
            bc1: false,
            size: 4,
            mips: 1,
        });
        let layers = if first_tile.is_some() { TILE_SLOTS } else { 1 };
        let near_format = if tile_format.bc1 {
            a3_render::TextureFormat::Bc1
        } else {
            a3_render::TextureFormat::Rgba8
        };
        let near_tiles = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("terrain satellite tiles"),
            size: wgpu::Extent3d {
                width: tile_format.size,
                height: tile_format.size,
                depth_or_array_layers: layers,
            },
            mip_level_count: tile_format.mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: near_format.wgpu_format(ColorSpace::Srgb),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let near_view = near_tiles.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let mut tiles: Vec<TileGpu> = landscape.tiles.tiles.iter().map(tile_gpu).collect();
        if tiles.is_empty() {
            tiles.push(TileGpu::zeroed());
        }
        let tile_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain tiles"),
            contents: bytemuck::cast_slice(&tiles),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let mut params = Params::zeroed();
        params.grid = [
            landscape.heights.cell,
            size as f32,
            landscape.world_size,
            landscape.land_cell,
        ];
        params.misc = [
            cells as f32,
            landscape.tiles.tiles.len() as f32,
            SEA_EXTENT,
            0.0,
        ];
        for level in 0..tree.levels().min(MAX_LEVELS as u32) {
            let (start, end) = tree.morph_range(level);
            let (start, end) = (start.min(1e30) as f32, end.min(1e30) as f32);
            params.morph[level as usize] = [start, end, 1.0 / (end - start).max(1e-3), 0.0];
        }
        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let linear_clamp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("terrain linear clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let tile_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("terrain tile sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain layout"),
            entries: &[
                texture_entry(
                    0,
                    wgpu::TextureSampleType::Float { filterable: false },
                    false,
                ),
                texture_entry(
                    1,
                    wgpu::TextureSampleType::Float { filterable: true },
                    false,
                ),
                texture_entry(
                    2,
                    wgpu::TextureSampleType::Float { filterable: true },
                    false,
                ),
                sampler_entry(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(5, wgpu::TextureSampleType::Uint, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(7, wgpu::TextureSampleType::Float { filterable: true }, true),
                sampler_entry(8),
            ],
        });
        let views = [
            heights.create_view(&Default::default()),
            normals.create_view(&Default::default()),
            cell_tiles.create_view(&Default::default()),
        ];
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain"),
            layout: &layout,
            entries: &[
                view_entry(0, &views[0]),
                view_entry(1, &views[1]),
                view_entry(2, &overview.view),
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&linear_clamp),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: params_buffer.as_entire_binding(),
                },
                view_entry(5, &views[2]),
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: tile_buffer.as_entire_binding(),
                },
                view_entry(7, &near_view),
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::Sampler(&tile_sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/terrain.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[Some(renderer.frame_layout()), Some(&layout)],
            immediate_size: 0,
        });
        let terrain_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_terrain"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 8,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Uint32x2],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<NodeInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &NodeInstance::ATTRIBUTES,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(depth_state(true)),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_terrain"),
                compilation_options: Default::default(),
                targets: &[Some(Renderer::SCENE_COLOR_FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sea_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sea"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sea"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(depth_state(true)),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_sea"),
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

        let leaf = tree.settings().leaf_cells;
        let (vertices, indices, full, quarter) = patch(leaf);
        let patch_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain patch vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let patch_indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain patch indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let instance_buffer = instance_buffer(device, 1024);

        let streaming = match (reader, first_tile, landscape.tiles.grid) {
            (Some(reader), Some(format), Some(grid)) => Some(Streaming {
                grid,
                loader: TileLoader::spawn(reader, format, 2),
                residency: TileResidency::new(TILE_SLOTS),
                tiles: landscape.tiles.tiles.clone(),
                last_centre: None,
            }),
            _ => None,
        };

        TerrainRenderer {
            cell: tree.cell(),
            tree,
            world_size: landscape.world_size,
            selected: Vec::new(),
            instances: Vec::new(),
            full_count: 0,
            quarter_count: 0,
            terrain_pipeline,
            sea_pipeline,
            bind_group,
            params,
            params_buffer,
            tiles,
            tile_buffer,
            near_tiles,
            patch_vertices,
            patch_indices,
            full_indices: full,
            quarter_indices: quarter,
            instance_buffer,
            streaming,
            sea: true,
            stats: Arc::default(),
        }
    }

    /// Do not draw the placeholder sea (for a separate ocean renderer).
    pub fn without_sea(mut self) -> Self {
        self.sea = false;
        self
    }

    /// Shared handle to this renderer's per-frame statistics.
    pub fn stats(&self) -> Arc<Mutex<TerrainStats>> {
        self.stats.clone()
    }

    fn select(&mut self, cx: &PrepareContext<'_>) {
        let camera = cx.camera.position;
        let frustum = Frustum::from_view_projection(cx.view_projection);
        self.tree.select(camera, Some(&frustum), &mut self.selected);
        let leaf = self.tree.settings().leaf_cells;
        let mut quarters = Vec::new();
        self.instances.clear();
        for node in &self.selected {
            let instance = NodeInstance {
                rel_origin: [
                    (f64::from(node.x) * self.cell - camera.x) as f32,
                    (f64::from(node.z) * self.cell - camera.z) as f32,
                ],
                grid_origin: [node.x, node.z],
                step_level: [1 << node.level, node.level],
            };
            if node.is_quarter(leaf) {
                quarters.push(instance);
            } else {
                self.instances.push(instance);
            }
        }
        self.full_count = self.instances.len() as u32;
        self.quarter_count = quarters.len() as u32;
        self.instances.extend(quarters);
        let bytes = std::mem::size_of_val(self.instances.as_slice()) as u64;
        if bytes > self.instance_buffer.size() {
            self.instance_buffer = instance_buffer(cx.device, self.instances.len() * 2);
        }
        cx.queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&self.instances),
        );
    }

    fn stream(&mut self, cx: &PrepareContext<'_>) {
        let Some(s) = self.streaming.as_mut() else {
            return;
        };
        let camera = cx.camera.position;
        let moved = s
            .last_centre
            .is_none_or(|c| (c - camera).length() > f64::from(s.grid.step) * 0.1);
        if moved {
            s.last_centre = Some(camera);
            let wanted = nearest_tiles(
                &s.grid,
                &s.tiles,
                self.world_size,
                camera.x as f32,
                camera.z as f32,
                TILE_RADIUS,
            );
            for tile in s.residency.want(wanted) {
                if let Some(path) = s
                    .tiles
                    .get(usize::from(tile))
                    .and_then(|t| t.satellite.clone())
                {
                    s.loader.request(tile, path);
                }
            }
        }
        let mut dirty = false;
        for _ in 0..UPLOADS_PER_FRAME {
            let Some((tile, data)) = s.loader.try_recv() else {
                break;
            };
            let Some(data) = data else {
                // Unreadable: drop the request so the tile does not stay pending.
                s.residency.failed(tile);
                continue;
            };
            let Some(placement) = s.residency.arrived(tile) else {
                continue;
            };
            upload_layer(cx.queue, &self.near_tiles, &data, placement.slot);
            if let Some(old) = placement.evicted {
                self.tiles[usize::from(old)].slot[0] = -1;
            }
            self.tiles[usize::from(tile)].slot[0] = placement.slot as i32;
            dirty = true;
        }
        if dirty {
            cx.queue
                .write_buffer(&self.tile_buffer, 0, bytemuck::cast_slice(&self.tiles));
        }
    }
}

impl RenderFeature for TerrainRenderer {
    fn prepare(&mut self, cx: &PrepareContext<'_>) {
        self.select(cx);
        self.stream(cx);
        let p = cx.camera.position;
        self.params.camera = [p.x as f32, p.y as f32, p.z as f32, 0.0];
        cx.queue
            .write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&self.params));
        let leaf = u64::from(self.tree.settings().leaf_cells);
        if let Ok(mut stats) = self.stats.lock() {
            stats.nodes = self.full_count + self.quarter_count;
            stats.triangles = 2 * leaf * leaf * u64::from(self.full_count)
                + leaf * leaf / 2 * u64::from(self.quarter_count);
            if let Some(s) = &self.streaming {
                stats.resident_tiles = s.residency.resident() as u32;
                stats.pending_tiles = s.residency.pending() as u32;
            }
        }
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>) {
        if phase != Phase::Opaque {
            return;
        }
        pass.set_bind_group(1, &self.bind_group, &[]);
        if self.full_count + self.quarter_count > 0 {
            pass.set_pipeline(&self.terrain_pipeline);
            pass.set_vertex_buffer(0, self.patch_vertices.slice(..));
            pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            pass.set_index_buffer(self.patch_indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(self.full_indices.clone(), 0, 0..self.full_count);
            let quarters = self.full_count..self.full_count + self.quarter_count;
            pass.draw_indexed(self.quarter_indices.clone(), 0, quarters);
        }
        if self.sea {
            pass.set_pipeline(&self.sea_pipeline);
            pass.draw(0..6, 0..1);
        }
    }
}

fn tile_gpu(tile: &Tile) -> TileGpu {
    TileGpu {
        u: [tile.uv.u[0], tile.uv.u[1], tile.uv.u[2], 0.0],
        v: [tile.uv.v[0], tile.uv.v[1], tile.uv.v[2], 0.0],
        slot: [-1, 0, 0, 0],
    }
}

/// Grid patch: `(leaf + 1)^2` vertices of local quad coordinates; indices of the whole patch
/// and of its south-west quarter. Each quad splits along the diagonal from `(i + 1, j)` to
/// `(i, j + 1)` like the engine's terrain, wound clockwise seen from above.
fn patch(
    leaf: u32,
) -> (
    Vec<[u32; 2]>,
    Vec<u32>,
    std::ops::Range<u32>,
    std::ops::Range<u32>,
) {
    let n = leaf + 1;
    let vertices = (0..n * n).map(|k| [k % n, k / n]).collect();
    let mut indices = Vec::new();
    let mut quads = |side: u32| {
        let start = indices.len() as u32;
        for j in 0..side {
            for i in 0..side {
                let v = |di: u32, dj: u32| (j + dj) * n + i + di;
                indices.extend_from_slice(&[v(0, 0), v(0, 1), v(1, 0), v(1, 0), v(0, 1), v(1, 1)]);
            }
        }
        start..indices.len() as u32
    };
    let full = quads(leaf);
    let quarter = quads(leaf / 2);
    (vertices, indices, full, quarter)
}

fn instance_buffer(device: &wgpu::Device, count: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("terrain nodes"),
        size: (count.max(1) * std::mem::size_of::<NodeInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn upload_layer(queue: &wgpu::Queue, texture: &wgpu::Texture, data: &TextureData, layer: u32) {
    let dim = data.format.block_dim();
    for (level, bytes) in data.mips.iter().enumerate() {
        if level as u32 >= texture.mip_level_count() {
            break;
        }
        let layout =
            a3_render::texture::MipLayout::new(data.format, data.width, data.height, level as u32);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(layout.bytes_per_row),
                rows_per_image: Some(layout.rows),
            },
            wgpu::Extent3d {
                width: layout.width.div_ceil(dim) * dim,
                height: layout.height.div_ceil(dim) * dim,
                depth_or_array_layers: 1,
            },
        );
    }
}

fn texture_2d(
    gpu: &Gpu,
    label: &str,
    (width, height): (u32, u32),
    format: wgpu::TextureFormat,
    bytes: &[u8],
    bytes_per_texel: u32,
) -> wgpu::Texture {
    gpu.device.create_texture_with_data(
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        {
            debug_assert_eq!(bytes.len(), (width * height * bytes_per_texel) as usize);
            bytes
        },
    )
}

fn texture_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
    array: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: if array {
                wgpu::TextureViewDimension::D2Array
            } else {
                wgpu::TextureViewDimension::D2
            },
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn view_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

fn depth_state(write: bool) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: Renderer::DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        depth_compare: Some(wgpu::CompareFunction::Greater),
        stencil: Default::default(),
        bias: Default::default(),
    }
}
