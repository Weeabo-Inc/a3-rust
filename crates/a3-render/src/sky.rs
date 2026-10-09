//! The sky as a [`RenderFeature`]: gradient from the lighting tables, sun and moon discs,
//! stars and a cloud layer.
//!
//! The feature draws a full-screen triangle at the far plane in the opaque phase (depth test
//! `GreaterEqual` against the cleared reversed-Z depth, no depth write), so it fills exactly
//! the pixels no geometry covers, whichever order the features run in. Set
//! [`RenderSettings::procedural_sky`](crate::RenderSettings::procedural_sky) to `false` so the
//! post pass keeps it.
//!
//! Parameters live behind a shared [`SkyHandle`]: the game updates them every frame from the
//! environment (`a3-environment`), the feature uploads them in `prepare`.

use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Vec3};

use crate::feature::{Phase, PrepareContext, RenderFeature};
use crate::renderer::{Renderer, pipeline_layout, shader};
use crate::texture::{ColorSpace, GpuTexture, TextureData};
use crate::{Gpu, TextureFormat};

/// What the sky shows. Colours are linear HDR in the scene's units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyParams {
    /// Sky colour straight up (RV `sky`).
    pub zenith: Vec3,
    /// Sky colour at the horizon (RV `fogColor`).
    pub horizon: Vec3,
    /// Sky colour around the sun (RV `skyAroundSun`).
    pub around_sun: Vec3,
    /// Colour below the horizon (seen only where no terrain or sea is drawn).
    pub ground: Vec3,
    /// Unit vector towards the sun.
    pub sun_direction: Vec3,
    /// Sun light colour (RV `diffuse`).
    pub sun_color: Vec3,
    /// Disc radiance as a multiple of `sun_color`.
    pub sun_disc_scale: f32,
    /// Angular radius of the sun disc in radians.
    pub sun_radius: f32,
    /// Unit vector towards the moon.
    pub moon_direction: Vec3,
    /// Full-moon disc colour (RV `moonObjectColorFull`); the phase shading comes from the sun
    /// direction.
    pub moon_color: Vec3,
    /// Angular radius of the moon disc in radians.
    pub moon_radius: f32,
    /// Star brightness (0 by day).
    pub stars: f32,
    /// Rotation from equatorial (x: RA 0h, z: north pole) to world axes, for the stars.
    pub star_rotation: Mat3,
    /// Cloud colour lit by the sky (RV `cloudsColor`).
    pub cloud_color: Vec3,
    /// Fraction of the sky covered by clouds, 0..1.
    pub cloud_cover: f32,
    /// Opacity of the clouds, 0..1.
    pub cloud_opacity: f32,
    /// Cloud base height in metres.
    pub cloud_height: f32,
    /// Size of a cloud noise period in metres.
    pub cloud_scale: f32,
    /// Cloud drift in metres per second (x east, y north).
    pub wind: glam::Vec2,
    /// Seconds, for cloud drift and star twinkle.
    pub time: f32,
}

impl Default for SkyParams {
    fn default() -> Self {
        SkyParams {
            zenith: Vec3::new(0.12, 0.28, 0.65),
            horizon: Vec3::new(0.62, 0.72, 0.85),
            around_sun: Vec3::new(1.0, 0.95, 0.85),
            ground: Vec3::new(0.2, 0.2, 0.2),
            sun_direction: Vec3::new(0.45, 0.6, -0.65).normalize(),
            sun_color: Vec3::new(1.0, 0.95, 0.85),
            sun_disc_scale: 50.0,
            sun_radius: 0.27f32.to_radians(),
            moon_direction: Vec3::new(-0.45, -0.6, 0.65).normalize(),
            moon_color: Vec3::splat(0.5),
            moon_radius: 0.26f32.to_radians(),
            stars: 0.0,
            star_rotation: Mat3::IDENTITY,
            cloud_color: Vec3::ONE,
            cloud_cover: 0.0,
            cloud_opacity: 0.0,
            cloud_height: 2_850.0,
            cloud_scale: 12_000.0,
            wind: glam::Vec2::new(4.0, 1.0),
            time: 0.0,
        }
    }
}

/// Shared access to a [`SkyFeature`]'s parameters.
pub type SkyHandle = Arc<Mutex<SkyParams>>;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct SkyUniforms {
    zenith: [f32; 4],
    horizon: [f32; 4],
    around_sun: [f32; 4],
    ground: [f32; 4],
    // xyz: towards the sun, w: cos of the disc radius.
    sun: [f32; 4],
    // rgb: sun colour, w: disc scale.
    sun_color: [f32; 4],
    // xyz: towards the moon, w: sin of the disc radius.
    moon: [f32; 4],
    moon_color: [f32; 4],
    // rgb: cloud colour, w: cover.
    clouds: [f32; 4],
    // x: opacity, y: height, z: 1 / scale, w: time.
    cloud_params: [f32; 4],
    // xy: wind, z: stars, w: unused.
    misc: [f32; 4],
    star_rotation: [[f32; 4]; 3],
}

