//! GPU tests of the sky feature and the environment terms of the frame (hemisphere ambient,
//! height fog). Skip without an adapter.

mod common;

use common::gpu;

use a3_render::sky::{SkyFeature, SkyParams};
use a3_render::{Camera, DrawList, Gpu, MeshData, MeshDraw, Renderer, wgpu::TextureFormat};
use glam::{DVec3, Vec3};

const SIZE: u32 = 64;
const DT: f32 = 1.0 / 60.0;

fn pixel(image: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [image[i], image[i + 1], image[i + 2], image[i + 3]]
}

/// A camera looking north, level.
fn camera() -> Camera {
    Camera {
        position: DVec3::new(10_000.0, 30.0, 10_000.0),
        ..Camera::default()
    }
}

fn renderer_with_sky(gpu: &Gpu, params: SkyParams) -> (Renderer, a3_render::sky::SkyHandle) {
    let mut renderer = Renderer::new(gpu, TextureFormat::Rgba8UnormSrgb);
    renderer.settings.hdr.fixed_exposure = Some(1.0);
    renderer.settings.procedural_sky = false;
    renderer.settings.fog_density = 0.0;
    let (feature, handle) = SkyFeature::new(gpu, &renderer, None);
    *handle.lock().unwrap() = params;
    renderer.add_feature(Box::new(feature));
    (renderer, handle)
}

fn dark_sky() -> SkyParams {
    SkyParams {
        zenith: Vec3::new(0.0, 0.0, 0.5),
        horizon: Vec3::new(0.0, 0.0, 0.5),
        around_sun: Vec3::ZERO,
        sun_direction: Vec3::new(0.0, -1.0, 0.0),
        sun_color: Vec3::ZERO,
        ..SkyParams::default()
    }
}

#[test]
fn the_sky_feature_fills_the_background_behind_geometry() {
    let Some(gpu) = gpu() else { return };
    let (mut renderer, _) = renderer_with_sky(
        &gpu,
        SkyParams {
            zenith: Vec3::new(0.0, 0.6, 0.0),
            horizon: Vec3::new(0.0, 0.6, 0.0),
            ..dark_sky()
        },
    );
    let cube = renderer.upload_mesh(&gpu, &MeshData::cuboid(Vec3::splat(2.0)));
    let cam = camera();
    let mut draws = DrawList::default();
    draws.mesh(MeshDraw::at(
        cube,
        cam.position + DVec3::new(0.0, 0.0, 5.0),
        [1.0, 0.0, 0.0, 1.0],
    ));
    renderer.settings.sun_color = Vec3::ZERO;
    renderer.settings.hemisphere = Some(a3_render::HemisphereAmbient {
        sky: Vec3::splat(1.0),
        mid: Vec3::splat(1.0),
        ground: Vec3::splat(1.0),
    });
    let image = renderer
        .render_to_image(&gpu, SIZE, SIZE, &cam, &draws, DT)
        .unwrap();
    let [r, g, b, _] = pixel(&image, SIZE / 2, 2);
    assert!(
        g > 100 && r < 30 && b < 30,
        "sky should be the feature's green: {r} {g} {b}"
    );
    let [r, g, b, _] = pixel(&image, SIZE / 2, SIZE / 2);
    assert!(r > 100 && g < 40, "the cube stays in front: {r} {g} {b}");
}

#[test]
fn the_sun_disc_shows_where_the_sun_is() {
    let Some(gpu) = gpu() else { return };
    let (mut renderer, handle) = renderer_with_sky(
        &gpu,
        SkyParams {
            sun_direction: Vec3::Z,
            sun_color: Vec3::splat(1.0),
            sun_disc_scale: 1000.0,
            // Several pixels wide at this resolution.
            sun_radius: 4f32.to_radians(),
            ..dark_sky()
        },
    );
    let image = renderer
        .render_to_image(&gpu, SIZE, SIZE, &camera(), &DrawList::default(), DT)
        .unwrap();
    let [r, g, _, _] = pixel(&image, SIZE / 2, SIZE / 2);
    assert!(r > 240 && g > 240, "looking at the sun: {r} {g}");
    let [r, _, b, _] = pixel(&image, 4, 4);
    assert!(
        r < 40 && b > 60,
        "away from the sun the sky is blue: {r} {b}"
    );

    handle.lock().unwrap().sun_direction = Vec3::new(0.0, -0.2, 1.0).normalize();
    let image = renderer
        .render_to_image(&gpu, SIZE, SIZE, &camera(), &DrawList::default(), DT)
        .unwrap();
    let [r, _, _, _] = pixel(&image, SIZE / 2, SIZE / 2);
    assert!(r < 60, "a sun below the horizon draws no disc: {r}");
}

