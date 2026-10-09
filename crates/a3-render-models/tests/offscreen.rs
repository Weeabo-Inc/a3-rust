//! Draws a synthetic model through the model feature offscreen and checks pixels. Skips when no
//! GPU adapter (not even a software one) is available.

use a3_render::{Camera, DrawList, Gpu, MeshData, Renderer};
use a3_render_models::prepare::{ModelVertex, PreparedLod, PreparedSection};
use a3_render_models::{
    MaterialDesc, ModelFeature, ModelRenderer, PlacedObject, PreparedModel, TextureOptions,
};
use glam::{DAffine3, DVec3, Vec3};

const SIZE: u32 = 64;
const DT: f32 = 1.0 / 60.0;

/// A 2 m cube with one procedural colour.
fn cube(color: &str) -> PreparedModel {
    let mesh = MeshData::cuboid(Vec3::splat(2.0));
    let vertices = mesh
        .vertices
        .iter()
        .map(|v| ModelVertex {
            position: v.position,
            normal: v.normal,
            uv0: v.uv,
            uv1: v.uv,
            tangent: v.tangent,
        })
        .collect();
    let faces = mesh.indices.len() as u32 / 3;
    PreparedModel {
        lods: vec![PreparedLod {
            resolution: 1.0,
            vertices,
            sections: vec![PreparedSection {
                indices: 0..mesh.indices.len() as u32,
                material: MaterialDesc::new(None, Some(color)),
            }],
            indices: mesh.indices,
            proxies: Vec::new(),
            skin: None,
            radius: 3f32.sqrt(),
            faces,
            polygons: faces,
        }],
        lod_density_coef: 1.0,
        draw_importance: 1.0,
        radius: 3f32.sqrt(),
        bbox: (Vec3::splat(-1.0), Vec3::splat(1.0)),
    }
}

fn pixel(image: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [image[i], image[i + 1], image[i + 2], image[i + 3]]
}

#[test]
fn a_placed_model_is_drawn_with_its_texture() {
    let gpu = match Gpu::headless() {
        Ok(gpu) => gpu,
        Err(e) if std::env::var_os("A3_REQUIRE_GPU").is_some_and(|v| !v.is_empty() && v != "0") => {
            panic!("A3_REQUIRE_GPU is set but no GPU adapter exists: {e}")
        }
        Err(e) => {
            eprintln!("skipping: no GPU adapter ({e})");
            return;
        }
    };
    let mut renderer = Renderer::new(&gpu, a3_render::wgpu::TextureFormat::Rgba8UnormSrgb);
    let models = ModelFeature::new(ModelRenderer::new(
        &gpu.device,
        renderer.frame_layout(),
        a3_vfs::Vfs::new(),
        TextureOptions {
            max_size: 256,
            bc_supported: Renderer::supports_bc(&gpu),
        },
        1,
    ));
    renderer.add_feature(Box::new(models.clone()));

    // Far from the world origin, so camera-relative rendering is exercised.
    let camera = Camera {
        position: DVec3::new(20_000.0, 50.0, 20_000.0),
        ..Camera::default()
    };
    {
        let mut m = models.lock();
        let red = m.insert_prepared("test\\red_cube.p3d", cube("#(argb,8,8,3)color(1,0,0,1,CO)"));
        m.add_dynamic(PlacedObject {
            model: red,
            transform: DAffine3::from_translation(camera.position + DVec3::new(0.0, 0.0, 6.0)),
        });
        // Behind the camera: culled, never drawn.
        let blue = m.insert_prepared(
            "test\\blue_cube.p3d",
            cube("#(argb,8,8,3)color(0,0,1,1,CO)"),
        );
        m.add_dynamic(PlacedObject {
            model: blue,
            transform: DAffine3::from_translation(camera.position - DVec3::new(0.0, 0.0, 6.0)),
        });
    }

    let draws = DrawList::default();
    let mut image = Vec::new();
    for _ in 0..500 {
        image = renderer
            .render_to_image(&gpu, SIZE, SIZE, &camera, &draws, DT)
            .expect("render");
        if models.lock().is_idle() {
            break;
        }
    }
    assert!(models.lock().is_idle(), "texture loading did not finish");
    let image = renderer
        .render_to_image(&gpu, SIZE, SIZE, &camera, &draws, DT)
        .map(|i| if i.is_empty() { image } else { i })
        .expect("render");

    let [r, g, b, _] = pixel(&image, SIZE / 2, SIZE / 2);
    assert!(
        r > 60 && r > 2 * g && r > 2 * b,
        "centre should be the red cube: {r} {g} {b}"
    );
    let [r, g, b, _] = pixel(&image, SIZE / 2, 0);
    assert!(b > r, "top edge should be sky: {r} {g} {b}");
    let stats = models.lock().stats();
    assert_eq!(stats.instances, 1, "the cube behind the camera is culled");
    assert_eq!(stats.models_ready, 2);
}
