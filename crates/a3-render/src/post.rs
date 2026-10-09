//! The post chain: atmosphere into HDR, eye adaptation, bloom, tonemapping and anti-aliasing.
//!
//! Follows RV's HDR chain as reverse engineered in `docs/re/render-atmosphere.md` §3: the
//! lighting entry's aperture stage (`exposure = 1 / aperture²`, the measured-luminance curve of
//! [`HdrSettings::aperture`]), the log-average luminance meter, assumed-luminance adaptation
//! between the aperture's limits, bloom mixed before the curve, `tonemapMethod` curves and a
//! final gamma. Defaults mirror `CfgWorlds >> Altis >> HDRNewPars` and `Lighting0`.

use bytemuck::{Pod, Zeroable};

/// Tonemapping curve, numbered as RV's `HDRNewPars >> tonemapMethod` (see `docs/re/hdr.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tonemap {
    /// Method 0: exposure only, clamped to the display range.
    None,
    /// Method 1: Hable's filmic curve with [`FilmicCurve`] parameters.
    Filmic,
    /// Method 2: extended Reinhard with [`HdrSettings::reinhard_white`] as white point.
    Reinhard,
}

impl Tonemap {
    /// The curve for RV's `tonemapMethod` value; unknown values fall back to filmic.
    pub fn from_rv_method(method: i32) -> Tonemap {
        match method {
            0 => Tonemap::None,
            2 => Tonemap::Reinhard,
            _ => Tonemap::Filmic,
        }
    }

    /// RV's `tonemapMethod` value, also the selector in `tonemap.wgsl`.
    pub fn rv_method(self) -> i32 {
        match self {
            Tonemap::None => 0,
            Tonemap::Filmic => 1,
            Tonemap::Reinhard => 2,
        }
    }

    fn shader_index(self) -> f32 {
        self.rv_method() as f32
    }
}

/// Parameters of Hable's filmic curve, named after RV's `tonemap*` config entries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilmicCurve {
    /// `tonemapShoulderStrength` (A).
    pub shoulder_strength: f32,
    /// `tonemapLinearStrength` (B).
    pub linear_strength: f32,
    /// `tonemapLinearAngle` (C).
    pub linear_angle: f32,
    /// `tonemapToeStrength` (D).
    pub toe_strength: f32,
    /// `tonemapToeNumerator` (E).
    pub toe_numerator: f32,
    /// `tonemapToeDenominator` (F).
    pub toe_denominator: f32,
    /// `tonemapLinearWhite` (W).
    pub linear_white: f32,
}

impl Default for FilmicCurve {
    /// Altis / CAWorld values.
    fn default() -> Self {
        FilmicCurve {
            shoulder_strength: 0.22,
            linear_strength: 0.12,
            linear_angle: 0.1,
            toe_strength: 0.2,
            toe_numerator: 0.022,
            toe_denominator: 0.2,
            linear_white: 11.2,
        }
    }
}

/// Post-process anti-aliasing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntiAliasing {
    None,
    /// FXAA-style edge blur on luma.
    Fxaa,
}

/// Bloom parameters, named after RV's `HDRNewPars` entries (`PSC_BloomPars`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomSettings {
    pub enabled: bool,
    /// `bloomImageScale`: scale of the scene image in the final mix.
    pub image_scale: f32,
    /// `bloomScale`: strength of the bloom on dark pixels (it fades out on bright ones).
    pub scale: f32,
    /// `bloomExponent`: power applied to the blurred image.
    pub exponent: f32,
}

impl Default for BloomSettings {
    /// Altis / CAWorld values.
    fn default() -> Self {
        BloomSettings {
            enabled: true,
            image_scale: 1.0,
            scale: 0.09,
            exponent: 0.75,
        }
    }
}

