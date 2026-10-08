//! Skinned models through the model feature offscreen: the bone palette moves vertices and the
//! rest pose (identity matrices) draws exactly like the unskinned model. Every comparison is
//! made inside one frame — the sky and the exposure adapt between frames — so the tests stage
//! both the posed and the reference instance side by side. Skips when no GPU adapter (not even
//! a software one) is available.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

use a3_render::{Camera, DrawList, Gpu, MeshData, Renderer};
use a3_render_models::prepare::{ModelVertex, PreparedLod, PreparedSection};
use a3_render_models::{
    MaterialDesc, ModelFeature, ModelId, ModelRenderer, PlacedObject, PreparedModel, SkinData,
    SkinVertex, TextureOptions,
};
use glam::{Affine3A, DAffine3, DVec3, Vec3};

const SIZE: u32 = 64;
const DT: f32 = 1.0 / 60.0;
/// The models are placed 6 m in front of the camera.
const DISTANCE: f32 = 6.0;
/// B_Soldier_F's model: the class inherits the BLUFOR soldier.
const SOLDIER: &str = r"a3\characters_f\blufor\b_soldier_01.p3d";

/// One device shared by the tests of this binary, used in turns: several headless devices in
/// threads at once crash some Vulkan loaders.
fn gpu() -> Option<MutexGuard<'static, Gpu>> {
    static GPU: OnceLock<Option<Mutex<Gpu>>> = OnceLock::new();
    let require = std::env::var_os("A3_REQUIRE_GPU").is_some_and(|v| !v.is_empty() && v != "0");
    let gpu = GPU.get_or_init(|| match Gpu::headless() {
        Ok(gpu) => {
            eprintln!("adapter: {}", gpu.adapter_name());
            Some(Mutex::new(gpu))
        }
        Err(e) if require => panic!("A3_REQUIRE_GPU is set but no GPU adapter exists: {e}"),
        Err(e) => {
            eprintln!("skipping: no GPU adapter ({e})");
            None
        }
    });
    gpu.as_ref()
        .map(|m| m.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}

struct Harness {
    gpu: MutexGuard<'static, Gpu>,
    renderer: Renderer,
    models: ModelFeature,
    camera: Camera,
    draws: DrawList,
}

impl Harness {
    /// `None` when no GPU adapter (not even a software one) exists.
    fn new(vfs: a3_vfs::Vfs) -> Option<Harness> {
        let gpu = gpu()?;
        let mut renderer = Renderer::new(&gpu, a3_render::wgpu::TextureFormat::Rgba8UnormSrgb);
        let models = ModelFeature::new(ModelRenderer::new(
            &gpu.device,
            renderer.frame_layout(),
            vfs,
            TextureOptions {
                max_size: 256,
                bc_supported: Renderer::supports_bc(&gpu),
            },
            1,
        ));
        renderer.add_feature(Box::new(models.clone()));
        Some(Harness {
            gpu,
            renderer,
            models,
            // Far from the world origin, so camera-relative rendering is exercised.
            camera: Camera {
                position: DVec3::new(20_000.0, 50.0, 20_000.0),
                ..Camera::default()
            },
            draws: DrawList::default(),
        })
    }

    /// The world transform of a model `x` metres right of the camera's forward axis, `z` metres
    /// ahead.
    fn at(&self, model: ModelId, x: f64, z: f32) -> PlacedObject {
        PlacedObject {
            model,
            transform: DAffine3::from_translation(
                self.camera.position + DVec3::new(x, 0.0, f64::from(z)),
            ),
        }
    }

    /// The world transform of a model `x` metres right of the axis, at the test distance.
    fn at_x(&self, model: ModelId, x: f64) -> PlacedObject {
        self.at(model, x, DISTANCE)
    }

    /// How many image columns one metre spans `z` metres ahead.
    fn metres(&self, z: f32) -> f32 {
        SIZE as f32 / 2.0 / (z * self.camera.fov.top)
    }

    /// Render until every model and texture has loaded, then once more for a settled frame.
    fn render(&mut self) -> Vec<u8> {
        let mut image = Vec::new();
        for _ in 0..500 {
            image = self
                .renderer
                .render_to_image(&self.gpu, SIZE, SIZE, &self.camera, &self.draws, DT)
                .expect("render");
            if self.models.lock().is_idle() {
                break;
            }
        }
        assert!(self.models.lock().is_idle(), "loading did not finish");
        self.renderer
            .render_to_image(&self.gpu, SIZE, SIZE, &self.camera, &self.draws, DT)
            .map(|i| if i.is_empty() { image } else { i })
            .expect("render")
    }
}

fn pixel(image: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [image[i], image[i + 1], image[i + 2], image[i + 3]]
}

