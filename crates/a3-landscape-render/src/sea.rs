//! The sea: the engine's ocean surface (`docs/re/render-water.md`).
//!
//! - [`WaveField`]: the wave function the vertex shader and physics share (§3.1).
//! - [`patch_mesh`] / [`subdivisions`]: the six LOD meshes of a sea patch (§2).
//! - [`patch_lod`]: which LOD a patch at some distance draws.
//! - [`SeaRenderer`]: the [`RenderFeature`] drawing the patches in [`Phase::Water`] with the
//!   `PSWater` shading: normal maps, sky reflection, screen-space reflection, refraction of the
//!   opaque scene, underwater fog of what it refracts, foam and sun glint.
//!
//! The per-frame state (time, waves, overcast, light, sky reflection textures) lives behind a
//! shared [`SeaHandle`] that the game updates from the environment.

use std::f32::consts::{PI, TAU};
use std::sync::{Arc, Mutex};

use a3_core::VfsPath;
use a3_landscape::{SeaWaves, WaterExPars};
use a3_render::wgpu;
use a3_render::wgpu::util::DeviceExt as _;
use a3_render::{
    ColorSpace, Frustum, Gpu, GpuTexture, Phase, PrepareContext, RenderFeature, Renderer,
    TextureData, TextureFormat,
};
use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Vec3};

use crate::heights::HeightField;
use crate::stream::FileReader;

/// Water cells along each side of a sea patch (the engine's `N`).
pub const PATCH_CELLS: u32 = 8;
/// LOD meshes of a patch.
pub const LOD_COUNT: u32 = 6;
/// Angular wave units per turn: `angle · 79.577472` (500 per full circle).
const ANGLE_UNITS: f32 = 79.577_47;

/// The waves at one moment (`docs/re/render-water.md` §3.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveField {
    /// Wave amplitude: `waves · 0.5 · SeaWaveHScale`.
    pub amplitude: f32,
    /// Radial phase, 0..1 (`(t mod XDuration) / XDuration`).
    pub phase_x: f32,
    /// Angular phase, 0..8 (`(t mod 8·ZDuration) / ZDuration`).
    pub phase_z: f32,
    /// `WaterGrid · SeaWaveXScale`: radial cycles per water cell.
    pub u_scale: f32,
    /// `WaterGrid · SeaWaveZScale`: angular cycles per angle unit.
    pub v_scale: f32,
    /// `WaterGrid`: water cell edge in metres.
    pub grid: f32,
    /// The map centre in water cells: the rings' centre.
    pub centre: f32,
    /// The sea level (with the tide).
    pub sea_level: f32,
}

impl WaveField {
    /// The waves `weather_ms` milliseconds into the weather's clock, at wave strength `waves`
    /// (`setWaves`, 0..1), over a map `map_size` metres wide.
    pub fn new(
        config: &SeaWaves,
        map_size: f32,
        sea_level: f32,
        waves: f32,
        weather_ms: i64,
    ) -> WaveField {
        let x_period = i64::from(config.x_duration_ms.max(1));
        let z_period = i64::from(config.z_duration_ms.max(1));
        WaveField {
            amplitude: waves * 0.5 * config.h_scale,
            phase_x: weather_ms.rem_euclid(x_period) as f32 / x_period as f32,
            phase_z: weather_ms.rem_euclid(8 * z_period) as f32 / z_period as f32,
            u_scale: config.water_grid * config.x_scale,
            v_scale: config.water_grid * config.z_scale,
            grid: config.water_grid,
            centre: map_size * 0.5 / config.water_grid,
            sea_level,
        }
    }

    /// The wave coordinates `(u', v)` at world `(x, z)`: radial and angular wave position.
    pub fn coordinates(&self, x: f32, z: f32) -> (f32, f32) {
        let gx = x / self.grid - self.centre;
        let gz = z / self.grid - self.centre;
        let radius = (gx * gx + gz * gz).sqrt();
        let angle = gx.atan2(gz) * ANGLE_UNITS;
        let u = radius * self.u_scale + self.phase_x;
        let v = angle * self.v_scale + self.phase_z;
        let wobble = 0.2 * wave_sin(0.25 * v) + 0.12 * wave_sin(0.375 * v);
        (u + wobble, v)
    }

    /// The wave displacement at world `(x, z)` before the depth attenuation:
    /// `A·(sin(2πu' − π) + sin(2πv − π))`.
    pub fn displacement(&self, x: f32, z: f32) -> f32 {
        let (u, v) = self.coordinates(x, z);
        self.amplitude * (wave_sin(u) + wave_sin(v))
    }

    /// The depth attenuation of the waves over terrain at `terrain` height: full from 8 m
    /// deep, none shallower than 1 m.
    pub fn depth_factor(&self, terrain: f32) -> f32 {
        let k = ((-1.0 - (terrain - self.sea_level)) / 7.0).clamp(0.0, 1.0);
        k * k
    }

