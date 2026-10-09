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
    /// The sky dome's elevation ramp, for elevations `0 .. 90` degrees in
    /// [`DOME_RAMP_STEPS`] even steps. See [`dome_ramp`]. All ones without a `skyTexture`.
    pub dome_ramp: [Vec3; DOME_RAMP_STEPS],
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
            dome_ramp: [Vec3::ONE; DOME_RAMP_STEPS],
        }
    }
}

/// Steps of [`SkyParams::dome_ramp`]: one per `90 / (DOME_RAMP_STEPS - 1)` degrees of
/// elevation. The shipped ramp is smooth, so the shader's linear interpolation between them is
/// exact enough (the largest step is about 2 % of the ramp's range).
pub const DOME_RAMP_STEPS: usize = 32;

/// Elevations of the shipped sky dome's rings in degrees, with the `v` coordinate of its UV at
/// each: `A3\Map_Stratis\data\obloha.p3d`, one resolution LOD, 335 vertices. Below the first
/// ring the dome's skirt keeps `v = 0`, so every elevation from the horizon to 19.6 degrees
/// has the ramp's horizon value (`docs/re/render-atmosphere.md` §4).
const DOME_RINGS: [(f32, f32); 11] = [
    (19.64, 0.000),
    (22.74, 0.156),
    (26.28, 0.309),
    (30.39, 0.454),
    (35.23, 0.588),
    (41.01, 0.707),
    (47.97, 0.809),
    (56.35, 0.891),
    (66.30, 0.951),
    (77.69, 0.988),
    (89.95, 1.000),
];

/// The dome's `v` for an elevation in degrees, linearly between its rings.
fn dome_v(elevation: f32) -> f32 {
    let (first_e, first_v) = DOME_RINGS[0];
    if elevation <= first_e {
        return first_v;
    }
    for pair in DOME_RINGS.windows(2) {
        let ((e0, v0), (e1, v1)) = (pair[0], pair[1]);
        if elevation <= e1 {
            return v0 + (v1 - v0) * (elevation - e0) / (e1 - e0);
        }
    }
    1.0
}

