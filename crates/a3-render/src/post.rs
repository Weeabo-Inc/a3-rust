//! The post chain: atmosphere into HDR, eye adaptation, bloom, tonemapping and anti-aliasing.
//!
//! Follows RV's HDR chain as reverse engineered in `docs/re/render-atmosphere.md` §3: log-average
//! luminance meter, assumed-luminance adaptation, bloom mixed before the curve, `tonemapMethod`
//! curves and a final gamma. Defaults mirror `CfgWorlds >> Altis >> HDRNewPars`.

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
    /// `minAperture` / `maxAperture`: range of the adapted luminance (exposure is
    /// `key / adapted`).
    pub min_aperture: f32,
    pub max_aperture: f32,
    /// `eyeAdaptFactorLight`: how fast the eye adapts to a brighter scene, in stops per second
    /// _(uncertain mapping to RV's per-frame limits, see `docs/re/render-atmosphere.md` §3.3)_.
    pub eye_adapt_light: f32,
    /// `eyeAdaptFactorDark`: how fast the eye adapts to a darker scene, stops per second.
    pub eye_adapt_dark: f32,
    /// Target of the adaptation: exposure = key / measured log-average luminance (RV's
    /// `PSC_AssumedLuminancePars1.z`; its value is not traced, ours is chosen by eye).
    pub key: f32,
    /// Final power applied after the curve (`PSC_RgbEyeCoef.w`; CPU value not traced, 1 keeps
    /// the curve's output). The sRGB encode for display follows separately.
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
            eye_adapt_light: 3.3,
            eye_adapt_dark: 0.75,
            key: 0.3,
            final_gamma: 1.0,
            fixed_exposure: None,
            anti_aliasing: AntiAliasing::Fxaa,
        }
    }
}

impl HdrSettings {
    /// Exposure range implied by the aperture limits: `key / maxAperture ..= key / minAperture`.
    pub fn exposure_range(&self) -> (f32, f32) {
        let max_ap = self.max_aperture.max(self.min_aperture).max(1e-12);
        let min_ap = self.min_aperture.max(1e-12);
        (self.key / max_ap, self.key / min_ap)
    }

    /// Per-frame limits of the exposure ratio (RV's `PSC_AssumedLuminancePars2.zw`): exposure
    /// may fall by `eye_adapt_light` stops per second (scene got brighter) and rise by
    /// `eye_adapt_dark` stops per second (scene got darker).
    pub fn adaptation_limits(&self, dt: f32) -> (f32, f32) {
        let dt = dt.max(0.0);
        (
            (-self.eye_adapt_light.max(0.0) * dt).exp2(),
            (self.eye_adapt_dark.max(0.0) * dt).exp2(),
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
                self.key,
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
    fn exposure_range_follows_the_aperture_limits() {
        let s = HdrSettings::default();
        let (lo, hi) = s.exposure_range();
        assert!((lo - 0.3 / 256.0).abs() < 1e-7);
        assert!((hi - 0.3 / 1e-5).abs() < 1.0);
    }

    #[test]
    fn adaptation_limits_are_stops_per_second() {
        let s = HdrSettings::default();
        let (down, up) = s.adaptation_limits(0.5);
        // eyeAdaptFactorLight 3.3 stops/s down, eyeAdaptFactorDark 0.75 stops/s up.
        assert!((down - 2f32.powf(-1.65)).abs() < 1e-6);
        assert!((up - 2f32.powf(0.375)).abs() < 1e-6);
        assert_eq!(s.adaptation_limits(0.0), (1.0, 1.0));
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
