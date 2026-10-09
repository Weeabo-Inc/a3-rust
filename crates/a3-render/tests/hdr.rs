//! GPU tests of the post chain: eye adaptation, fixed exposure and FXAA. Skip without an adapter.

mod common;

use common::gpu;

use a3_render::{
    AntiAliasing, Camera, DrawList, Gpu, HdrSettings, MeshData, MeshDraw, Renderer,
    wgpu::TextureFormat,
};
use glam::{DAffine3, DQuat, DVec3, Vec3};

const SIZE: u32 = 64;
const DT: f32 = 1.0 / 60.0;

struct Scene {
    renderer: Renderer,
    camera: Camera,
    draws: DrawList,
}

/// A grey cube rotated 30 degrees about the view axis in front of the sky.
fn scene(gpu: &Gpu) -> Scene {
    let mut renderer = Renderer::new(gpu, TextureFormat::Rgba8UnormSrgb);
    let cube = renderer.upload_mesh(gpu, &MeshData::cuboid(Vec3::splat(2.0)));
    let camera = Camera {
        position: DVec3::new(5_000.0, 20.0, 5_000.0),
        ..Camera::default()
    };
    let mut draws = DrawList::default();
    draws.mesh(MeshDraw {
        transform: DAffine3::from_rotation_translation(
            DQuat::from_rotation_z(0.5),
            camera.position + DVec3::new(0.0, 0.0, 6.0),
        ),
        ..MeshDraw::at(cube, DVec3::ZERO, [0.1, 0.1, 0.1, 1.0])
    });
    Scene {
        renderer,
        camera,
        draws,
    }
}

impl Scene {
    fn render(&mut self, gpu: &Gpu, dt: f32) -> Vec<u8> {
        self.renderer
            .render_to_image(gpu, SIZE, SIZE, &self.camera, &self.draws, dt)
            .expect("render")
    }

    /// Multiply every light source by `factor`.
    fn scale_light(&mut self, factor: f32) {
        let s = &mut self.renderer.settings;
        s.sun_color *= factor;
        s.ambient *= factor;
        s.sky_zenith *= factor;
        s.sky_horizon *= factor;
    }

    /// An aperture stage wide enough that the test scene's luminance stays between its limits,
    /// so the per-frame limits are the only thing shaping the step.
    fn wide_aperture(&mut self) {
        self.renderer.settings.hdr = HdrSettings {
            aperture_min: 0.25,
            aperture_standard: 4.0,
            aperture_max: 64.0,
            standard_avg_lum: 4.0,
            ..HdrSettings::default()
        };
    }

    /// The exposure the CPU-side stage asks for at the measured luminance: the aperture stage, or
    /// the assumed-luminance step without a lighting entry.
    fn expected_exposure(&self, measured: f32) -> f32 {
        let hdr = &self.renderer.settings.hdr;
        let wanted = if hdr.has_aperture_stage() {
            hdr.aperture_exposure(measured)
        } else {
            hdr.assumed_key() / measured
        };
        let (lo, hi) = hdr.exposure_range();
        wanted.clamp(lo, hi)
    }
}

fn mean_rgb(image: &[u8]) -> f32 {
    let sum: u64 = image
        .chunks_exact(4)
        .map(|p| u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2]))
        .sum();
    sum as f32 / (image.len() / 4 * 3) as f32
}

#[test]
fn the_exposure_pass_applies_the_aperture_stage() {
    let Some(gpu) = gpu() else { return };
    for settings in [
        HdrSettings::default(),
        HdrSettings {
            aperture_min: 0.25,
            aperture_standard: 4.0,
            aperture_max: 64.0,
            standard_avg_lum: 4.0,
            ..HdrSettings::default()
        },
    ] {
        let mut s = scene(&gpu);
        s.renderer.settings.hdr = settings;
        for _ in 0..120 {
            s.render(&gpu, DT);
        }
        let settled = s.renderer.read_exposure(&gpu).unwrap();
        let wanted = s.expected_exposure(settled.average_luminance);
        assert!(
            (settled.exposure / wanted - 1.0).abs() < 0.01,
            "the pass must settle on 1 / aperture(measured)²: {settled:?} vs {wanted} \
             for {settings:?}"
        );
    }
}