    /// The sea surface height at world `(x, z)`, where the terrain is at `terrain` (the
    /// engine's CPU wave height, used by physics; the vertex shader adds a distance fade).
    pub fn surface_height(&self, x: f32, z: f32, terrain: f32) -> f32 {
        self.sea_level + self.displacement(x, z) * self.depth_factor(terrain)
    }
}

/// `sin(2π·frac(x) − π)`, the engine's wave sine (= −sin(2πx)).
fn wave_sin(x: f32) -> f32 {
    (x.fract().rem_euclid(1.0) * TAU - PI).sin()
}

/// Subdivisions per water cell of the finest patch mesh, from the scene complexity (objects
/// quality) the way the engine sizes its sea mesh (`docs/re/render-water.md` §2).
pub fn subdivisions(scene_complexity: f32, water_grid: f32) -> u32 {
    let budget = (scene_complexity * 0.1).round().clamp(10_000.0, 300_000.0);
    let n = PATCH_CELLS as f32;
    let r = (390.0 / (n * water_grid) + 0.5).round().max(1.0);
    let s = (budget / (r * r * n * n)).sqrt().round().clamp(1.0, 16.0) as u32;
    let step = 32 / PATCH_CELLS;
    if step > 1 {
        (s & !(step - 1)).max(step)
    } else {
        s
    }
}

/// The mesh of LOD `lod` of a sea patch: vertices (x, z) in metres around the patch centre
/// and triangle indices, clockwise seen from above. Border vertices are pushed out 1 cm so
/// that neighbouring patches overlap; quads alternate their diagonal in a checkerboard.
pub fn patch_mesh(lod: u32, subdivisions: u32, water_grid: f32) -> (Vec<[f32; 2]>, Vec<u32>) {
    let n = (subdivisions * PATCH_CELLS) >> lod;
    let step = (1u32 << lod) as f32 / subdivisions as f32 * water_grid;
    let half = n as f32 * 0.5;
    let coordinate = |i: u32| {
        let mut c = (i as f32 - half) * step;
        if i == 0 {
            c -= 0.01;
        } else if i == n {
            c += 0.01;
        }
        c
    };
    let mut vertices = Vec::with_capacity(((n + 1) * (n + 1)) as usize);
    for j in 0..=n {
        for i in 0..=n {
            vertices.push([coordinate(i), coordinate(j)]);
        }
    }
    let mut indices = Vec::with_capacity((n * n * 6) as usize);
    let v = |i: u32, j: u32| j * (n + 1) + i;
    for j in 0..n {
        for i in 0..n {
            let (a, b, c, d) = (v(i, j), v(i + 1, j), v(i, j + 1), v(i + 1, j + 1));
            // z grows north; seen from above (x right, z up the screen) this is clockwise.
            if (i + j) % 2 == 0 {
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            } else {
                indices.extend_from_slice(&[a, c, d, a, d, b]);
            }
        }
    }
    (vertices, indices)
}

/// The LOD a patch draws (`docs/re/render-water.md` §2). `distance` is from the camera to the
/// patch centre minus the patch radius, `zoom` is `sqrt(P00·P11)` of the projection,
/// `patch_size` the patch edge in metres; `deep` tells whether all four patch corners are at
/// least 10 m deep (only then are LODs 4 and 5 used).
pub fn patch_lod(distance: f32, zoom: f32, near: f32, patch_size: f32, deep: bool) -> u32 {
    if distance / zoom * 0.01 < 1.0 {
        return 0;
    }
    let lod = (distance.max(near) / patch_size).round().max(0.0) as u32;
    match lod {
        0..=3 => lod,
        _ if deep => lod.min(LOD_COUNT - 1),
        _ => 3,
    }
}

/// The sea's world settings: `CfgWorlds >> <world> >> Sea` and `WaterExPars`.
#[derive(Debug, Clone, PartialEq)]
pub struct SeaConfig {
    pub waves: SeaWaves,
    pub water_ex: WaterExPars,
    /// The material's stage textures: wave normals, fine normals and foam
    /// (`CfgMaterials >> Water`).
    pub normal_map: VfsPath,
    pub detail_normal_map: VfsPath,
    pub foam: VfsPath,
    /// Objects quality's scene complexity (`render-lod.md` §1), sizes the patch mesh.
    pub scene_complexity: f32,
    /// The video option "screen-space reflections": off sets the strength to 0, as the
    /// engine does.
    pub screen_space_reflections: bool,
}

impl SeaConfig {
    /// The base game's `CfgMaterials >> Water` textures with the given world settings.
    pub fn new(waves: SeaWaves, water_ex: WaterExPars) -> SeaConfig {
        SeaConfig {
            waves,
            water_ex,
            normal_map: VfsPath::new(r"a3\data_f_exp\water_nofhq.paa"),
            detail_normal_map: VfsPath::new(r"a3\data_f_exp\water2_nohq.paa"),
            foam: VfsPath::new(r"a3\data_f_exp\sea_foam_lco.paa"),
            scene_complexity: 900_000.0,
            screen_space_reflections: true,
        }
    }
}