/// HDR, eye adaptation, tonemapping and anti-aliasing settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HdrSettings {
    pub tonemap: Tonemap,
    pub filmic: FilmicCurve,
    /// `tonemapExposureBias`; applies to the filmic curve only, as in RV.
    pub exposure_bias: f32,
    /// `tonemapLinearWhiteReinhard`: white point of the Reinhard curve.
    pub reinhard_white: f32,
    pub bloom: BloomSettings,
    /// `HDRNewPars >> minAperture` / `maxAperture`: the global clamp of the aperture value
    /// (not of the exposure; exposure is `1 / aperture²`).
    pub min_aperture: f32,
    pub max_aperture: f32,
    /// `LightingN >> apertureMin`: the aperture a scene at the dark end of the range
    /// (`standardAvgLum / apertureRatioMin`) is exposed with.
    pub aperture_min: f32,
    /// `LightingN >> apertureStandard`: the aperture at `standardAvgLum`. **0 means no aperture
    /// stage**: without a lighting entry (world-less renders, synthetic scenes) the exposure
    /// falls back to the assumed-luminance step `key / measured`.
    pub aperture_standard: f32,
    /// `LightingN >> apertureMax`: the aperture at `standardAvgLum · apertureRatioMax`.
    pub aperture_max: f32,
    /// `LightingN >> standardAvgLum`: the measured scene luminance the lighting entry is
    /// calibrated for, in the meter's units.
    pub standard_avg_lum: f32,
    /// `HDRNewPars >> apertureRatioMin` / `apertureRatioMax`: how far below / above
    /// `standard_avg_lum` the measured luminance must be for the aperture to reach
    /// `aperture_min` / `aperture_max`.
    pub aperture_ratio_min: f32,
    pub aperture_ratio_max: f32,
    /// `eyeAdaptFactorLight`: speed of RV's CPU exposure stage towards a brighter scene
    /// (`docs/re/render-atmosphere.md` §3.3). The stage's curve is [`Self::aperture_exposure`];
    /// its interpolation runs on the GPU as part of the assumed-luminance pass.
    pub eye_adapt_light: f32,
    /// `eyeAdaptFactorDark`: the same towards a darker scene.
    pub eye_adapt_dark: f32,
    /// Assumed-luminance target of the fallback stage (`PSC_AssumedLuminancePars1.z`): the
    /// exposure is `key / measured` when no lighting entry supplies an aperture stage
    /// ([`aperture_standard`](Self::aperture_standard) `<= 0`). RV takes it from
    /// `engine+0x368 · brightness · 0.5`, whose first factor is not identified; ours is chosen by
    /// eye (the client's steady-state exposure does not depend on it, see `docs/re/hdr.md`).
    pub key: f32,
    /// The exposure RV's CPU stage currently holds (`state+0x20`, `1 / aperture²` after its
    /// interpolation). It sets how fast the GPU stage may adapt; 1 until the CPU stage's
    /// per-frame state is read back.
    pub cpu_exposure: f32,
    /// Final power applied after the curve: `PSC_RgbEyeCoef.w`, which RV always sets to 1. The
    /// sRGB encode for display follows separately.
    pub final_gamma: f32,
    /// Fixed exposure instead of eye adaptation (`setAperture`-like override, and for tests).
    pub fixed_exposure: Option<f32>,
    pub anti_aliasing: AntiAliasing,
}

impl Default for HdrSettings {
    fn default() -> Self {
        HdrSettings {
            tonemap: Tonemap::Filmic,
            filmic: FilmicCurve::default(),
            exposure_bias: 1.0,
            reinhard_white: 2.5,
            bloom: BloomSettings::default(),
            min_aperture: 1e-5,
            max_aperture: 256.0,
            // No lighting entry sampled yet: the assumed-luminance step is used. A World's
            // `LightingN` entry overwrites all five per frame in
            // `SceneEnvironment::apply` (docs/re/render-atmosphere.md §1.3).
            aperture_min: 4.0,
            aperture_standard: 0.0,
            aperture_max: 8.0,
            standard_avg_lum: 4.0,
            aperture_ratio_min: 10.0,
            aperture_ratio_max: 4.0,
            eye_adapt_light: 3.3,
            eye_adapt_dark: 0.75,
            key: 0.3,
            cpu_exposure: 1.0,
            final_gamma: 1.0,
            fixed_exposure: None,
            anti_aliasing: AntiAliasing::Fxaa,
        }
    }
}

impl HdrSettings {
    /// `true` when a lighting entry supplied an aperture stage.
    pub fn has_aperture_stage(&self) -> bool {
        self.aperture_standard > 0.0 && self.standard_avg_lum > 0.0
    }