/// The sky dome's elevation ramp from a world's `skyTexture` (`CfgWorlds >> skyTexture`; the
/// shipped ones are 8x8 `Sky` textures), normalised to 1 at the horizon.
///
/// The engine's dome shader shades `tint * texture` (`docs/re/render-atmosphere.md` §4), so the
/// texture rides along as a *relative* ramp on top of the sky's colour: the horizon is
/// unchanged and the zenith keeps the ramp's own ratio and hue. `Sky` textures store the zenith
/// at `v = 1` (the first row of the mip) and the horizon at `v = 0` (the last). `None` when the
/// texture has no pixels; the caller then keeps [`SkyParams::dome_ramp`] at one.
///
/// The pixels are read as stored: a `Sky` texture is linear, and the values are only ever used
/// as a ratio to the horizon's.
pub fn dome_ramp(texture: &TextureData) -> Option<[Vec3; DOME_RAMP_STEPS]> {
    let (width, height, pixels) = (
        texture.width as usize,
        texture.height as usize,
        texture.mips.first()?,
    );
    if width == 0 || height == 0 || pixels.len() < width * height * 4 {
        return None;
    }
    // v = 0 is the horizon, the last row of the mip.
    let texel = |v: f32| -> Vec3 {
        let v = v.clamp(0.0, 1.0);
        let y = (1.0 - v) * (height - 1) as f32;
        let (y0, t) = (y.floor() as usize, y.fract());
        let y0 = y0.min(height - 1);
        let y1 = (y0 + 1).min(height - 1);
        let row = |y: usize| {
            let at = (y * width) * 4;
            Vec3::new(
                f32::from(pixels[at]) / 255.0,
                f32::from(pixels[at + 1]) / 255.0,
                f32::from(pixels[at + 2]) / 255.0,
            )
        };
        row(y0) + (row(y1) - row(y0)) * t
    };
    let horizon = texel(0.0);
    // A ramp divides by the horizon, so a black horizon has no usable ramp.
    let scale = Vec3::new(
        if horizon.x > 1e-6 {
            1.0 / horizon.x
        } else {
            1.0
        },
        if horizon.y > 1e-6 {
            1.0 / horizon.y
        } else {
            1.0
        },
        if horizon.z > 1e-6 {
            1.0 / horizon.z
        } else {
            1.0
        },
    );
    let last = (DOME_RAMP_STEPS - 1) as f32;
    let mut ramp = [Vec3::ONE; DOME_RAMP_STEPS];
    for (i, r) in ramp.iter_mut().enumerate() {
        let elevation = 90.0 * i as f32 / last;
        *r = texel(dome_v(elevation)) * scale;
    }
    Some(ramp)
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
    // The dome's ramp over elevation, one entry per `90 / (DOME_RAMP_STEPS - 1)` degrees.
    dome_ramp: [[f32; 4]; DOME_RAMP_STEPS],
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
            dome_ramp: p.dome_ramp.map(|c| c.extend(0.0).to_array()),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An 8x8 ramp texture with one row per entry, `rows` given from the zenith (`v = 1`) to the
    /// horizon (`v = 0`) as the shipped `Sky` textures are stored.
    fn ramp_texture(rows: [[u8; 3]; 8]) -> TextureData {
        let pixels: Vec<u8> = rows
            .iter()
            .flat_map(|row| (0..8).flat_map(move |_| [row[0], row[1], row[2], 255]))
            .collect();
        TextureData {
            format: TextureFormat::Rgba8,
            width: 8,
            height: 8,
            mips: vec![pixels],
        }
    }

    /// The shipped `A3\Map_Stratis\data\sky_semicloudy_sky.paa`, first mip: the zenith row
    /// first, the horizon row last.
    fn shipped_ramp() -> [[u8; 3]; 8] {
        [
            [13, 22, 60],
            [13, 27, 60],
            [18, 32, 71],
            [24, 39, 82],
            [33, 48, 99],
            [41, 63, 110],
            [49, 72, 121],
            [57, 81, 132],
        ]
    }

    #[test]
    fn dome_v_follows_the_dome_rings() {
        // The skirt below 19.64 degrees keeps v = 0, the rings interpolate, the zenith is 1.
        assert_eq!(dome_v(-30.0), 0.0);
        assert_eq!(dome_v(0.0), 0.0);
        assert_eq!(dome_v(19.64), 0.0);
        assert!((dome_v(89.95) - 1.0).abs() < 1e-6);
        assert!(dome_v(90.0) >= 0.999);
        // Halfway between two rings.
        let mid = 0.5 * (22.74 + 26.28);
        let want = 0.5 * (0.156 + 0.309);
        assert!(
            (dome_v(mid) - want).abs() < 1e-4,
            "{} vs {}",
            dome_v(mid),
            want
        );
        // Monotonic in elevation.
        let mut last = -1.0;
        for i in 0..=90 {
            let v = dome_v(i as f32);
            assert!(v >= last, "dome_v({i}) = {v} < {last}");
            last = v;
        }
    }

    #[test]
    fn dome_ramp_is_normalised_at_the_horizon_and_dimmer_at_the_zenith() {
        let texture = ramp_texture(shipped_ramp());
        let ramp = dome_ramp(&texture).expect("ramp");
        // The horizon row is the texture's last: the ramp starts at one everywhere below the
        // dome's first ring and never exceeds one above it.
        let horizon_row = Vec3::new(57.0, 81.0, 132.0) / 255.0;
        for (i, r) in ramp.iter().enumerate() {
            let elevation = 90.0 * i as f32 / (DOME_RAMP_STEPS - 1) as f32;
            if elevation <= DOME_RINGS[0].0 {
                assert!(
                    (r - Vec3::ONE).length() < 1e-5,
                    "step {i} ({elevation} deg, below the first ring) = {r:?}"
                );
            } else {
                assert!(
                    r.cmple(Vec3::ONE).all() && *r != Vec3::ONE,
                    "step {i} ({elevation} deg) = {r:?} is not dimmer than the horizon"
                );
            }
        }
        // The zenith is the texture's first row over the last: a deeper, dimmer blue.
        let zenith = ramp[DOME_RAMP_STEPS - 1];
        let want = (Vec3::new(13.0, 22.0, 60.0) / 255.0) / horizon_row;
        assert!((zenith - want).length() < 1e-3, "{zenith:?} vs {want:?}");
        // (13/57, 22/81, 60/132): dimmer than the horizon and bluer than it (blue/red 2.0
        // against the horizon's 1.0).
        assert!(
            (zenith.z / zenith.x - 2.0).abs() < 0.2,
            "{zenith:?} is not bluer"
        );
        assert!(zenith.z < 0.6, "{zenith:?} is not dimmer");
    }

    #[test]
    fn dome_ramp_without_a_texture_leaves_the_sky_alone() {
        // An empty texture has no ramp; the caller keeps SkyParams::default().
        let empty = TextureData {
            format: TextureFormat::Rgba8,
            width: 0,
            height: 0,
            mips: Vec::new(),
        };
        assert!(dome_ramp(&empty).is_none());
        assert_eq!(SkyParams::default().dome_ramp, [Vec3::ONE; DOME_RAMP_STEPS]);
        // A black horizon has no usable ratio either: the ramp must stay finite.
        let black = ramp_texture([[0, 0, 0]; 8]);
        let ramp = dome_ramp(&black).expect("ramp");
        assert!(ramp.iter().all(|r| r.is_finite()));
    }
}