/// The lit colour of the test cubes: bright and clearly red.
fn is_red(p: [u8; 4]) -> bool {
    let [r, g, b, _] = p;
    let (r, g, b) = (u32::from(r), u32::from(g), u32::from(b));
    r > 60 && r > 2 * g && r > 2 * b
}

/// Where a model's red pixels are in one half of the frame.
#[derive(Debug)]
struct Red {
    /// min x, min y, max x, max y.
    bbox: (u32, u32, u32, u32),
    count: u32,
    /// The pixel at the bounding box's centre, which is inside the model.
    centre: [u8; 4],
}

impl Red {
    fn centre_x(&self) -> f32 {
        (self.bbox.0 + self.bbox.2) as f32 / 2.0
    }

    fn width(&self) -> u32 {
        self.bbox.2 - self.bbox.0 + 1
    }
}

/// The red pixels of the frame columns `x0..x1`.
fn red_in(image: &[u8], x0: u32, x1: u32) -> Option<Red> {
    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    let mut count = 0;
    for y in 0..SIZE {
        for x in x0..x1 {
            if is_red(pixel(image, x, y)) {
                count += 1;
                bbox = Some(match bbox {
                    None => (x, y, x, y),
                    Some((ax, ay, bx, by)) => (ax.min(x), ay.min(y), bx.max(x), by.max(y)),
                });
            }
        }
    }
    let bbox = bbox?;
    let centre = pixel(image, (bbox.0 + bbox.2) / 2, (bbox.1 + bbox.3) / 2);
    Some(Red {
        bbox,
        count,
        centre,
    })
}

/// A 2 m cube of one procedural colour, skinned to a two-bone skeleton: every vertex blends
/// both bones, half each. Palette slot 0 is Skeleton bone 0, slot 1 bone 1, slot 2 the identity
/// slot our [`SkinData`] appends after the Skeleton's bones.
fn skinned_cube(color: &str) -> PreparedModel {
    let mut model = plain_cube(color);
    let vertices = model.lods[0].vertices.len();
    model.lods[0].skin = Some(SkinData {
        vertices: vec![
            SkinVertex {
                bones: [0, 1, 2, 2],
                weights: [0.5, 0.5, 0.0, 0.0],
            };
            vertices
        ],
        palette_len: 3,
    });
    model
}

/// A 2 m cube of one procedural colour, unskinned.
fn plain_cube(color: &str) -> PreparedModel {
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
        }],
        lod_density_coef: 1.0,
        radius: 3f32.sqrt(),
        bbox: (Vec3::splat(-1.0), Vec3::splat(1.0)),
    }
}

/// The static cube and the skinned one draw the same picture at rest, in one frame: the bone
/// palettes are the identity, so the blend is the identity and the vertices do not move. The
/// two instances sit on opposite sides of the view axis; a cube draws the same silhouette
/// mirrored, so identical drawings mirror each other about the frame centre.
#[test]
fn a_skinned_model_in_its_rest_pose_draws_like_the_unskinned_one() {
    let Some(mut h) = Harness::new(a3_vfs::Vfs::new()) else {
        return;
    };
    let cube = {
        let mut m = h.models.lock();
        m.insert_prepared(
            "test\\cube.p3d",
            skinned_cube("#(argb,8,8,3)color(1,0,0,1,CO)"),
        )
    };
    {
        let mut m = h.models.lock();
        // Left: the model unskinned (the rest-pose vertices). Right: skinned at rest.
        m.add_dynamic(h.at_x(cube, -2.0));
        m.add_skinned(h.at_x(cube, 2.0), &[Affine3A::IDENTITY; 2]);
    }
    let image = h.render();

    let unskinned = red_in(&image, 0, SIZE / 2).expect("the unskinned cube is drawn");
    let skinned = red_in(&image, SIZE / 2, SIZE).expect("the skinned cube is drawn");
    assert!(
        close(mirror(unskinned.bbox), skinned.bbox, 1),
        "at rest the skinned cube draws what the unskinned one draws: \
         {unskinned:?} then {skinned:?}"
    );
    assert!(
        unskinned.count.abs_diff(skinned.count) <= 4,
        "the same cube: {unskinned:?} then {skinned:?}"
    );
    assert_eq!(
        unskinned.centre, skinned.centre,
        "the rest pose should shade like the unskinned model"
    );
}