#[test]
fn eye_adaptation_follows_the_aperture_stage() {
    let Some(gpu) = gpu() else { return };
    let mut s = scene(&gpu);
    s.wide_aperture();
    for _ in 0..120 {
        s.render(&gpu, DT);
    }
    let normal = mean_rgb(&s.render(&gpu, DT));
    let normal_exposure = s.renderer.read_exposure(&gpu).unwrap();

    // 16x brighter light: the aperture stage lets less light through, so the image changes far
    // less than the light does (the aperture alone cannot hold the mean, see the unit tests).
    s.scale_light(16.0);
    s.renderer.reset_eye_adaptation();
    let bright = mean_rgb(&s.render(&gpu, DT));
    let bright_exposure = s.renderer.read_exposure(&gpu).unwrap();

    let ratio = normal_exposure.exposure / bright_exposure.exposure;
    assert!(
        ratio > 4.0,
        "exposure should follow the aperture stage: {normal_exposure:?} vs {bright_exposure:?}"
    );
    assert!(
        bright < 4.0 * normal,
        "the adapted image should change far less than the light: mean {normal} vs {bright}"
    );
}

#[test]
fn adaptation_steps_are_limited_per_frame() {
    let Some(gpu) = gpu() else { return };
    let mut s = scene(&gpu);
    s.wide_aperture();
    s.render(&gpu, DT);
    let start = s.renderer.read_exposure(&gpu).unwrap();

    // 16x brighter: at a CPU exposure of 1 the exposure may halve every 0.5 s: 0.2 stops in 0.1 s.
    s.scale_light(16.0);
    s.render(&gpu, 0.1);
    let step = s.renderer.read_exposure(&gpu).unwrap();
    let ratio = step.exposure / start.exposure;
    assert!(
        (ratio - 2f32.powf(-0.2)).abs() < 1e-3,
        "ratio {ratio}: {start:?} -> {step:?}"
    );
    for _ in 0..60 {
        s.render(&gpu, 0.1);
    }
    let settled = s.renderer.read_exposure(&gpu).unwrap();
    let wanted = s.expected_exposure(settled.average_luminance);
    assert!(
        (settled.exposure / wanted - 1.0).abs() < 0.02,
        "exposure should settle on the aperture stage: {settled:?} vs {wanted}"
    );

    // 16x darker again: it may double every 1 s: 0.1 stops in 0.1 s.
    s.scale_light(1.0 / 16.0);
    s.render(&gpu, 0.1);
    let darker = s.renderer.read_exposure(&gpu).unwrap();
    let ratio = darker.exposure / settled.exposure;
    assert!(
        (ratio - 2f32.powf(0.1)).abs() < 1e-3,
        "ratio {ratio}: {settled:?} -> {darker:?}"
    );
}

#[test]
fn the_aperture_stage_pins_the_exposure_at_the_entry_limits() {
    let Some(gpu) = gpu() else { return };
    let exposure = |settings: HdrSettings| {
        let mut s = scene(&gpu);
        s.renderer.settings.hdr = settings;
        s.render(&gpu, DT);
        s.renderer.read_exposure(&gpu).unwrap().exposure
    };
    // A scene far darker than the entry's standard luminance: the aperture is fully open and
    // the exposure is 1 / apertureMin² (Altis Lighting0 at night).
    let night = HdrSettings {
        aperture_min: 4.0,
        aperture_standard: 4.0,
        aperture_max: 8.0,
        standard_avg_lum: 1.0e6,
        ..HdrSettings::default()
    };
    let open = exposure(night);
    assert!((open - 1.0 / 16.0).abs() < 1e-6, "{open}");
    // A scene far brighter: the aperture is closed, exposure 1 / apertureMax².
    let noon = HdrSettings {
        aperture_min: 70.0,
        aperture_standard: 120.0,
        aperture_max: 120.0,
        standard_avg_lum: 1.0e-6,
        ..HdrSettings::default()
    };
    let closed = exposure(noon);
    assert!((closed - 1.0 / 14400.0).abs() < 1e-9, "{closed}");
}