/// What the sea shows this frame. Colours are linear HDR in the scene's units.
#[derive(Debug, Clone, PartialEq)]
pub struct SeaParams {
    /// Global time in seconds (normal map scroll and ripple animation).
    pub time: f64,
    /// The weather's clock in milliseconds (wave phases).
    pub weather_ms: i64,
    /// Wave strength 0..1 (`waves`).
    pub waves: f32,
    /// Cloudiness 0..1 (the shader's `o`).
    pub overcast: f32,
    /// Sea level including the tide.
    pub sea_level: f32,
    /// Unit vector towards the main light (sun or moon).
    pub light_direction: Vec3,
    /// Main light colour (RV `diffuse`).
    pub diffuse: Vec3,
    /// Sky light colour (RV `ambient`).
    pub ambient: Vec3,
    /// Water fog colour (`PSC_WaterFogColor`).
    pub water_fog_color: Vec3,
    /// The overcast sample's sky reflection textures and the blend towards the second.
    pub sky_reflection: (String, String, f32),
    /// View distance in metres: how far patches are drawn and screen-space reflections reach.
    pub view_distance: f32,
}

impl Default for SeaParams {
    fn default() -> Self {
        SeaParams {
            time: 0.0,
            weather_ms: 0,
            waves: 0.0,
            overcast: 0.0,
            sea_level: 0.0,
            light_direction: Vec3::new(0.45, 0.6, -0.65).normalize(),
            diffuse: Vec3::splat(1.0),
            ambient: Vec3::new(0.3, 0.35, 0.45),
            water_fog_color: Vec3::new(0.03, 0.07, 0.09),
            sky_reflection: (String::new(), String::new(), 0.0),
            view_distance: 3000.0,
        }
    }
}

/// Shared handle to the sea's per-frame parameters.
pub type SeaHandle = Arc<Mutex<SeaParams>>;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct SeaUniforms {
    // A, A, 2π·XScale·A, 2π·ZScale·A (VSC_WaveHeight)
    wave: [f32; 4],
    // u scale, v scale, phase x, phase z
    wave_uv: [f32; 4],
    // 1 / WaterGrid, centre (cells), lattice points per side, sea level
    grid: [f32; 4],
    // time (s), camera world height, 1 / sqrt(P00·P11), P00
    misc: [f32; 4],
    // camera forward (unit), near plane
    forward: [f32; 4],
    // camera world x, z, view distance, unused
    camera: [f32; 4],
    cwp0: [f32; 4],
    cwp1: [f32; 4],
    cwp3: [f32; 4],
    cwp4: [f32; 4],
    ap0: [f32; 4],
    ap1: [f32; 4],
    ap2: [f32; 4],
    ssr0: [f32; 4],
    ssr1: [f32; 4],
    // foam colour (PSC_WaveColor)
    wave_color: [f32; 4],
    // sun specular colour (PSC_Specular)
    specular: [f32; 4],
    // direction the light travels (unit), unused
    light: [f32; 4],
    // sky reflection tint (GlassEnvColor), w: blend towards the second sky texture
    env: [f32; 4],
    // water fog colour, w: extinction per metre
    water_fog: [f32; 4],
    // water fog gradient (down, horizon, up)
    water_gradient: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct PatchInstance {
    /// Patch centre minus camera (x, z).
    rel: [f32; 2],
    /// Patch centre in world metres (x, z).
    world: [f32; 2],
}

impl PatchInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![1 => Float32x2, 2 => Float32x2];
}

struct LodMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

/// Per-frame numbers for overlays and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SeaStats {
    /// Patches drawn, per LOD.
    pub patches: [u32; LOD_COUNT as usize],
}

/// Draws the sea. Register with [`Renderer::add_feature`].
pub struct SeaRenderer {
    config: SeaConfig,
    heights: HeightField,
    world_size: f32,
    /// Lowest terrain of every water cell over the map (row-major, `cells` per side).
    cell_min: Vec<f32>,
    cells: u32,
    subdivisions: u32,
    params: SeaHandle,
    stats: Arc<Mutex<SeaStats>>,
    reader: Option<FileReader>,

    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    lattice_view: wgpu::TextureView,
    normal: GpuTexture,
    detail_normal: GpuTexture,
    foam: GpuTexture,
    sky: [GpuTexture; 2],
    sky_names: (String, String),
    repeat: wgpu::Sampler,
    clamp: wgpu::Sampler,
    exposure: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    meshes: Vec<LodMesh>,
    instances: wgpu::Buffer,
    /// Instance range of each LOD in `instances`.
    ranges: [std::ops::Range<u32>; LOD_COUNT as usize],
}