/// Every vertex weighs bone 0 and bone 1 half each, so bone matrices of +1 m and -1 m along x
/// blend to the identity: the blended cube draws what the unskinned one draws.
#[test]
fn bone_weights_blend_between_their_matrices() {
    let Some(mut h) = Harness::new(a3_vfs::Vfs::new()) else {
        return;
    };
    let cube = {
        let mut m = h.models.lock();
        m.insert_prepared(
            "test\\cube.p3d",
            skinned_cube("#(argb,8,8,3)color(1,0,0,1,CO)"),
        )
    };
    {
        let mut m = h.models.lock();
        m.add_dynamic(h.at_x(cube, -2.0));
        m.add_skinned(
            h.at_x(cube, 2.0),
            &[
                Affine3A::from_translation(Vec3::new(1.0, 0.0, 0.0)),
                Affine3A::from_translation(Vec3::new(-1.0, 0.0, 0.0)),
            ],
        );
    }
    let image = h.render();

    let unskinned = red_in(&image, 0, SIZE / 2).expect("the unskinned cube is drawn");
    let blended = red_in(&image, SIZE / 2, SIZE).expect("the blended cube is drawn");
    assert!(
        close(mirror(unskinned.bbox), blended.bbox, 1),
        "a half-and-half blend of opposite offsets is the rest pose: \
         {unskinned:?} then {blended:?}"
    );
    assert!(
        unskinned.count.abs_diff(blended.count) <= 4,
        "the same cube: {unskinned:?} then {blended:?}"
    );
    assert_eq!(unskinned.centre, blended.centre);
}

/// Two palettes, one frame: the posed cube is drawn half a metre right of where its rest-pose
/// twin is, and the twin itself does not move when the other instance's palette changes.
#[test]
fn different_bone_palettes_draw_different_poses() {
    let Some(mut h) = Harness::new(a3_vfs::Vfs::new()) else {
        return;
    };
    let cube = {
        let mut m = h.models.lock();
        m.insert_prepared(
            "test\\cube.p3d",
            skinned_cube("#(argb,8,8,3)color(1,0,0,1,CO)"),
        )
    };
    let rest = [Affine3A::IDENTITY; 2];
    let shifted = [Affine3A::from_translation(Vec3::new(0.5, 0.0, 0.0)); 2];

    // Both cubes at rest.
    {
        let mut m = h.models.lock();
        m.add_skinned(h.at_x(cube, -1.5), &rest);
        m.add_skinned(h.at_x(cube, 1.5), &rest);
    }
    let image = h.render();
    let left_rest = red_in(&image, 0, SIZE / 2).expect("the left cube is drawn");
    let right_rest = red_in(&image, SIZE / 2, SIZE).expect("the right cube is drawn");

    // The right cube posed half a metre right; the left one keeps its rest palette.
    {
        let mut m = h.models.lock();
        m.clear_skinned();
        m.add_skinned(h.at_x(cube, -1.5), &rest);
        m.add_skinned(h.at_x(cube, 1.5), &shifted);
    }
    let image = h.render();
    let left_moved = red_in(&image, 0, SIZE / 2).expect("the left cube is drawn");
    let right_shifted = red_in(&image, SIZE / 2, SIZE).expect("the right cube is drawn");
    eprintln!("cube rest: {left_rest:?} {right_rest:?}, posed: {left_moved:?} {right_shifted:?}");

    // The two frames differ (the sky and the exposure drift), so the pose is measured against
    // the unposed twin, which stands still across the change.
    assert!(
        close(left_rest.bbox, left_moved.bbox, 1),
        "the left cube is not posed: {left_rest:?} then {left_moved:?}"
    );
    let expected = 0.5 * h.metres(DISTANCE);
    let measured = right_shifted.centre_x() - right_rest.centre_x();
    assert!(
        (measured - expected).abs() < 1.5,
        "the posed cube moved 0.5 m right: {measured} not {expected:?}"
    );
    assert!(
        right_rest.width().abs_diff(right_shifted.width()) <= 2,
        "a translation does not resize the cube: {right_rest:?} then {right_shifted:?}"
    );
}