#[test]
fn hemisphere_ambient_lights_up_facing_faces_from_the_sky_and_down_facing_from_the_ground() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = Renderer::new(&gpu, TextureFormat::Rgba8UnormSrgb);
    renderer.settings.hdr.fixed_exposure = Some(1.0);
    renderer.settings.fog_density = 0.0;
    renderer.settings.sun_color = Vec3::ZERO;
    renderer.settings.hemisphere = Some(a3_render::HemisphereAmbient {
        sky: Vec3::new(0.0, 0.0, 1.0),
        mid: Vec3::new(0.0, 0.0, 0.0),
        ground: Vec3::new(1.0, 0.0, 0.0),
    });
    let cube = renderer.upload_mesh(&gpu, &MeshData::cuboid(Vec3::splat(2.0)));
    let cam = camera();
    let mut draws = DrawList::default();
    // One cube above the eye (we see its bottom), one below (we see its top).
    draws.mesh(MeshDraw::at(
        cube,
        cam.position + DVec3::new(0.0, 3.0, 4.0),
        [1.0; 4],
    ));
    draws.mesh(MeshDraw::at(
        cube,
        cam.position + DVec3::new(0.0, -3.0, 4.0),
        [1.0; 4],
    ));
    let image = renderer
        .render_to_image(&gpu, SIZE, SIZE, &cam, &draws, DT)
        .unwrap();
    let [r, _, b, _] = pixel(&image, SIZE / 2, 6);
    assert!(r > b, "bottom face lit from the ground: {r} {b}");
    let [r, _, b, _] = pixel(&image, SIZE / 2, SIZE - 7);
    assert!(b > r, "top face lit from the sky: {r} {b}");
}

#[test]
fn height_fog_is_thicker_low_down() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = Renderer::new(&gpu, TextureFormat::Rgba8UnormSrgb);
    renderer.settings.hdr.fixed_exposure = Some(1.0);
    renderer.settings.sun_color = Vec3::ZERO;
    renderer.settings.hemisphere = Some(a3_render::HemisphereAmbient {
        sky: Vec3::ZERO,
        mid: Vec3::ZERO,
        ground: Vec3::ZERO,
    });
    renderer.settings.sky_horizon = Vec3::ONE;
    renderer.settings.fog_density = 0.02;
    renderer.settings.fog_decay = 0.05;
    let cube = renderer.upload_mesh(&gpu, &MeshData::cuboid(Vec3::new(40.0, 40.0, 1.0)));
    let mut draws = DrawList::default();
    let render = |renderer: &mut Renderer, draws: &mut DrawList, height: f64| {
        let cam = Camera {
            position: DVec3::new(0.0, height, 0.0),
            ..Camera::default()
        };
        draws.clear();
        draws.mesh(MeshDraw::at(
            cube,
            cam.position + DVec3::new(0.0, 0.0, 100.0),
            [1.0; 4],
        ));
        let image = renderer
            .render_to_image(&gpu, SIZE, SIZE, &cam, draws, DT)
            .unwrap();
        pixel(&image, SIZE / 2, SIZE / 2)[0]
    };
    let low = render(&mut renderer, &mut draws, 0.0);
    let high = render(&mut renderer, &mut draws, 200.0);
    assert!(low > high + 50, "fog at sea level {low} vs 200 m {high}");
}

#[test]
fn linear_fog_hides_everything_past_the_fog_end() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = Renderer::new(&gpu, TextureFormat::Rgba8UnormSrgb);
    renderer.settings.hdr.fixed_exposure = Some(1.0);
    renderer.settings.fog_density = 0.0;
    renderer.settings.sun_color = Vec3::ZERO;
    renderer.settings.hemisphere = Some(a3_render::HemisphereAmbient {
        sky: Vec3::ZERO,
        mid: Vec3::ZERO,
        ground: Vec3::ZERO,
    });
    renderer.settings.sky_horizon = Vec3::ONE;
    renderer.settings.fog_start = 20.0;
    renderer.settings.fog_end = 50.0;
    let cube = renderer.upload_mesh(&gpu, &MeshData::cuboid(Vec3::new(40.0, 40.0, 1.0)));
    let cam = camera();
    let mut draws = DrawList::default();
    draws.mesh(MeshDraw::at(
        cube,
        cam.position + DVec3::new(0.0, 0.0, 100.0),
        [1.0; 4],
    ));
    let image = renderer
        .render_to_image(&gpu, SIZE, SIZE, &cam, &draws, DT)
        .unwrap();
    let [r, _, _, _] = pixel(&image, SIZE / 2, SIZE / 2);
    assert!(r > 200, "past the fog end only fog is left: {r}");
}