impl SeaRenderer {
    /// Build the sea over `heights` (a terrain `world_size` metres wide). `reader` loads the
    /// material and sky textures; without it they are flat stand-ins.
    pub fn new(
        gpu: &Gpu,
        renderer: &Renderer,
        heights: &HeightField,
        world_size: f32,
        config: SeaConfig,
        reader: Option<FileReader>,
    ) -> (SeaRenderer, SeaHandle) {
        let device = &gpu.device;
        let grid = config.waves.water_grid.max(1.0);
        let lattice_points = (world_size / grid).ceil() as u32 + 1;
        let lattice: Vec<f32> = (0..lattice_points * lattice_points)
            .map(|k| {
                let (i, j) = (k % lattice_points, k / lattice_points);
                heights.sample(i as f32 * grid, j as f32 * grid)
            })
            .collect();
        let lattice_texture = device.create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("sea depth lattice"),
                size: wgpu::Extent3d {
                    width: lattice_points,
                    height: lattice_points,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytemuck::cast_slice(&lattice),
        );
        let lattice_view = lattice_texture.create_view(&Default::default());
        let cells = (world_size / grid).ceil().max(1.0) as u32;
        let cell_min = cell_minimums(heights, cells, grid);

        let load = |path: &VfsPath, fallback: [u8; 4], space: ColorSpace, label: &str| {
            let data = reader
                .as_ref()
                .and_then(|read| read(path))
                .and_then(|bytes| decode_paa(&bytes))
                .unwrap_or_else(|| {
                    if reader.is_some() {
                        log::warn!("sea: cannot load {path}");
                    }
                    TextureData::solid_rgba8(fallback)
                });
            GpuTexture::upload(device, &gpu.queue, &data, space, Some(label))
                .expect("decoded PAA mips are valid RGBA8")
        };
        // Flat normal: x and y are stored in green and alpha.
        let flat = [128, 128, 255, 128];
        let normal = load(&config.normal_map, flat, ColorSpace::Linear, "sea normals");
        let detail_normal = load(
            &config.detail_normal_map,
            flat,
            ColorSpace::Linear,
            "sea detail normals",
        );
        let foam = load(&config.foam, [0, 0, 0, 255], ColorSpace::Srgb, "sea foam");
        let sky_texture = |label| {
            GpuTexture::upload(
                device,
                &gpu.queue,
                &TextureData::solid_rgba8([60, 70, 85, 255]),
                ColorSpace::Linear,
                Some(label),
            )
            .expect("1x1 RGBA8")
        };
        let sky = [sky_texture("sea sky 0"), sky_texture("sea sky 1")];

        let sampler = |label, mode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: mode,
                address_mode_v: mode,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                anisotropy_clamp: 16,
                ..Default::default()
            })
        };
        let repeat = sampler("sea repeat", wgpu::AddressMode::Repeat);
        let clamp = sampler("sea clamp", wgpu::AddressMode::ClampToEdge);
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sea uniforms"),
            size: std::mem::size_of::<SeaUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sea layout"),
            entries: &[
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
                texture_entry(1, wgpu::TextureSampleType::Float { filterable: false }),
                texture_entry(2, float),
                texture_entry(3, float),
                texture_entry(4, float),
                texture_entry(5, float),
                texture_entry(6, float),
                sampler_entry(7),
                sampler_entry(8),
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let exposure = renderer.exposure_buffer().clone();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sea"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/sea.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sea"),
            bind_group_layouts: &[
                Some(renderer.frame_layout()),
                Some(&layout),
                Some(renderer.scene_copy_layout()),
            ],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sea"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sea"),
                compilation_options: Default::default(),
                buffers: &[
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 8,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<PatchInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &PatchInstance::ATTRIBUTES,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Front faces look up: the shader tells the underside by the facing.
                front_face: wgpu::FrontFace::Cw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: Renderer::DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_sea"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: Renderer::SCENE_COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let subdivisions = subdivisions(config.scene_complexity, grid);
        let meshes = (0..LOD_COUNT)
            .map(|lod| {
                let (vertices, indices) = patch_mesh(lod, subdivisions, grid);
                LodMesh {
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("sea patch vertices"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("sea patch indices"),
                        contents: bytemuck::cast_slice(&indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                    index_count: indices.len() as u32,
                }
            })
            .collect();
        let instances = instance_buffer(device, 256);
        let handle: SeaHandle = Arc::default();
        let bind_group = sea_bind_group(
            device,
            &layout,
            &uniforms,
            &lattice_view,
            [&normal, &detail_normal, &foam, &sky[0], &sky[1]],
            [&repeat, &clamp],
            &exposure,
        );
        let sea = SeaRenderer {
            config,
            heights: heights.clone(),
            world_size,
            cell_min,
            cells,
            subdivisions,
            params: handle.clone(),
            stats: Arc::default(),
            reader,
            bind_group,
            pipeline,
            layout,
            uniforms,
            lattice_view,
            normal,
            detail_normal,
            foam,
            sky,
            sky_names: (String::new(), String::new()),
            repeat,
            clamp,
            exposure,
            meshes,
            instances,
            ranges: Default::default(),
        };
        (sea, handle)
    }

    /// Shared handle to this renderer's per-frame statistics.
    pub fn stats(&self) -> Arc<Mutex<SeaStats>> {
        self.stats.clone()
    }

    /// Subdivisions per water cell of the finest patch mesh.
    pub fn mesh_subdivisions(&self) -> u32 {
        self.subdivisions
    }

    fn rebind(&mut self, device: &wgpu::Device) {
        self.bind_group = sea_bind_group(
            device,
            &self.layout,
            &self.uniforms,
            &self.lattice_view,
            [
                &self.normal,
                &self.detail_normal,
                &self.foam,
                &self.sky[0],
                &self.sky[1],
            ],
            [&self.repeat, &self.clamp],
            &self.exposure,
        );
    }
    /// Reload the sky reflection textures when the weather names others.
    fn update_sky(&mut self, cx: &PrepareContext<'_>, names: &(String, String, f32)) {
        if (names.0.as_str(), names.1.as_str())
            == (self.sky_names.0.as_str(), self.sky_names.1.as_str())
        {
            return;
        }
        self.sky_names = (names.0.clone(), names.1.clone());
        let Some(read) = self.reader.clone() else {
            return;
        };
        for (slot, name) in [&names.0, &names.1].into_iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            let data = read(&VfsPath::new(name)).and_then(|bytes| decode_paa(&bytes));
            match data.and_then(|d| {
                // Read linear: an sRGB decode leaves the reflection far darker than the real
                // client's (measured on the oracle's stratis_coast shot).
                GpuTexture::upload(cx.device, cx.queue, &d, ColorSpace::Linear, Some("sea sky"))
                    .ok()
            }) {
                Some(texture) => self.sky[slot] = texture,
                None => log::warn!("sea: cannot load sky reflection {name}"),
            }
        }
        self.rebind(cx.device);
    }

    /// The lowest terrain of the water cell `(i, j)`, clamped to the map.
    fn cell_min(&self, i: i64, j: i64) -> f32 {
        let last = i64::from(self.cells) - 1;
        let (i, j) = (i.clamp(0, last), j.clamp(0, last));
        self.cell_min[(j * i64::from(self.cells) + i) as usize]
    }

    fn select(&mut self, cx: &PrepareContext<'_>, params: &SeaParams) {
        let grid = self.config.waves.water_grid.max(1.0);
        let patch = PATCH_CELLS as f32 * grid;
        let camera = cx.camera.position;
        let reach = f64::from(params.view_distance.max(patch));
        let first = |c: f64| ((c - reach) / f64::from(patch)).floor() as i64;
        let last = |c: f64| ((c + reach) / f64::from(patch)).ceil() as i64;
        let frustum = Frustum::from_view_projection(cx.view_projection);
        let aspect = cx.viewport.0 as f32 / cx.viewport.1.max(1) as f32;
        let projection = cx.camera.projection(aspect);
        let zoom = (projection.x_axis.x * projection.y_axis.y)
            .abs()
            .sqrt()
            .max(1e-3);
        let radius = (2.0 * (patch * 0.5).powi(2) + 0.25).sqrt();
        let limit = params.sea_level + self.config.waves.max_wave;
        let lift = (params.waves * self.config.waves.h_scale).max(0.0) + 1.0;
        let mut by_lod: [Vec<PatchInstance>; LOD_COUNT as usize] = Default::default();
        for pj in first(camera.z)..last(camera.z) {
            for pi in first(camera.x)..last(camera.x) {
                let (x0, z0) = (pi as f32 * patch, pj as f32 * patch);
                let centre = DVec3::new(
                    f64::from(x0 + patch * 0.5),
                    f64::from(params.sea_level),
                    f64::from(z0 + patch * 0.5),
                );
                let distance = (centre - camera).length() as f32;
                if distance > params.view_distance + radius {
                    continue;
                }
                let (ci, cj) = (pi * i64::from(PATCH_CELLS), pj * i64::from(PATCH_CELLS));
                let lowest = (cj..cj + i64::from(PATCH_CELLS))
                    .flat_map(|j| (ci..ci + i64::from(PATCH_CELLS)).map(move |i| (i, j)))
                    .map(|(i, j)| self.cell_min(i, j))
                    .fold(f32::INFINITY, f32::min);
                if lowest > limit {
                    continue;
                }
                let rel = (centre - camera).as_vec3();
                let half = Vec3::new(patch * 0.5, lift, patch * 0.5);
                if !frustum.intersects_aabb(rel - half, rel + half) {
                    continue;
                }
                let deep = [
                    (x0, z0),
                    (x0 + patch, z0),
                    (x0, z0 + patch),
                    (x0 + patch, z0 + patch),
                ]
                .iter()
                .all(|&(x, z)| self.heights.sample(x, z) <= -10.0);
                let lod = patch_lod(distance - radius, zoom, cx.camera.near, patch, deep);
                by_lod[lod as usize].push(PatchInstance {
                    rel: [rel.x, rel.z],
                    world: [centre.x as f32, centre.z as f32],
                });
            }
        }
        let mut all = Vec::new();
        let mut stats = SeaStats::default();
        for (lod, list) in by_lod.iter().enumerate() {
            let start = all.len() as u32;
            all.extend_from_slice(list);
            self.ranges[lod] = start..all.len() as u32;
            stats.patches[lod] = list.len() as u32;
        }
        let bytes = std::mem::size_of_val(all.as_slice()) as u64;
        if bytes > self.instances.size() {
            self.instances = instance_buffer(cx.device, all.len() * 2);
        }
        if !all.is_empty() {
            cx.queue
                .write_buffer(&self.instances, 0, bytemuck::cast_slice(&all));
        }
        if let Ok(mut s) = self.stats.lock() {
            *s = stats;
        }
    }

    fn uniforms(&self, cx: &PrepareContext<'_>, params: &SeaParams) -> SeaUniforms {
        let w = &self.config.waves;
        let ex = &self.config.water_ex;
        let field = WaveField::new(
            w,
            self.world_size,
            params.sea_level,
            params.waves,
            params.weather_ms,
        );
        let t = params.time as f32;
        let o = params.overcast.clamp(0.0, 1.0);
        let x = 1.0 - o;
        let octaves = |speed: f32, scale: f32| {
            let a = f64::from(speed) * params.time;
            std::array::from_fn(|k| {
                scale * ((a + k as f64 * std::f64::consts::FRAC_PI_2).sin() as f32 + 1.0)
            })
        };
        // Light direction as the engine stores it: the way the light travels.
        let l = -params.light_direction.normalize_or(Vec3::NEG_Y);
        let sun_up = ((-l.y + 0.03489) * 3.368_251).clamp(0.0, 1.0);
        let smooth = x * x * (3.0 - 2.0 * x);
        let p0 = ex.specular_power_overcast0.unwrap_or(200.0);
        let p1 = ex.specular_power_overcast1.unwrap_or(50.0);
        let intensity = ex.specular_max_intensity.unwrap_or(25.0);
        let zero = |v: Option<f32>| v.unwrap_or(0.0);
        let aspect = cx.viewport.0 as f32 / cx.viewport.1.max(1) as f32;
        let projection = cx.camera.projection(aspect);
        let (p00, p11) = (projection.x_axis.x.abs(), projection.y_axis.y.abs());
        let camera = cx.camera.position;
        let forward = cx.camera.forward();
        let foam_speed = zero(ex.foam_time_move_speed) * t;
        let foam_amount = zero(ex.foam_time_move_amount);
        let lattice_points = (self.world_size / w.water_grid.max(1.0)).ceil() + 1.0;
        let gradient = ex.fog_gradient_coefs.unwrap_or(Vec3::new(0.4, 1.0, 1.5));
        SeaUniforms {
            wave: [
                field.amplitude,
                field.amplitude,
                TAU * w.x_scale * field.amplitude,
                TAU * w.z_scale * field.amplitude,
            ],
            wave_uv: [field.u_scale, field.v_scale, field.phase_x, field.phase_z],
            grid: [
                1.0 / w.water_grid.max(1.0),
                field.centre,
                lattice_points,
                params.sea_level,
            ],
            misc: [
                (params.time % 100_000.0) as f32,
                camera.y as f32,
                1.0 / (p00 * p11).sqrt().max(1e-3),
                p00,
            ],
            forward: forward.extend(cx.camera.near).to_array(),
            camera: [camera.x as f32, camera.z as f32, params.view_distance, 0.0],
            cwp0: octaves(2.0 * x + 3.0, 0.6),
            cwp1: octaves(2.01 * x + 3.011, 0.5),
            cwp3: [
                0.5 - 0.5 * x,
                4.0 * x + 2.5,
                sun_up * smooth * intensity,
                p0 - o * (p0 - p1),
            ],
            cwp4: [
                x,
                ((-l.y + 0.1736) * 1.677_289_5).clamp(0.0, 1.0),
                1.0 - ((-l.y + 0.1736) * 1.677_289_5).clamp(0.0, 1.0),
                t,
            ],
            ap0: [
                zero(ex.refraction_min_coef),
                zero(ex.refraction_max_coef),
                ex.refraction_max_dist
                    .filter(|d| *d > 0.0)
                    .map_or(0.0, |d| 1.0 / d),
                zero(ex.shadow_intensity),
            ],
            ap1: [
                zero(ex.foam_around_objects_intensity),
                zero(ex.foam_deformation_coef),
                zero(ex.foam_texture_coef),
                zero(ex.foam_around_objects_fade_coef),
            ],
            ap2: [
                foam_amount * foam_speed.sin(),
                foam_amount * foam_speed.cos(),
                zero(ex.specular_normal_modify_coef),
                zero(ex.surface_opacity),
            ],
            ssr0: [
                if self.config.screen_space_reflections {
                    zero(ex.ss_reflection_strength)
                } else {
                    0.0
                },
                params.view_distance,
                zero(ex.ss_reflection_max_jitter),
                zero(ex.ss_reflection_ripple_influence),
            ],
            ssr1: [
                zero(ex.ss_reflection_edge_fading_coef),
                zero(ex.ss_reflection_dist_fading_coef),
                1.0 / p00.max(1e-6),
                1.0 / p11.max(1e-6),
            ],
            wave_color: (0.25 * (params.ambient + params.diffuse) * zero(ex.foam_color_coef))
                .extend(-0.4)
                .to_array(),
            // CfgMaterials >> Water >> specular = 0.12.
            specular: (params.diffuse * 0.12).extend(0.0).to_array(),
            light: l.extend(0.0).to_array(),
            env: (params.ambient + 0.05 * params.diffuse)
                .extend(params.sky_reflection.2.clamp(0.0, 1.0))
                .to_array(),
            water_fog: params
                .water_fog_color
                .extend(ex.fog_density.unwrap_or(0.04))
                .to_array(),
            water_gradient: gradient.extend(0.0).to_array(),
        }
    }
}