impl SkyUniforms {
    fn new(p: &SkyParams) -> Self {
        let r = p.star_rotation;
        SkyUniforms {
            zenith: p.zenith.extend(0.0).to_array(),
            horizon: p.horizon.extend(0.0).to_array(),
            around_sun: p.around_sun.extend(0.0).to_array(),
            ground: p.ground.extend(0.0).to_array(),
            sun: p
                .sun_direction
                .normalize_or_zero()
                .extend(p.sun_radius.cos())
                .to_array(),
            sun_color: p.sun_color.extend(p.sun_disc_scale).to_array(),
            moon: p
                .moon_direction
                .normalize_or_zero()
                .extend(p.moon_radius.sin())
                .to_array(),
            moon_color: p.moon_color.extend(0.0).to_array(),
            clouds: p.cloud_color.extend(p.cloud_cover).to_array(),
            cloud_params: [
                p.cloud_opacity,
                p.cloud_height,
                1.0 / p.cloud_scale.max(1.0),
                p.time,
            ],
            misc: [p.wind.x, p.wind.y, p.stars, 0.0],
            star_rotation: [
                r.x_axis.extend(0.0).to_array(),
                r.y_axis.extend(0.0).to_array(),
                r.z_axis.extend(0.0).to_array(),
            ],
        }
    }
}

/// Draws the sky. See the module documentation.
pub struct SkyFeature {
    params: SkyHandle,
    _noise: GpuTexture,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl SkyFeature {
    /// Creates the feature for `renderer` (its frame layout is group 0). `noise` is the
    /// tileable cloud noise (RV `SimulWeather >> noiseTexture`, three octaves in RGB); a
    /// built-in noise is used when `None`.
    pub fn new(gpu: &Gpu, renderer: &Renderer, noise: Option<&TextureData>) -> (Self, SkyHandle) {
        let device = &gpu.device;
        let fallback = builtin_noise();
        let noise = GpuTexture::upload(
            device,
            &gpu.queue,
            noise.unwrap_or(&fallback),
            ColorSpace::Linear,
            Some("sky noise"),
        )
        .or_else(|_| {
            GpuTexture::upload(
                device,
                &gpu.queue,
                &fallback,
                ColorSpace::Linear,
                Some("sky noise"),
            )
        })
        .expect("built-in noise uploads");
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky uniforms"),
            size: std::mem::size_of::<SkyUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky noise"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&noise.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let module = shader(device, "sky", include_str!("../shaders/sky.wgsl"));
        let pipeline_layout = pipeline_layout(device, "sky", &[renderer.frame_layout(), &layout]);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: Renderer::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: Renderer::SCENE_COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let params: SkyHandle = Arc::new(Mutex::new(SkyParams::default()));
        (
            SkyFeature {
                params: params.clone(),
                _noise: noise,
                uniforms,
                bind_group,
                pipeline,
            },
            params,
        )
    }
}

impl RenderFeature for SkyFeature {
    fn prepare(&mut self, cx: &PrepareContext<'_>) {
        let params = *self
            .params
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cx.queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&SkyUniforms::new(&params)),
        );
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>) {
        if phase != Phase::Opaque {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// A small tileable value noise, three octaves in RGB, for when no noise texture is given.
fn builtin_noise() -> TextureData {
    const N: u32 = 64;
    let hash = |x: u32, y: u32, seed: u32| {
        let mut h = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263) ^ seed;
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        ((h ^ (h >> 16)) & 0xff) as f32 / 255.0
    };
    let octave = |x: u32, y: u32, period: u32, seed: u32| {
        let cell = N / period;
        let (fx, fy) = (
            (x % cell) as f32 / cell as f32,
            (y % cell) as f32 / cell as f32,
        );
        let (cx, cy) = (x / cell, y / cell);
        let v = |i: u32, j: u32| hash((cx + i) % period, (cy + j) % period, seed);
        let s = |t: f32| t * t * (3.0 - 2.0 * t);
        let (sx, sy) = (s(fx), s(fy));
        let top = v(0, 0) + (v(1, 0) - v(0, 0)) * sx;
        let bottom = v(0, 1) + (v(1, 1) - v(0, 1)) * sx;
        top + (bottom - top) * sy
    };
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            for (period, seed) in [(4, 1), (8, 2), (16, 3)] {
                rgba.push((octave(x, y, period, seed) * 255.0) as u8);
            }
            rgba.push(255);
        }
    }
    TextureData {
        format: TextureFormat::Rgba8,
        width: N,
        height: N,
        mips: vec![rgba],
    }
}