#[test]
fn fixed_exposure_overrides_adaptation() {
    let Some(gpu) = gpu() else { return };
    let mut s = scene(&gpu);
    s.renderer.settings.hdr.fixed_exposure = Some(0.75);
    s.render(&gpu, DT);
    assert_eq!(s.renderer.read_exposure(&gpu).unwrap().exposure, 0.75);
}

#[test]
fn fxaa_softens_aliased_edges() {
    let Some(gpu) = gpu() else { return };
    // Pixels that are neither cube nor sky: blended edge pixels.
    let blended = |image: &[u8], cube: [u8; 4], sky: [u8; 4]| {
        let far = |a: &[u8], b: [u8; 4]| (0..3).any(|i| a[i].abs_diff(b[i]) > 24);
        image
            .chunks_exact(4)
            .filter(|p| far(p, cube) && far(p, sky))
            .count()
    };
    let mut counts = Vec::new();
    for aa in [AntiAliasing::None, AntiAliasing::Fxaa] {
        let mut s = scene(&gpu);
        s.renderer.settings.hdr = HdrSettings {
            anti_aliasing: aa,
            fixed_exposure: Some(1.0),
            ..HdrSettings::default()
        };
        // Flat sky so only the cube edge has contrast.
        s.renderer.settings.sky_zenith = s.renderer.settings.sky_horizon;
        let image = s.render(&gpu, DT);
        let px = |x: u32, y: u32| {
            let i = ((y * SIZE + x) * 4) as usize;
            [image[i], image[i + 1], image[i + 2], image[i + 3]]
        };
        counts.push(blended(&image, px(SIZE / 2, SIZE / 2), px(1, 1)));
    }
    assert!(
        counts[1] > counts[0] + 10,
        "FXAA should add blended edge pixels: none {} vs fxaa {}",
        counts[0],
        counts[1]
    );
}

#[test]
fn rv_tonemap_methods_differ_on_bright_light() {
    use a3_render::Tonemap;
    let Some(gpu) = gpu() else { return };
    let centre = |method| {
        let mut s = scene(&gpu);
        // Scene values between 1 and the Reinhard white point (2.5): only method 0 clips.
        s.scale_light(12.0);
        s.renderer.settings.hdr = HdrSettings {
            tonemap: method,
            fixed_exposure: Some(1.0),
            anti_aliasing: AntiAliasing::None,
            ..HdrSettings::default()
        };
        let image = s.render(&gpu, DT);
        let i = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
        image[i]
    };
    let none = centre(Tonemap::None);
    let filmic = centre(Tonemap::Filmic);
    let reinhard = centre(Tonemap::Reinhard);
    assert_eq!(none, 255, "method 0 only clamps");
    assert!(
        filmic < 255 && reinhard < 255,
        "filmic {filmic}, reinhard {reinhard}"
    );
    assert_ne!(filmic, reinhard);
}

#[test]
fn bloom_lifts_dark_pixels_next_to_bright_ones() {
    use a3_render::BloomSettings;
    let Some(gpu) = gpu() else { return };
    let brightness = |bloom: bool| {
        let mut s = scene(&gpu);
        s.renderer.settings.hdr = HdrSettings {
            fixed_exposure: Some(1.0),
            anti_aliasing: AntiAliasing::None,
            bloom: BloomSettings {
                enabled: bloom,
                ..BloomSettings::default()
            },
            ..HdrSettings::default()
        };
        // Bright sky around a dark cube.
        s.scale_light(4.0);
        let image = s.render(&gpu, DT);
        let i = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
        u32::from(image[i]) + u32::from(image[i + 1]) + u32::from(image[i + 2])
    };
    let (off, on) = (brightness(false), brightness(true));
    assert!(
        on > off,
        "bloom should brighten the dark cube centre: {off} -> {on}"
    );
}