impl RenderFeature for SeaRenderer {
    fn prepare(&mut self, cx: &PrepareContext<'_>) {
        let params = self
            .params
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        self.update_sky(cx, &params.sky_reflection);
        self.select(cx, &params);
        let uniforms = self.uniforms(cx, &params);
        cx.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>) {
        if phase != Phase::Water {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(1, self.instances.slice(..));
        for (mesh, range) in self.meshes.iter().zip(&self.ranges) {
            if range.is_empty() {
                continue;
            }
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, range.clone());
        }
    }

    fn wants_scene_copy(&self) -> bool {
        true
    }
}

/// Group 1 of the sea pipeline.
fn sea_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    lattice: &wgpu::TextureView,
    textures: [&GpuTexture; 5],
    [repeat, clamp]: [&wgpu::Sampler; 2],
    exposure: &wgpu::Buffer,
) -> wgpu::BindGroup {
    let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
    let view = |t: usize| wgpu::BindingResource::TextureView(&textures[t].view);
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sea"),
        layout,
        entries: &[
            entry(0, uniforms.as_entire_binding()),
            entry(1, wgpu::BindingResource::TextureView(lattice)),
            entry(2, view(0)),
            entry(3, view(1)),
            entry(4, view(2)),
            entry(5, view(3)),
            entry(6, view(4)),
            entry(7, wgpu::BindingResource::Sampler(repeat)),
            entry(8, wgpu::BindingResource::Sampler(clamp)),
            entry(9, exposure.as_entire_binding()),
        ],
    })
}

