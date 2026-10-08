//! GPU tests of cascaded sun shadows. Skip without an adapter.

mod common;

use a3_render::{
    BloomSettings, Camera, DrawList, Gpu, HdrSettings, MeshData, MeshDraw, Renderer,
    wgpu::TextureFormat,
};
use common::gpu;
use glam::{DAffine3, DVec3, Vec3};

const SIZE: u32 = 96;

struct Shot {
    image: Vec<u8>,
    camera: Camera,
}

impl Shot {
    /// Pixel at the screen position of a world point.
    fn at(&self, world: DVec3) -> [u8; 4] {
        let clip = self.camera.view_projection(1.0) * self.camera.relative(world).extend(1.0);
        let ndc = clip.truncate() / clip.w;
        let x = ((ndc.x * 0.5 + 0.5) * SIZE as f32) as u32;
        let y = ((0.5 - ndc.y * 0.5) * SIZE as f32) as u32;
        let i = ((y.min(SIZE - 1) * SIZE + x.min(SIZE - 1)) * 4) as usize;
        [
            self.image[i],
            self.image[i + 1],
            self.image[i + 2],
            self.image[i + 3],
        ]
    }
}

/// A 2 m cube floating 3 m above a ground plane, sun almost overhead, camera looking down.
fn render(gpu: &Gpu, shadows: bool, sun: Vec3) -> Shot {
    let mut renderer = Renderer::new(gpu, TextureFormat::Rgba8UnormSrgb);
    // Fixed exposure and no bloom: only the shadow term changes between renders.
    renderer.settings.hdr = HdrSettings {
        fixed_exposure: Some(0.5),
        bloom: BloomSettings {
            enabled: false,
            ..BloomSettings::default()
        },
        ..HdrSettings::default()
    };
    renderer.settings.shadows.enabled = shadows;
    renderer.settings.sun_direction = sun.normalize();
    renderer.settings.fog_density = 0.0;
    let ground = renderer.upload_mesh(gpu, &MeshData::ground_plane(200.0, 1.0));
    let cube = renderer.upload_mesh(gpu, &MeshData::cuboid(Vec3::splat(2.0)));
    let origin = DVec3::new(10_000.0, 0.0, 10_000.0);
    let camera = Camera {
        position: origin + DVec3::new(0.0, 12.0, -12.0),
        pitch: -0.75,
        ..Camera::default()
    };
    let mut draws = DrawList::default();
    draws.mesh(MeshDraw::at(ground, origin, [0.8, 0.8, 0.8, 1.0]));
    draws.mesh(MeshDraw {
        transform: DAffine3::from_translation(origin + DVec3::new(0.0, 4.0, 0.0)),
        ..MeshDraw::at(cube, DVec3::ZERO, [0.8, 0.2, 0.2, 1.0])
    });
    let image = renderer
        .render_to_image(gpu, SIZE, SIZE, &camera, &draws, 1.0 / 60.0)
        .expect("render");
    Shot { image, camera }
}

fn brightness(p: [u8; 4]) -> u32 {
    u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])
}

#[test]
fn cube_casts_a_shadow_on_the_ground() {
    let Some(gpu) = gpu() else { return };
    let sun = Vec3::new(0.05, 1.0, 0.05);
    let lit = render(&gpu, false, sun);
    let shadowed = render(&gpu, true, sun);
    let origin = DVec3::new(10_000.0, 0.0, 10_000.0);
    let under = origin + DVec3::new(0.0, 0.0, 0.0);
    let away = origin + DVec3::new(-5.0, 0.0, 6.0);

    let (lit_under, shadow_under) = (brightness(lit.at(under)), brightness(shadowed.at(under)));
    assert!(
        shadow_under * 2 < lit_under,
        "ground under the cube should be in shadow: {shadow_under} vs lit {lit_under}"
    );
    let (lit_away, shadow_away) = (brightness(lit.at(away)), brightness(shadowed.at(away)));
    assert!(
        lit_away.abs_diff(shadow_away) <= 6,
        "open ground should be unaffected (no acne): {shadow_away} vs {lit_away}"
    );
}

#[test]
fn shadow_follows_the_sun_direction() {
    let Some(gpu) = gpu() else { return };
    // Low sun from the east: the shadow falls west of the cube.
    let shot = render(&gpu, true, Vec3::new(1.0, 1.0, 0.0));
    let origin = DVec3::new(10_000.0, 0.0, 10_000.0);
    let west = brightness(shot.at(origin + DVec3::new(-4.0, 0.0, 0.0)));
    let east = brightness(shot.at(origin + DVec3::new(4.0, 0.0, 0.0)));
    assert!(
        west * 2 < east,
        "shadow should fall west: west {west} vs east {east}"
    );
}