/// B_Soldier_F, posed by its own Skeleton: the whole soldier translated half a metre right by
/// its bone matrices stands exactly that much further right than its rest-pose twin. Both are
/// in one frame, so the pose is measured against the same sky and the same exposure, and the
/// twins cancel the model's own asymmetry.
#[test]
fn a_real_soldier_follows_its_bone_palette() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    const SOLDIER_DISTANCE: f32 = 3.0;
    /// How far the soldiers stand to each side of the view axis: their silhouettes are wide
    /// enough that a narrower pair would reach across the frame's centre line.
    const SOLDIER_SIDE: f64 = 1.3;
    let vfs = a3_vfs::Vfs::new();
    vfs.mount_game(Path::new(&root), &[]);
    let model = a3_p3d::Model::from_bytes(&vfs.open(SOLDIER).expect("the soldier model opens"))
        .expect("the soldier model decodes");
    // Pose the palette through the model's own Skeleton, as a3-anim's `Pose::bones` does.
    let bones = model
        .skeleton
        .as_ref()
        .expect("a soldier is skinned")
        .bones
        .len();
    let Some(mut h) = Harness::new(vfs) else {
        return;
    };
    let soldier = {
        let mut m = h.models.lock();
        m.model(SOLDIER)
    };
    let rest: Vec<Affine3A> = vec![Affine3A::IDENTITY; bones];
    let shifted = vec![Affine3A::from_translation(Vec3::new(0.5, 0.0, 0.0)); bones];
    // Both soldiers at rest, a control twin on each side of the axis.
    {
        let mut m = h.models.lock();
        m.add_skinned(h.at(soldier, -SOLDIER_SIDE, SOLDIER_DISTANCE), &rest);
        m.add_skinned(h.at(soldier, SOLDIER_SIDE, SOLDIER_DISTANCE), &rest);
    }
    let image = h.render();
    let left_rest = soldier_in(&image, 0, SIZE / 2).expect("the left soldier is drawn");
    let right_rest = soldier_in(&image, SIZE / 2, SIZE).expect("the right soldier is drawn");

    // Now pose the right soldier's whole skeleton half a metre right; the left one keeps the
    // rest palette.
    {
        let mut m = h.models.lock();
        m.clear_skinned();
        m.add_skinned(h.at(soldier, -SOLDIER_SIDE, SOLDIER_DISTANCE), &rest);
        m.add_skinned(h.at(soldier, SOLDIER_SIDE, SOLDIER_DISTANCE), &shifted);
    }
    let image = h.render();
    let left_posed = soldier_in(&image, 0, SIZE / 2).expect("the left soldier is drawn");
    let right_posed = soldier_in(&image, SIZE / 2, SIZE).expect("the right soldier is drawn");
    eprintln!("soldier rest: {left_rest:?} {right_rest:?}, posed: {left_posed:?} {right_posed:?}");

    let stats = h.models.lock().stats();
    assert!(stats.instances >= 2, "both soldiers are drawn");
    assert_eq!(stats.models_failed, 0);
    // The two frames drift (the sky and the exposure move on), so the pose is measured against
    // the unposed twin, which stands still across the change.
    assert!(
        close(left_rest.bbox, left_posed.bbox, 1),
        "the left soldier is not posed: {left_rest:?} then {left_posed:?}"
    );
    // The palette translates every bone half a metre right, which at this distance is
    // half a metre of screen.
    let expected = 0.5 * h.metres(SOLDIER_DISTANCE);
    let measured = right_posed.centre_x() - right_rest.centre_x();
    assert!(
        (measured - expected).abs() < 2.5,
        "the palette moved the soldier 0.5 m right: {measured} not {expected:?}"
    );
    assert!(
        right_rest.width().abs_diff(right_posed.width()) <= 1,
        "a translation does not resize the soldier: {right_rest:?} then {right_posed:?}"
    );
}

/// Where the soldier is in one half of the frame: the pixels that are not the sky at their row.
/// The sky is a horizontal band, so the row's first pixel is the background for that row.
#[derive(Debug)]
struct Soldier {
    bbox: (u32, u32, u32, u32),
}

impl Soldier {
    fn centre_x(&self) -> f32 {
        (self.bbox.0 + self.bbox.2) as f32 / 2.0
    }

    fn width(&self) -> u32 {
        self.bbox.2 - self.bbox.0 + 1
    }
}

fn soldier_in(image: &[u8], x0: u32, x1: u32) -> Option<Soldier> {
    let differs = |a: [u8; 4], b: [u8; 4]| (0..3).any(|i| a[i].abs_diff(b[i]) > 12);
    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    for y in 0..SIZE {
        let sky = pixel(image, 0, y);
        for x in x0..x1 {
            if differs(pixel(image, x, y), sky) {
                bbox = Some(match bbox {
                    None => (x, y, x, y),
                    Some((ax, ay, bx, by)) => (ax.min(x), ay.min(y), bx.max(x), by.max(y)),
                });
            }
        }
    }
    bbox.map(|bbox| Soldier { bbox })
}

/// The bounding box of the same drawing on the other side of the frame centre.
fn mirror(bbox: (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
    let (x0, y0, x1, y1) = bbox;
    (SIZE - 1 - x1, y0, SIZE - 1 - x0, y1)
}

/// Whether two bounding boxes agree within `slack` pixels on every edge.
fn close(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32), slack: u32) -> bool {
    let (a, b) = ([a.0, a.1, a.2, a.3], [b.0, b.1, b.2, b.3]);
    (0..4).all(|i| a[i].abs_diff(b[i]) <= slack)
}
