//! The post chain: atmosphere into HDR, eye adaptation, tonemapping and anti-aliasing.
//!
//! Defaults mirror `CfgWorlds >> Altis >> HDRNewPars` (see `docs/re/hdr.md`).

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

/// HDR, eye adaptation, tonemapping and anti-aliasing settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HdrSettings {
    pub tonemap: Tonemap,
    pub filmic: FilmicCurve,
    /// `tonemapExposureBias`.
    pub exposure_bias: f32,
    /// `tonemapLinearWhiteReinhard`.
    pub reinhard_white: f32,
    /// `minAperture` / `maxAperture`: range of the adapted luminance.
    pub min_aperture: f32,
    pub max_aperture: f32,
    /// `eyeAdaptFactorLight`: adaptation speed (1/s) towards brighter scenes.
    pub eye_adapt_light: f32,
    /// `eyeAdaptFactorDark`: adaptation speed (1/s) towards darker scenes.
    pub eye_adapt_dark: f32,
    /// Display value the average scene luminance is exposed to (our choice, see
    /// `docs/re/hdr.md`).
    pub key: f32,
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
            min_aperture: 1e-5,
            max_aperture: 256.0,
            eye_adapt_light: 3.3,
            eye_adapt_dark: 0.75,
            key: 0.3,
            fixed_exposure: None,
            anti_aliasing: AntiAliasing::Fxaa,
        }
    }
}

impl HdrSettings {
    /// log2 of the histogram's lower bound and its range in stops.
    pub fn log_luminance_range(&self) -> (f32, f32) {
        let min = self.min_aperture.max(1e-12).log2();
        let max = self.max_aperture.max(self.min_aperture * 2.0).log2();
        (min, max - min)
    }

    fn uniforms(&self, dt: f32, reset: bool, output_srgb: bool) -> PostUniforms {
        let f = &self.filmic;
        let (min_log, range) = self.log_luminance_range();
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
            exposure: [min_log, range, self.key, self.fixed_exposure.unwrap_or(0.0)],
            adaptation: [
                self.eye_adapt_light,
                self.eye_adapt_dark,
                dt.max(0.0),
                if reset { 1.0 } else { 0.0 },
            ],
            misc: [
                self.tonemap.shader_index(),
                self.reinhard_white,
                if output_srgb { 1.0 } else { 0.0 },
                if self.anti_aliasing == AntiAliasing::Fxaa {
                    1.0
                } else {
                    0.0
                },
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
}

/// Format of the intermediate HDR image after the atmosphere pass.
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Gamma-encoded tonemapped image with luma in alpha.
const LDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct PostTargets {
    size: (u32, u32),
    atmosphere_inputs: wgpu::BindGroup,
    hdr_view: wgpu::TextureView,
    exposure_group: wgpu::BindGroup,
    tonemap_group: wgpu::BindGroup,
    ldr_view: wgpu::TextureView,
    final_group: wgpu::BindGroup,
}

/// GPU side of the post chain.
pub(crate) struct PostChain {
    output_srgb: bool,
    uniforms: wgpu::Buffer,
    histogram: wgpu::Buffer,
    state: wgpu::Buffer,
    sampler: wgpu::Sampler,
    atmosphere_layout: wgpu::BindGroupLayout,
    atmosphere: wgpu::RenderPipeline,
    exposure_layout: wgpu::BindGroupLayout,
    build_histogram: wgpu::ComputePipeline,
    adapt: wgpu::ComputePipeline,
    tonemap_layout: wgpu::BindGroupLayout,
    tonemap: wgpu::RenderPipeline,
    final_layout: wgpu::BindGroupLayout,
    fxaa: wgpu::RenderPipeline,
    copy: wgpu::RenderPipeline,
    targets: Option<PostTargets>,
    reset_adaptation: bool,
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
        let histogram = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("luminance histogram"),
            size: 256 * 4,
            usage: wgpu::BufferUsages::STORAGE,
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

        let float_tex = |binding, visibility, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uniform = |visibility| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let storage = |binding, visibility, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let atmosphere_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atmosphere inputs"),
            entries: &[
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
        });
        let exposure_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("exposure"),
            entries: &[
                uniform(S::COMPUTE),
                float_tex(1, S::COMPUTE, false),
                storage(2, S::COMPUTE, false),
                storage(3, S::COMPUTE, false),
            ],
        });
        let tonemap_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tonemap"),
            entries: &[
                uniform(S::FRAGMENT),
                float_tex(1, S::FRAGMENT, false),
                storage(2, S::FRAGMENT, true),
            ],
        });
        let final_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("final"),
            entries: &[
                uniform(S::FRAGMENT),
                float_tex(1, S::FRAGMENT, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: S::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let atmosphere_shader = crate::renderer::shader(
            device,
            "atmosphere",
            include_str!("../shaders/atmosphere.wgsl"),
        );
        let atmosphere = fullscreen_pipeline(
            device,
            "atmosphere",
            &[frame_layout, &atmosphere_layout],
            &atmosphere_shader,
            "fs_main",
            HDR_FORMAT,
        );

        let exposure_shader =
            crate::renderer::shader(device, "exposure", include_str!("../shaders/exposure.wgsl"));
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
        let build_histogram = compute("build_histogram");
        let adapt = compute("adapt");

        let tonemap_shader =
            crate::renderer::shader(device, "tonemap", include_str!("../shaders/tonemap.wgsl"));
        let tonemap = fullscreen_pipeline(
            device,
            "tonemap",
            &[&tonemap_layout],
            &tonemap_shader,
            "fs_main",
            LDR_FORMAT,
        );
        let final_shader =
            crate::renderer::shader(device, "final", include_str!("../shaders/final.wgsl"));
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
            histogram,
            state,
            sampler,
            atmosphere_layout,
            atmosphere,
            exposure_layout,
            build_histogram,
            adapt,
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
        let texture = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
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
                .create_view(&Default::default())
        };
        let hdr_view = texture("post hdr", HDR_FORMAT);
        let ldr_view = texture("post ldr", LDR_FORMAT);
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
                entry(2, self.histogram.as_entire_binding()),
                entry(3, self.state.as_entire_binding()),
            ],
        );
        let tonemap_group = group(
            "tonemap",
            &self.tonemap_layout,
            &[
                entry(0, self.uniforms.as_entire_binding()),
                entry(1, view(&hdr_view)),
                entry(2, self.state.as_entire_binding()),
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
            size,
            atmosphere_inputs,
            hdr_view,
            exposure_group,
            tonemap_group,
            ldr_view,
            final_group,
        });
    }

    /// Encode atmosphere, eye adaptation, tonemapping and the final pass into `output`.
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
            pass.set_pipeline(&self.build_histogram);
            pass.dispatch_workgroups(t.size.0.div_ceil(16), t.size.1.div_ceil(16), 1);
            pass.set_pipeline(&self.adapt);
            pass.dispatch_workgroups(1, 1, 1);
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
    fn histogram_spans_the_aperture_range() {
        let s = HdrSettings::default();
        let (min, range) = s.log_luminance_range();
        assert!((min - 1e-5f32.log2()).abs() < 1e-4);
        assert!((min + range - 8.0).abs() < 1e-4, "max aperture 256 = 2^8");
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
        let auto = HdrSettings::default().uniforms(-1.0, false, false);
        assert_eq!((auto.exposure[3], auto.adaptation[2]), (0.0, 0.0));
    }
}