    /// RV's CPU aperture stage (`0x14175ac10`, `docs/re/render-atmosphere.md` §3.3): the aperture
    /// the lighting entry asks for at a measured scene luminance, in the entry's own units.
    ///
    /// At `standard_avg_lum` the aperture is `aperture_standard`; it moves linearly towards
    /// `aperture_min` / `aperture_max` as the luminance falls / rises, reaching them at
    /// `standard_avg_lum / aperture_ratio_min` and `standard_avg_lum · aperture_ratio_max`.
    /// The dark branch is the engine's, including its discontinuity at `standard_avg_lum` when
    /// `aperture_standard != aperture_min`. The result is clamped to
    /// `[min_aperture, max_aperture]`.
    pub fn aperture(&self, luminance: f32) -> f32 {
        let (pmin, pstd, pmax) = (self.aperture_min, self.aperture_standard, self.aperture_max);
        let lstd = self.standard_avg_lum;
        if pstd <= 0.0 || lstd <= 0.0 {
            return 1.0;
        }
        let ap = if luminance <= lstd {
            let x = -lstd / luminance.max(1e-6);
            let rmin = self.aperture_ratio_min;
            if rmin <= 1.0 || x < -rmin {
                pmin
            } else {
                pmin + (x + rmin) / (1.0 + rmin) * (pstd - pmin)
            }
        } else {
            let y = luminance / lstd;
            let rmax = self.aperture_ratio_max;
            if rmax <= 1.0 || y > rmax {
                pmax
            } else {
                pstd + (y - 1.0) / (rmax - 1.0) * (pmax - pstd)
            }
        };
        ap.clamp(self.min_aperture, self.max_aperture.max(self.min_aperture))
    }

    /// The exposure RV's aperture stage produces: `1 / aperture²`.
    pub fn aperture_exposure(&self, luminance: f32) -> f32 {
        let ap = self.aperture(luminance).max(1e-12);
        1.0 / (ap * ap)
    }

    /// The exposure range the adaptation clamps to: the aperture stage at the two ends of the
    /// entry's luminance range, i.e. `1 / apertureMax² ..= 1 / apertureMin²`. Without a lighting
    /// entry it is the fallback stage's own bounds, `key / maxAperture ..= key / minAperture`.
    pub fn exposure_range(&self) -> (f32, f32) {
        if !self.has_aperture_stage() {
            let max_ap = self.max_aperture.max(self.min_aperture).max(1e-12);
            let min_ap = self.min_aperture.max(1e-12);
            return (self.key / max_ap, self.key / min_ap);
        }
        let lstd = self.standard_avg_lum;
        let lo = self.aperture_exposure(lstd * self.aperture_ratio_max.max(1.0));
        let hi = self.aperture_exposure(lstd / self.aperture_ratio_min.max(1.0));
        (lo.min(hi), hi.max(lo))
    }

    /// The target of the assumed-luminance stage: `key` without a lighting entry, and
    /// `standardAvgLum / apertureStandard²` with one — where the aperture stage and the
    /// assumed-luminance step agree exactly.
    pub fn assumed_key(&self) -> f32 {
        if !self.has_aperture_stage() {
            return self.key.max(0.0);
        }
        let pstd = self.aperture_standard.max(self.min_aperture).max(1e-6);
        (self.standard_avg_lum / (pstd * pstd)).max(0.0)
    }

    /// Per-frame limits of the exposure ratio (RV's `PSC_AssumedLuminancePars2.zw`,
    /// `0x14173fda0`): the exposure may halve every `τ_down` and double every `τ_up` seconds,
    /// 0.5 s and 1 s at a CPU exposure up to 1, slowing linearly to 1 s and 20 s at 4 and above;
    /// at least 0.1 % per frame either way.
    pub fn adaptation_limits(&self, dt: f32) -> (f32, f32) {
        let dt = dt.max(0.0);
        let f = ((self.cpu_exposure - 1.0) / 3.0).clamp(0.0, 1.0);
        let tau_down = 0.5 * (1.0 - f) + f;
        let tau_up = (1.0 - f) + 20.0 * f;
        (
            (-dt / tau_down).exp2().min(0.999),
            (dt / tau_up).exp2().max(1.001),
        )
    }