/// Lowest terrain height of each water cell (`cells` per side, `grid` metres) over the height
/// samples it covers, edges included.
fn cell_minimums(heights: &HeightField, cells: u32, grid: f32) -> Vec<f32> {
    let step = heights.cell.max(1e-3);
    let mut out = vec![f32::INFINITY; (cells * cells) as usize];
    for j in 0..cells {
        for i in 0..cells {
            let (x0, z0) = (i as f32 * grid, j as f32 * grid);
            let a = |c: f32| (c / step).floor() as i64;
            let b = |c: f32| ((c + grid) / step).ceil() as i64;
            let mut m = f32::INFINITY;
            for hj in a(z0)..=b(z0) {
                for hi in a(x0)..=b(x0) {
                    m = m.min(heights.at(hi, hj));
                }
            }
            out[(j * cells + i) as usize] = m;
        }
    }
    out
}

/// Decode a PAA with all its mips to RGBA8.
fn decode_paa(bytes: &[u8]) -> Option<TextureData> {
    let texture = a3_paa::Texture::read(bytes).ok()?;
    let mips = texture
        .mips
        .iter()
        .map(|m| a3_paa::decode_rgba8(texture.format, m))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(TextureData {
        format: TextureFormat::Rgba8,
        width: u32::from(texture.width()),
        height: u32::from(texture.height()),
        mips,
    })
}

