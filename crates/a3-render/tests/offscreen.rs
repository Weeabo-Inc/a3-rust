//! Renders tiny offscreen frames and checks a few pixels. Skips when no GPU adapter (not even
//! a software one) is available.

mod common;

use common::gpu;

use a3_render::texture::{bc1_block, rgb565};
use a3_render::{
    Camera, ColorSpace, DrawList, MeshData, MeshDraw, Renderer, TextureData, TextureFormat,
};
use glam::{DVec3, Vec3};

const SIZE: u32 = 64;
const DT: f32 = 1.0 / 60.0;

fn pixel(image: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [image[i], image[i + 1], image[i + 2], image[i + 3]]
}

/// A camera far from the world origin, so camera-relative rendering is exercised.
fn camera() -> Camera {
    Camera {
        position: DVec3::new(20_000.0, 50.0, 20_000.0),
        ..Camera::default()
    }
}

#[test]
fn cube_in_front_of_camera_is_drawn_over_the_sky() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = Renderer::new(&gpu, a3_render::wgpu::TextureFormat::Rgba8UnormSrgb);
    let cube = renderer.upload_mesh(&gpu, &MeshData::cuboid(Vec3::splat(2.0)));
    let camera = camera();
    let mut draws = DrawList::default();
    draws.mesh(MeshDraw::at(
        cube,
        camera.position + DVec3::new(0.0, 0.0, 5.0),
        [1.0, 0.0, 0.0, 1.0],
    ));

    let image = renderer
        .render_to_image(&gpu, SIZE, SIZE, &camera, &draws, DT)
        .expect("render");

    let [r, g, b, _] = pixel(&image, SIZE / 2, SIZE / 2);
    assert!(
        r > 100 && g < 40 && b < 40,
        "centre should be the red cube: {r} {g} {b}"
    );
    let [r, g, b, _] = pixel(&image, SIZE / 2, 0);
    assert!(b > r && b > 100, "top edge should be sky: {r} {g} {b}");
}

#[test]
fn back_faces_are_culled() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = Renderer::new(&gpu, a3_render::wgpu::TextureFormat::Rgba8UnormSrgb);
    let cube = renderer.upload_mesh(&gpu, &MeshData::cuboid(Vec3::splat(2.0)));
    let camera = camera();
    let mut draws = DrawList::default();
    // Camera inside the cube: every face is seen from behind and culled, so we see sky.
    draws.mesh(MeshDraw::at(cube, camera.position, [1.0, 0.0, 0.0, 1.0]));
    let image = renderer
        .render_to_image(&gpu, SIZE, SIZE, &camera, &draws, DT)
        .expect("render");
    let [r, _, b, _] = pixel(&image, SIZE / 2, SIZE / 2);
    assert!(b > r, "inside a cube only sky is visible: r {r} b {b}");
}

#[test]
fn bc1_texture_is_sampled() {
    let Some(gpu) = gpu() else { return };
    if !Renderer::supports_bc(&gpu) {
        eprintln!("skipping: adapter lacks BC texture compression");
        return;
    }
    let mut renderer = Renderer::new(&gpu, a3_render::wgpu::TextureFormat::Rgba8UnormSrgb);
    // 8x8 BC1, two mips, every texel colour0 = pure green.
    let green = bc1_block(rgb565(0, 255, 0), 0, [[0; 4]; 4]);
    let texture = TextureData {
        format: TextureFormat::Bc1,
        width: 8,
        height: 8,
        mips: vec![green.repeat(4), green.to_vec()],
    };
    let texture = renderer
        .upload_texture(&gpu, &texture, ColorSpace::Srgb)
        .expect("upload");
    let cube = renderer.upload_mesh(&gpu, &MeshData::cuboid(Vec3::splat(2.0)));
    let camera = camera();
    let mut draws = DrawList::default();
    draws.mesh(MeshDraw {
        texture: Some(texture),
        ..MeshDraw::at(cube, camera.position + DVec3::new(0.0, 0.0, 5.0), [1.0; 4])
    });
    let image = renderer
        .render_to_image(&gpu, SIZE, SIZE, &camera, &draws, DT)
        .expect("render");
    let [r, g, b, _] = pixel(&image, SIZE / 2, SIZE / 2);
    assert!(
        g > 100 && r < 40 && b < 40,
        "centre should be green: {r} {g} {b}"
    );
}

#[test]
fn debug_text_and_lines_reach_the_output() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = Renderer::new(&gpu, a3_render::wgpu::TextureFormat::Rgba8UnormSrgb);
    let camera = camera();
    let mut draws = DrawList::default();
    // '#' glyph at (0, 0), scale 2: its row 2 is solid from x 0 to 9.
    draws.text(0.0, 0.0, 2.0, [1.0, 0.0, 1.0, 1.0], "#");
    // A vertical yellow line straight ahead.
    draws.lines.line(
        camera.position + DVec3::new(0.0, -5.0, 10.0),
        camera.position + DVec3::new(0.0, 5.0, 10.0),
        [1.0, 1.0, 0.0, 1.0],
    );
    let image = renderer
        .render_to_image(&gpu, SIZE, SIZE, &camera, &draws, DT)
        .expect("render");
    assert_eq!(pixel(&image, 3, 5), [255, 0, 255, 255], "text pixel");
    let centre_column: Vec<[u8; 4]> = (SIZE / 2 - 1..=SIZE / 2)
        .map(|x| pixel(&image, x, SIZE / 2 + 8))
        .collect();
    assert!(
        centre_column
            .iter()
            .any(|p| p[0] > 150 && p[1] > 150 && p[2] < 100),
        "line pixel near the centre: {centre_column:?}"
    );
}