    fn uniforms(&self, dt: f32, reset: bool, output_srgb: bool) -> PostUniforms {
        let f = &self.filmic;
        let (min_exposure, max_exposure) = self.exposure_range();
        let (ratio_min, ratio_max) = self.adaptation_limits(dt);
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        PostUniforms {
            filmic_abcd: [
                f.shoulder_strength,
                f.linear_strength,
                f.linear_angle,
                f.toe_strength,
            ],
            filmic_efw_bias: [
                f.toe_numerator,
                f.toe_denominator,
                f.linear_white,
                self.exposure_bias,
            ],
            exposure: [
                min_exposure,
                max_exposure,
                self.assumed_key(),
                self.fixed_exposure.unwrap_or(0.0),
            ],
            adaptation: [ratio_min, ratio_max, dt.max(0.0), flag(reset)],
            misc: [
                self.tonemap.shader_index(),
                self.reinhard_white,
                flag(output_srgb),
                flag(self.anti_aliasing == AntiAliasing::Fxaa),
            ],
            bloom: [
                self.bloom.image_scale,
                self.bloom.scale,
                1.0,
                self.bloom.exponent,
            ],
            output: [self.final_gamma, flag(self.bloom.enabled), 0.0, 0.0],
            aperture: [
                self.aperture_min,
                self.aperture_standard,
                self.aperture_max,
                self.standard_avg_lum,
            ],
            aperture_ratios: [
                self.aperture_ratio_min,
                self.aperture_ratio_max,
                self.min_aperture,
                self.max_aperture,
            ],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct PostUniforms {
    filmic_abcd: [f32; 4],
    filmic_efw_bias: [f32; 4],
    exposure: [f32; 4],
    adaptation: [f32; 4],
    misc: [f32; 4],
    bloom: [f32; 4],
    output: [f32; 4],
    /// `apertureMin, apertureStandard, apertureMax, standardAvgLum` of the lighting entry.
    aperture: [f32; 4],
    /// `apertureRatioMin, apertureRatioMax, minAperture, maxAperture`.
    aperture_ratios: [f32; 4],
}

/// Format of the intermediate HDR image after the atmosphere pass, and of the bloom targets.
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Gamma-encoded tonemapped image with luma in alpha.
const LDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Pixels per luminance-meter workgroup edge (16 invocations x 2 taps).
const METER_TILE: u32 = 32;

struct PostTargets {
    meter_groups: (u32, u32),
    atmosphere_inputs: wgpu::BindGroup,
    hdr_view: wgpu::TextureView,
    exposure_group: wgpu::BindGroup,
    bloom_a: wgpu::TextureView,
    bloom_b: wgpu::TextureView,
    bloom_down_group: wgpu::BindGroup,
    blur_h_group: wgpu::BindGroup,
    blur_v_group: wgpu::BindGroup,
    tonemap_group: wgpu::BindGroup,
    ldr_view: wgpu::TextureView,
    final_group: wgpu::BindGroup,
}

/// GPU side of the post chain.
pub(crate) struct PostChain {
    output_srgb: bool,
    uniforms: wgpu::Buffer,
    state: wgpu::Buffer,
    sampler: wgpu::Sampler,
    atmosphere_layout: wgpu::BindGroupLayout,
    atmosphere: wgpu::RenderPipeline,
    exposure_layout: wgpu::BindGroupLayout,
    measure: wgpu::ComputePipeline,
    adapt: wgpu::ComputePipeline,
    bloom_layout: wgpu::BindGroupLayout,
    bloom_down: wgpu::RenderPipeline,
    blur_h: wgpu::RenderPipeline,
    blur_v: wgpu::RenderPipeline,
    tonemap_layout: wgpu::BindGroupLayout,
    tonemap: wgpu::RenderPipeline,
    final_layout: wgpu::BindGroupLayout,
    fxaa: wgpu::RenderPipeline,
    copy: wgpu::RenderPipeline,
    targets: Option<PostTargets>,
    reset_adaptation: bool,
}

fn float_tex(
    binding: u32,
    visibility: wgpu::ShaderStages,
    filterable: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn uniform_entry(visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    read_only: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
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

impl PostChain {
    pub fn new(
        device: &wgpu::Device,
        frame_layout: &wgpu::BindGroupLayout,
        output_format: wgpu::TextureFormat,
    ) -> PostChain {
        use wgpu::ShaderStages as S;
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post uniforms"),
            size: std::mem::size_of::<PostUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let state = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exposure state"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post linear clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };

        let atmosphere_layout = layout(
            "atmosphere inputs",
            &[
                float_tex(0, S::FRAGMENT, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: S::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        );
        let exposure_layout = layout(
            "exposure",
            &[
                uniform_entry(S::COMPUTE),
                float_tex(1, S::COMPUTE, false),
                storage_entry(2, S::COMPUTE, false),
                storage_entry(3, S::COMPUTE, false),
            ],
        );
        let bloom_layout = layout(
            "bloom",
            &[
                uniform_entry(S::FRAGMENT),
                float_tex(1, S::FRAGMENT, false),
                storage_entry(2, S::FRAGMENT, true),
            ],
        );
        let tonemap_layout = layout(
            "tonemap",
            &[
                uniform_entry(S::FRAGMENT),
                float_tex(1, S::FRAGMENT, false),
                storage_entry(2, S::FRAGMENT, true),
                float_tex(3, S::FRAGMENT, true),
                sampler_entry(4),
            ],
        );
        let final_layout = layout(
            "final",
            &[
                uniform_entry(S::FRAGMENT),
                float_tex(1, S::FRAGMENT, true),
                sampler_entry(2),
            ],
        );

        let shader = |label, source| crate::renderer::shader(device, label, source);
        let atmosphere_shader = shader("atmosphere", include_str!("../shaders/atmosphere.wgsl"));
        let atmosphere = fullscreen_pipeline(
            device,
            "atmosphere",
            &[frame_layout, &atmosphere_layout],
            &atmosphere_shader,
            "fs_main",
            HDR_FORMAT,
        );

        let exposure_shader = shader("exposure", include_str!("../shaders/exposure.wgsl"));
        let exposure_pipeline_layout =
            crate::renderer::pipeline_layout(device, "exposure", &[&exposure_layout]);
        let compute = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&exposure_pipeline_layout),
                module: &exposure_shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let measure = compute("measure");
        let adapt = compute("adapt");

        let bloom_shader = shader("bloom", include_str!("../shaders/bloom.wgsl"));
        let bloom_pipeline = |entry| {
            fullscreen_pipeline(
                device,
                entry,
                &[&bloom_layout],
                &bloom_shader,
                entry,
                HDR_FORMAT,
            )
        };
        let bloom_down = bloom_pipeline("fs_down");
        let blur_h = bloom_pipeline("fs_blur_h");
        let blur_v = bloom_pipeline("fs_blur_v");

        let tonemap_shader = shader("tonemap", include_str!("../shaders/tonemap.wgsl"));
        let tonemap = fullscreen_pipeline(
            device,
            "tonemap",
            &[&tonemap_layout],
            &tonemap_shader,
            "fs_main",
            LDR_FORMAT,
        );
        let final_shader = shader("final", include_str!("../shaders/final.wgsl"));
        let fxaa = fullscreen_pipeline(
            device,
            "fxaa",
            &[&final_layout],
            &final_shader,
            "fs_fxaa",
            output_format,
        );
        let copy = fullscreen_pipeline(
            device,
            "final copy",
            &[&final_layout],
            &final_shader,
            "fs_copy",
            output_format,
        );

        PostChain {
            output_srgb: output_format.is_srgb(),
            uniforms,
            state,
            sampler,
            atmosphere_layout,
            atmosphere,
            exposure_layout,
            measure,
            adapt,
            bloom_layout,
            bloom_down,
            blur_h,
            blur_v,
            tonemap_layout,
            tonemap,
            final_layout,
            fxaa,
            copy,
            targets: None,
            reset_adaptation: true,
        }
    }

    /// Jump straight to the target exposure on the next frame (e.g. after a camera cut).
    pub fn reset_adaptation(&mut self) {
        self.reset_adaptation = true;
    }

    /// The buffer holding `{adapted luminance, exposure, average luminance, _}` as `f32`s.
    pub fn exposure_state(&self) -> &wgpu::Buffer {
        &self.state
    }

    /// (Re)create size-dependent targets; `scene_color` / `scene_depth` are the scene pass
    /// outputs.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        size: (u32, u32),
        scene_color: &wgpu::TextureView,
        scene_depth: &wgpu::TextureView,
    ) {
        let texture = |label, (width, height): (u32, u32), format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
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
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let bloom_size = ((size.0 / 4).max(1), (size.1 / 4).max(1));
        let meter_groups = (size.0.div_ceil(METER_TILE), size.1.div_ceil(METER_TILE));
        let hdr_view = texture("post hdr", size, HDR_FORMAT);
        let ldr_view = texture("post ldr", size, LDR_FORMAT);
        let bloom_a = texture("bloom a", bloom_size, HDR_FORMAT);
        let bloom_b = texture("bloom b", bloom_size, HDR_FORMAT);
        let partials = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("luminance partials"),
            size: u64::from(meter_groups.0 * meter_groups.1) * 2 * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let view = wgpu::BindingResource::TextureView;
        let group = |label, layout, entries: &[wgpu::BindGroupEntry<'_>]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries,
            })
        };
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        let atmosphere_inputs = group(
            "atmosphere inputs",
            &self.atmosphere_layout,
            &[entry(0, view(scene_color)), entry(1, view(scene_depth))],
        );
        let exposure_group = group(
            "exposure",
            &self.exposure_layout,
            &[
                entry(0, self.uniforms.as_entire_binding()),
                entry(1, view(&hdr_view)),
                entry(2, partials.as_entire_binding()),
                entry(3, self.state.as_entire_binding()),
            ],
        );
        let bloom_group = |label, source| {
            group(
                label,
                &self.bloom_layout,
                &[
                    entry(0, self.uniforms.as_entire_binding()),
                    entry(1, view(source)),
                    entry(2, self.state.as_entire_binding()),
                ],
            )
        };
        let bloom_down_group = bloom_group("bloom down", &hdr_view);
        let blur_h_group = bloom_group("bloom blur h", &bloom_a);
        let blur_v_group = bloom_group("bloom blur v", &bloom_b);
        let tonemap_group = group(
            "tonemap",
            &self.tonemap_layout,
            &[
                entry(0, self.uniforms.as_entire_binding()),
                entry(1, view(&hdr_view)),
                entry(2, self.state.as_entire_binding()),
                entry(3, view(&bloom_a)),
                entry(4, wgpu::BindingResource::Sampler(&self.sampler)),
            ],
        );
        let final_group = group(
            "final",
            &self.final_layout,
            &[
                entry(0, self.uniforms.as_entire_binding()),
                entry(1, view(&ldr_view)),
                entry(2, wgpu::BindingResource::Sampler(&self.sampler)),
            ],
        );
        self.targets = Some(PostTargets {
            meter_groups,
            atmosphere_inputs,
            hdr_view,
            exposure_group,
            bloom_a,
            bloom_b,
            bloom_down_group,
            blur_h_group,
            blur_v_group,
            tonemap_group,
            ldr_view,
            final_group,
        });
    }

    /// Encode atmosphere, eye adaptation, bloom, tonemapping and the final pass into `output`.
    pub fn encode(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame_group: &wgpu::BindGroup,
        output: &wgpu::TextureView,
        settings: &HdrSettings,
        dt: f32,
    ) {
        let uniforms = settings.uniforms(dt, self.reset_adaptation, self.output_srgb);
        self.reset_adaptation = false;
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let t = self.targets.as_ref().expect("resize before encode");

        fullscreen_pass(encoder, "atmosphere", &t.hdr_view, |pass| {
            pass.set_pipeline(&self.atmosphere);
            pass.set_bind_group(0, frame_group, &[]);
            pass.set_bind_group(1, &t.atmosphere_inputs, &[]);
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("eye adaptation"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &t.exposure_group, &[]);
            pass.set_pipeline(&self.measure);
            pass.dispatch_workgroups(t.meter_groups.0, t.meter_groups.1, 1);
            pass.set_pipeline(&self.adapt);
            pass.dispatch_workgroups(1, 1, 1);
        }
        if settings.bloom.enabled {
            fullscreen_pass(encoder, "bloom down", &t.bloom_a, |pass| {
                pass.set_pipeline(&self.bloom_down);
                pass.set_bind_group(0, &t.bloom_down_group, &[]);
            });
            fullscreen_pass(encoder, "bloom blur h", &t.bloom_b, |pass| {
                pass.set_pipeline(&self.blur_h);
                pass.set_bind_group(0, &t.blur_h_group, &[]);
            });
            fullscreen_pass(encoder, "bloom blur v", &t.bloom_a, |pass| {
                pass.set_pipeline(&self.blur_v);
                pass.set_bind_group(0, &t.blur_v_group, &[]);
            });
        }
        fullscreen_pass(encoder, "tonemap", &t.ldr_view, |pass| {
            pass.set_pipeline(&self.tonemap);
            pass.set_bind_group(0, &t.tonemap_group, &[]);
        });
        fullscreen_pass(encoder, "final", output, |pass| {
            pass.set_pipeline(match settings.anti_aliasing {
                AntiAliasing::Fxaa => &self.fxaa,
                AntiAliasing::None => &self.copy,
            });
            pass.set_bind_group(0, &t.final_group, &[]);
        });
    }
}

fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    groups: &[&wgpu::BindGroupLayout],
    shader: &wgpu::ShaderModule,
    fragment_entry: &str,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let layout = crate::renderer::pipeline_layout(device, label, groups);
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: Default::default(),
            targets: &[Some(format.into())],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// One full-screen triangle into `target`; `setup` binds pipeline and groups.
fn fullscreen_pass(
    encoder: &mut wgpu::CommandEncoder,
    label: &str,
    target: &wgpu::TextureView,
    setup: impl FnOnce(&mut wgpu::RenderPass<'_>),
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    setup(&mut pass);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rv_tonemap_methods() {
        // 0 = None, 1 = Filmic, 2 = Reinhard (reverse engineered; no ACES in RV).
        assert_eq!(Tonemap::from_rv_method(0), Tonemap::None);
        assert_eq!(Tonemap::from_rv_method(1), Tonemap::Filmic);
        assert_eq!(Tonemap::from_rv_method(2), Tonemap::Reinhard);
        assert_eq!(Tonemap::from_rv_method(7), Tonemap::Filmic);
        for t in [Tonemap::None, Tonemap::Filmic, Tonemap::Reinhard] {
            assert_eq!(Tonemap::from_rv_method(t.rv_method()), t);
        }
    }

    #[test]
    fn aperture_stage_follows_the_lighting_entry() {
        // Altis Lighting0 (the night entry): 4 / 4 / 8 at a standard luminance of 4.
        let night = HdrSettings {
            aperture_min: 4.0,
            aperture_standard: 4.0,
            aperture_max: 8.0,
            standard_avg_lum: 4.0,
            ..HdrSettings::default()
        };
        assert!(night.has_aperture_stage());
        assert_eq!(night.aperture(4.0), 4.0);
        // apertureStandard == apertureMin: the dark branch cannot open further.
        assert_eq!(night.aperture(0.4), 4.0);
        assert_eq!(night.aperture(1e-3), 4.0);
        // Brighter: linearly towards apertureMax, reached at 4 * apertureRatioMax = 16.
        assert!(
            (night.aperture(10.0) - 6.0).abs() < 1e-6,
            "{}",
            night.aperture(10.0)
        );
        assert_eq!(night.aperture(16.0), 8.0);
        assert_eq!(night.aperture(1000.0), 8.0);
        assert!((night.aperture_exposure(16.0) - 1.0 / 64.0).abs() < 1e-9);
        let (lo, hi) = night.exposure_range();
        assert!((lo - 1.0 / 64.0).abs() < 1e-9, "{lo}");
        assert!((hi - 1.0 / 16.0).abs() < 1e-9, "{hi}");
        assert!((night.assumed_key() - 0.25).abs() < 1e-6);
    }

    #[test]
    fn aperture_stage_follows_the_noon_entry() {
        // Altis Lighting11/12 (noon): 70 / 120 / 120 at a standard luminance of 8000.
        let noon = HdrSettings {
            aperture_min: 70.0,
            aperture_standard: 120.0,
            aperture_max: 120.0,
            standard_avg_lum: 8000.0,
            ..HdrSettings::default()
        };
        // Just above the standard luminance the bright branch starts at the standard aperture;
        // at the standard luminance itself the engine's literal dark branch is below it.
        assert_eq!(noon.aperture(8001.0), 120.0);
        assert_eq!(noon.aperture(32000.0), 120.0);
        assert_eq!(noon.aperture(1e6), 120.0);
        // apertureRatioMin = 10: the minimum is reached at 800.
        assert_eq!(noon.aperture(8000.0), 110.909_09);
        assert!(
            (noon.aperture(7999.0) - 110.909).abs() < 1e-3,
            "{}",
            noon.aperture(7999.0)
        );
        assert!(
            (noon.aperture(4000.0) - 106.3636).abs() < 1e-3,
            "{}",
            noon.aperture(4000.0)
        );
        assert!(
            (noon.aperture(800.0) - 70.0).abs() < 1e-4,
            "{}",
            noon.aperture(800.0)
        );
        assert_eq!(noon.aperture(10.0), 70.0);
        assert!((noon.aperture_exposure(32000.0) - 1.0 / 14400.0).abs() < 1e-12);
        let (lo, hi) = noon.exposure_range();
        assert!((lo - 1.0 / 14400.0).abs() < 1e-12, "{lo}");
        assert!((hi - 1.0 / 4900.0).abs() < 1e-12, "{hi}");
        assert!((noon.assumed_key() - 8000.0 / 14400.0).abs() < 1e-6);
    }

    #[test]
    fn aperture_is_clamped_to_the_global_limits() {
        // Altis Lighting0 with an HDRNewPars floor above the entry's own minimum: the aperture
        // cannot open past it, so the whole range collapses onto it.
        let floored = HdrSettings {
            min_aperture: 8.0,
            max_aperture: 16.0,
            aperture_min: 4.0,
            aperture_standard: 4.0,
            aperture_max: 8.0,
            standard_avg_lum: 4.0,
            ..HdrSettings::default()
        };
        assert_eq!(floored.aperture(4.0), 8.0);
        assert_eq!(floored.aperture(1e-3), 8.0);
        assert_eq!(floored.aperture(1000.0), 8.0);
        assert!((floored.aperture_exposure(4.0) - 1.0 / 64.0).abs() < 1e-12);
        assert_eq!(floored.exposure_range(), (1.0 / 64.0, 1.0 / 64.0));
        // A ceiling below the entry's own maximum (HDRNewPars maxAperture).
        let closed = HdrSettings {
            min_aperture: 1e-5,
            max_aperture: 6.0,
            aperture_min: 4.0,
            aperture_standard: 4.0,
            aperture_max: 8.0,
            standard_avg_lum: 4.0,
            ..HdrSettings::default()
        };
        assert_eq!(closed.aperture(1000.0), 6.0);
        assert!((closed.aperture_exposure(1000.0) - 1.0 / 36.0).abs() < 1e-12);
    }

    #[test]
    fn without_a_lighting_entry_the_assumed_luminance_step_stays() {
        // World-less renders and synthetic scenes: no aperture stage, so the exposure is
        // key / measured within key / maxAperture ..= key / minAperture, as before the stage.
        let s = HdrSettings::default();
        assert!(!s.has_aperture_stage());
        assert_eq!(s.assumed_key(), 0.3);
        let (lo, hi) = s.exposure_range();
        assert!((lo - 0.3 / 256.0).abs() < 1e-7, "{lo}");
        assert!((hi - 0.3 / 1e-5).abs() < 1.0, "{hi}");
        let u = s.uniforms(0.016, false, false);
        assert_eq!(u.exposure[2], 0.3);
        assert_eq!(u.aperture[1], 0.0);
    }

    #[test]
    fn uniforms_carry_the_aperture_exposure_range() {
        let noon = HdrSettings {
            aperture_min: 70.0,
            aperture_standard: 120.0,
            aperture_max: 120.0,
            standard_avg_lum: 8000.0,
            ..HdrSettings::default()
        };
        let u = noon.uniforms(0.016, false, false);
        assert!((u.exposure[0] - 1.0 / 14400.0).abs() < 1e-12);
        assert!((u.exposure[1] - 1.0 / 4900.0).abs() < 1e-12);
        assert!((u.exposure[2] - 8000.0 / 14400.0).abs() < 1e-6);
    }

    #[test]
    fn adaptation_halves_in_half_a_second_and_doubles_in_one_at_daylight_exposure() {
        // RV's PSC_AssumedLuminancePars2.zw (render-atmosphere.md §3.3): with the CPU
        // exposure at or below 1 the exposure may halve every 0.5 s and double every 1 s.
        let s = HdrSettings::default();
        let (down, up) = s.adaptation_limits(0.25);
        assert!((down - 0.5f32.sqrt()).abs() < 1e-6, "{down}");
        assert!((up - 2f32.powf(0.25)).abs() < 1e-6, "{up}");
    }

    #[test]
    fn adaptation_to_the_dark_slows_down_at_high_cpu_exposure() {
        // From a CPU exposure of 4 up: halving takes 1 s, doubling 20 s; linear in between.
        let night = HdrSettings {
            cpu_exposure: 6.0,
            ..HdrSettings::default()
        };
        let (down, up) = night.adaptation_limits(1.0);
        assert!((down - 0.5).abs() < 1e-6);
        assert!((up - 2f32.powf(1.0 / 20.0)).abs() < 1e-6);
        let dusk = HdrSettings {
            cpu_exposure: 2.5,
            ..HdrSettings::default()
        };
        // f = 0.5: tau_down = 0.75 s, tau_up = 10.5 s.
        let (down, up) = dusk.adaptation_limits(1.0);
        assert!((down - 2f32.powf(-1.0 / 0.75)).abs() < 1e-6);
        assert!((up - 2f32.powf(1.0 / 10.5)).abs() < 1e-6);
    }

    #[test]
    fn adaptation_always_allows_a_tenth_of_a_percent_per_frame() {
        let s = HdrSettings::default();
        assert_eq!(s.adaptation_limits(0.0), (0.999, 1.001));
        assert_eq!(s.adaptation_limits(1e-6), (0.999, 1.001));
    }

    #[test]
    fn uniforms_carry_reset_and_fixed_exposure() {
        let s = HdrSettings {
            fixed_exposure: Some(2.0),
            anti_aliasing: AntiAliasing::None,
            ..HdrSettings::default()
        };
        let u = s.uniforms(0.016, true, true);
        assert_eq!(u.exposure[3], 2.0);
        assert_eq!(u.adaptation[2..], [0.016, 1.0]);
        assert_eq!(u.misc[2..], [1.0, 0.0]);
        assert_eq!(u.bloom, [1.0, 0.09, 1.0, 0.75]);
        assert_eq!(u.output, [1.0, 1.0, 0.0, 0.0]);
        let auto = HdrSettings::default().uniforms(-1.0, false, false);
        assert_eq!((auto.exposure[3], auto.adaptation[2]), (0.0, 0.0));
    }
}