fn instance_buffer(device: &wgpu::Device, count: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sea patches"),
        size: (count.max(1) * std::mem::size_of::<PatchInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn altis_waves() -> SeaWaves {
        SeaWaves {
            water_map_scale: 20.0,
            water_grid: 50.0,
            max_tide: 0.0,
            max_wave: 0.25,
            x_scale: 2.0 / 50.0,
            z_scale: 1.0 / 50.0,
            h_scale: 1.0,
            x_duration_ms: 5000,
            z_duration_ms: 10000,
        }
    }

    #[test]
    fn wave_phases_wrap_like_the_engine() {
        let w = WaveField::new(&altis_waves(), 30720.0, 0.0, 1.0, 12_345);
        assert!((w.phase_x - 2345.0 / 5000.0).abs() < 1e-6);
        assert!((w.phase_z - 1.2345).abs() < 1e-6);
        // The angular phase runs over eight periods before it wraps.
        let late = WaveField::new(&altis_waves(), 30720.0, 0.0, 1.0, 85_000);
        assert!((late.phase_z - 0.5).abs() < 1e-6);
        assert_eq!(w.amplitude, 0.5, "waves 1 gives half a metre per sine");
        assert_eq!(w.centre, 307.2, "Altis' centre in 50 m water cells");
    }

    #[test]
    fn rings_travel_towards_the_map_centre_one_wavelength_per_period() {
        // Freeze the angular wave so only the rings move.
        let still = SeaWaves {
            z_duration_ms: i32::MAX,
            ..altis_waves()
        };
        let field = |ms| WaveField::new(&still, 30720.0, 0.0, 1.0, ms);
        // Due north of the centre, 2 km out: the angle is 0.
        let (x, z) = (15360.0, 15360.0 + 2000.0);
        let now = field(0).displacement(x, z);
        // A quarter period later the pattern moved a quarter wavelength (25 m / 4) inwards.
        let later = field(1250).displacement(x, z - 6.25);
        assert!((now - later).abs() < 1e-3, "{now} vs {later}");
    }

    #[test]
    fn waves_die_out_in_shallow_water() {
        let w = WaveField::new(&altis_waves(), 30720.0, 0.0, 1.0, 0);
        assert_eq!(w.depth_factor(-20.0), 1.0, "full waves 20 m deep");
        assert_eq!(w.depth_factor(-8.0), 1.0, "full waves from 8 m");
        assert_eq!(w.depth_factor(-1.0), 0.0, "none from 1 m");
        assert!(
            (w.depth_factor(-4.5) - 0.25).abs() < 1e-6,
            "squared falloff between"
        );
        assert_eq!(w.surface_height(100.0, 100.0, 5.0), 0.0, "flat over land");
    }

    #[test]
    fn displacement_matches_the_engine_formula() {
        let w = WaveField::new(&altis_waves(), 30720.0, 0.0, 0.8, 777);
        let (x, z) = (12_000.0f32, 9_000.0f32);
        let gx = x / 50.0 - 307.2;
        let gz = z / 50.0 - 307.2;
        let u = (gx * gx + gz * gz).sqrt() * 2.0 + 777.0 / 5000.0;
        let v = gx.atan2(gz) * 79.577_47 + 0.0777;
        let s = |a: f32| (TAU * a - PI).sin();
        let u2 = u + 0.2 * s(0.25 * v) + 0.12 * s(0.375 * v);
        let expected = 0.4 * ((TAU * u2.fract() - PI).sin() + (TAU * v.fract() - PI).sin());
        assert!((w.displacement(x, z) - expected).abs() < 1e-3);
    }

    #[test]
    fn mesh_subdivisions_follow_the_scene_complexity() {
        assert_eq!(subdivisions(100_000.0, 50.0), 12, "VeryLow");
        assert_eq!(subdivisions(600_000.0, 50.0), 16, "Standard");
        assert_eq!(subdivisions(2_600_000.0, 50.0), 16, "Extreme is capped");
    }

    #[test]
    fn patch_meshes_halve_per_lod_and_overlap_their_neighbours() {
        let (v0, i0) = patch_mesh(0, 16, 50.0);
        assert_eq!(v0.len(), 129 * 129);
        assert_eq!(i0.len(), 128 * 128 * 6);
        let (v5, i5) = patch_mesh(5, 16, 50.0);
        assert_eq!(v5.len(), 5 * 5);
        assert_eq!(i5.len(), 4 * 4 * 6);
        // 400 m patch, edges 1 cm beyond.
        assert_eq!(v0[0], [-200.01, -200.01]);
        assert_eq!(v0[128], [200.01, -200.01]);
        assert_eq!(v0[129 + 1], [-200.0 + 3.125, -200.0 + 3.125]);
    }

    #[test]
    fn patch_triangles_wind_clockwise_from_above() {
        let (v, i) = patch_mesh(4, 16, 50.0);
        for t in i.chunks(3) {
            let [a, b, c] = [v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]];
            // x east, z north: clockwise seen from above has a negative cross product.
            let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            assert!(cross < 0.0, "triangle {t:?} is counter-clockwise");
        }
    }

    #[test]
    fn far_patches_coarsen_but_shores_keep_lod_three() {
        let zoom = 1.0;
        assert_eq!(
            patch_lod(50.0, zoom, 0.1, 400.0, true),
            0,
            "near the camera"
        );
        assert_eq!(patch_lod(450.0, zoom, 0.1, 400.0, true), 1);
        assert_eq!(patch_lod(1300.0, zoom, 0.1, 400.0, true), 3);
        assert_eq!(patch_lod(1700.0, zoom, 0.1, 400.0, true), 4);
        assert_eq!(patch_lod(9000.0, zoom, 0.1, 400.0, true), 5);
        assert_eq!(
            patch_lod(9000.0, zoom, 0.1, 400.0, false),
            3,
            "near a shore"
        );
        // Zooming in (a larger projection scale) keeps the finest mesh further out.
        assert_eq!(patch_lod(450.0, 5.0, 0.1, 400.0, true), 0);
    }
}
