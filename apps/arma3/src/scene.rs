//! The debug scene: free-fly camera over a procedural ground grid with a few test meshes, or
//! over a World's terrain when one is loaded (`--world`).

use std::sync::{Arc, Mutex};

use a3_input::{ActionMap, InputCode, InputState, MouseAxis, actions};
use a3_landscape_render::{HeightField, TerrainRenderer, TerrainStats};
use a3_render::texture::{bc1_block, rgb565};
use a3_render::{
    Camera, ColorSpace, DrawList, FreeFlyController, FreeFlyInput, Gpu, MeshData, MeshDraw, MeshId,
    Renderer, TextureData, TextureFormat, TextureId,
};
use glam::{DAffine3, DQuat, DVec3, Vec3};

use crate::models::{ModelSpec, Orbit, stats_line};
use crate::world::{CameraSpec, LoadedWorld};
use a3_render_models::{ModelFeature, ModelStats};

/// Closest the free camera gets to the terrain surface, in metres.
const MIN_ALTITUDE: f64 = 1.5;

/// A loaded World's terrain as the scene sees it.
struct WorldView {
    name: String,
    heights: HeightField,
    stats: Arc<Mutex<TerrainStats>>,
}

/// Centre of the test area: far from the world origin, like the middle of a 30 km terrain, so
/// camera-relative rendering is always exercised.
pub const WORLD_CENTRE: DVec3 = DVec3::new(15_000.0, 0.0, 15_000.0);

/// Smoothed frame-rate measurement for the overlay.
#[derive(Debug, Default)]
pub struct FpsCounter {
    accumulated: f64,
    frames: u32,
    fps: f64,
    frame_ms: f64,
}

impl FpsCounter {
    /// Add one frame of `real_dt` seconds; the average refreshes every half second.
    pub fn frame(&mut self, real_dt: f64) {
        self.accumulated += real_dt;
        self.frames += 1;
        if self.fps == 0.0 && real_dt > 0.0 {
            // Show something before the first full averaging window.
            self.fps = 1.0 / real_dt;
            self.frame_ms = 1000.0 * real_dt;
        }
        if self.accumulated >= 0.5 {
            self.fps = f64::from(self.frames) / self.accumulated;
            self.frame_ms = 1000.0 * self.accumulated / f64::from(self.frames);
            self.accumulated = 0.0;
            self.frames = 0;
        }
    }

    pub fn fps(&self) -> f64 {
        self.fps
    }

    pub fn frame_ms(&self) -> f64 {
        self.frame_ms
    }
}

/// Turn rate of the `cameraLook*` actions at full deflection, radians per second.
const KEY_LOOK_RATE: f64 = 1.5;

/// Free-camera input: movement from user actions; look from raw mouse motion (when the mouse is
/// captured, like RV's free camera) plus the `cameraLook*` actions (keys, sticks).
/// `look_sensitivity` is the controller's radians per look unit; `dt` the frame time.
pub fn free_fly_input(
    map: &ActionMap,
    input: &InputState,
    mouse_look: bool,
    look_sensitivity: f32,
    dt: f64,
) -> FreeFlyInput {
    let v = |name| map.value(input, name);
    let mut out = FreeFlyInput {
        strafe: v(actions::CAMERA_MOVE_RIGHT) - v(actions::CAMERA_MOVE_LEFT),
        lift: v(actions::CAMERA_MOVE_UP) - v(actions::CAMERA_MOVE_DOWN),
        forward: v(actions::CAMERA_MOVE_FORWARD) - v(actions::CAMERA_MOVE_BACKWARD),
        speed_multiplier: 1.0,
        ..FreeFlyInput::default()
    };
    // Convert a rate into look units for this frame.
    let key_units = (KEY_LOOK_RATE * dt) as f32 / look_sensitivity.max(f32::EPSILON);
    out.yaw = (v(actions::CAMERA_LOOK_RIGHT) - v(actions::CAMERA_LOOK_LEFT)) * key_units;
    out.pitch = (v(actions::CAMERA_LOOK_UP) - v(actions::CAMERA_LOOK_DOWN)) * key_units;
    if mouse_look {
        let m = |axis| input.value(InputCode::MouseAxis(axis));
        out.yaw += m(MouseAxis::Right) - m(MouseAxis::Left);
        out.pitch += m(MouseAxis::Up) - m(MouseAxis::Down);
    }
    if map.is_active(input, actions::CAMERA_MOVE_TURBO1) {
        out.speed_multiplier *= 5.0;
    }
    if map.is_active(input, actions::CAMERA_MOVE_TURBO2) {
        out.speed_multiplier *= 20.0;
    }
    out
}

struct SceneAssets {
    cube: MeshId,
    sphere: MeshId,
    ground: MeshId,
    ground_texture: TextureId,
    crate_texture: TextureId,
}

/// Everything the debug view shows, independent of window or offscreen output.
pub struct DebugScene {
    pub camera: Camera,
    pub controller: FreeFlyController,
    pub actions: ActionMap,
    assets: Option<SceneAssets>,
    world: Option<WorldView>,
    /// Placed objects of the World, or the model viewer's model.
    models: Option<ModelFeature>,
    orbit: Option<Orbit>,
    sim_time: f64,
}

impl DebugScene {
    pub fn new() -> DebugScene {
        DebugScene {
            camera: Camera {
                position: WORLD_CENTRE + DVec3::new(-12.0, 6.0, -40.0),
                yaw: 0.15,
                pitch: -0.08,
                ..Camera::default()
            },
            controller: FreeFlyController::default(),
            actions: actions::default_map(),
            assets: None,
            world: None,
            models: None,
            orbit: None,
            sim_time: 0.0,
        }
    }

    /// Show `world`'s terrain instead of the test meshes and place the camera: at `spec`, or
    /// above the world's `centerPosition`.
    pub fn load_world(
        &mut self,
        gpu: &Gpu,
        renderer: &mut Renderer,
        world: LoadedWorld,
        spec: Option<CameraSpec>,
    ) {
        let terrain = TerrainRenderer::new(gpu, renderer, &world.landscape, Some(world.reader));
        if let Some(objects) = world.objects {
            self.models = Some(objects.attach(gpu, renderer));
        }
        let stats = terrain.stats();
        renderer.add_feature(Box::new(terrain));
        let heights = world.landscape.heights;
        let spec = spec.unwrap_or(CameraSpec {
            east: world.centre.x,
            north: world.centre.z,
            altitude: 400.0,
            heading: 30.0,
            pitch: -15.0,
        });
        let ground = f64::from(heights.sample(spec.east as f32, spec.north as f32)).max(0.0);
        self.camera = Camera {
            position: DVec3::new(spec.east, ground + spec.altitude, spec.north),
            yaw: spec.heading.to_radians(),
            pitch: spec.pitch.to_radians(),
            ..Camera::default()
        };
        // Terrain distances: fly faster than in the test scene.
        self.controller.speed = 60.0;
        self.world = Some(WorldView {
            name: world.name,
            heights,
            stats,
        });
    }

    /// Terrain statistics of the loaded World.
    /// Show one model with an orbit camera (the model viewer).
    pub fn load_model(
        &mut self,
        gpu: &Gpu,
        renderer: &mut Renderer,
        vfs: a3_vfs::Vfs,
        spec: ModelSpec,
    ) {
        let models = crate::models::model_feature(gpu, renderer, vfs, 4096);
        self.orbit = Some(Orbit::new(&models, spec, WORLD_CENTRE));
        self.models = Some(models);
    }

    /// Whether models or textures are still loading.
    pub fn models_loading(&self) -> bool {
        self.models.as_ref().is_some_and(|m| !m.lock().is_idle())
    }

    /// Model renderer statistics of the last frame.
    pub fn model_stats(&self) -> Option<ModelStats> {
        self.models.as_ref().map(|m| m.lock().stats())
    }

    pub fn terrain_stats(&self) -> Option<TerrainStats> {
        let world = self.world.as_ref()?;
        world.stats.lock().ok().map(|s| *s)
    }

    /// Upload meshes and textures.
    pub fn load(&mut self, gpu: &Gpu, renderer: &mut Renderer) {
        let ground_texture = renderer
            .upload_texture(
                gpu,
                &checker_rgba8(256, 8, [92, 110, 72, 255], [104, 124, 82, 255]),
                ColorSpace::Srgb,
            )
            .expect("generated checker is valid");
        let crate_data = if Renderer::supports_bc(gpu) {
            checker_bc1(64)
        } else {
            checker_rgba8(64, 4, [200, 140, 40, 255], [60, 40, 20, 255])
        };
        let crate_texture = renderer
            .upload_texture(gpu, &crate_data, ColorSpace::Srgb)
            .expect("generated crate texture is valid");
        self.assets = Some(SceneAssets {
            cube: renderer.upload_mesh(gpu, &MeshData::cuboid(Vec3::ONE)),
            sphere: renderer.upload_mesh(gpu, &MeshData::uv_sphere(1.0, 32, 16)),
            ground: renderer.upload_mesh(gpu, &MeshData::ground_plane(4_000.0, 400.0)),
            ground_texture,
            crate_texture,
        });
    }

    /// Advance the scene by one frame.
    pub fn update(&mut self, input: &InputState, mouse_look: bool, dt: f64) {
        self.sim_time += dt;
        let fly = free_fly_input(
            &self.actions,
            input,
            mouse_look,
            self.controller.look_sensitivity,
            dt,
        );
        if let (Some(orbit), Some(models)) = (&mut self.orbit, &self.models) {
            let look = self.controller.look_sensitivity;
            orbit.update(&mut self.camera, &fly, look, dt, models);
            return;
        }
        self.controller.update(&mut self.camera, &fly, dt);
        if let Some(world) = &self.world {
            let p = &mut self.camera.position;
            let ground = f64::from(world.heights.sample(p.x as f32, p.z as f32));
            p.y = p.y.max(ground.max(0.0) + MIN_ALTITUDE);
        }
    }

    /// Fill `draws` with this frame's meshes and lines.
    pub fn draw(&self, draws: &mut DrawList) {
        if let (Some(orbit), Some(models)) = (&self.orbit, &self.models) {
            orbit.draw(draws, models);
            return;
        }
        if self.world.is_some() {
            // The terrain draws itself as a render feature.
            return;
        }
        let Some(a) = &self.assets else { return };
        let c = WORLD_CENTRE;
        draws.mesh(MeshDraw {
            texture: Some(a.ground_texture),
            ..MeshDraw::at(a.ground, c, [1.0; 4])
        });

        let boxed = |pos: DVec3, size: DVec3, rot: DQuat| {
            DAffine3::from_scale_rotation_translation(size, rot, pos)
        };
        let spin = DQuat::from_rotation_y(self.sim_time * 0.8);
        let solid = |mesh, transform, color| MeshDraw {
            mesh,
            texture: None,
            transform,
            color,
            transparent: false,
        };
        draws.mesh(solid(
            a.cube,
            boxed(
                c + DVec3::new(-6.0, 1.0, 0.0),
                DVec3::splat(2.0),
                DQuat::IDENTITY,
            ),
            [0.8, 0.15, 0.1, 1.0],
        ));
        draws.mesh(solid(
            a.cube,
            boxed(c + DVec3::new(0.0, 1.5, 4.0), DVec3::splat(3.0), spin),
            [0.15, 0.6, 0.2, 1.0],
        ));
        draws.mesh(solid(
            a.sphere,
            boxed(
                c + DVec3::new(6.0, 2.0, 0.0),
                DVec3::splat(2.0),
                DQuat::IDENTITY,
            ),
            [0.2, 0.35, 0.9, 1.0],
        ));
        draws.mesh(MeshDraw {
            texture: Some(a.crate_texture),
            ..solid(
                a.cube,
                boxed(
                    c + DVec3::new(-2.0, 1.0, -6.0),
                    DVec3::splat(2.0),
                    DQuat::IDENTITY,
                ),
                [1.0; 4],
            )
        });
        draws.mesh(MeshDraw {
            transparent: true,
            ..solid(
                a.cube,
                boxed(
                    c + DVec3::new(3.0, 1.0, -5.0),
                    DVec3::new(2.0, 2.0, 0.3),
                    spin,
                ),
                [0.9, 0.9, 1.0, 0.35],
            )
        });

        // Towers at increasing distances north: depth precision out to view-distance range.
        for (i, distance) in [250.0, 1_000.0, 3_000.0, 6_000.0, 12_000.0]
            .iter()
            .enumerate()
        {
            let height = 20.0 + distance * 0.02;
            draws.mesh(solid(
                a.cube,
                boxed(
                    c + DVec3::new(-120.0 + 60.0 * i as f64, height * 0.5, *distance),
                    DVec3::new(10.0 + distance * 0.004, height, 10.0 + distance * 0.004),
                    DQuat::IDENTITY,
                ),
                [0.85, 0.75, 0.5, 1.0],
            ));
        }

        let lines = &mut draws.lines;
        let cam = self.camera.position;
        lines.grid(cam, 0.02, 10.0, 40, [0.25, 0.3, 0.22, 1.0]);
        lines.grid(cam, 0.03, 100.0, 40, [0.45, 0.5, 0.35, 1.0]);
        lines.axes(c + DVec3::new(0.0, 0.05, 0.0), 5.0);
        lines.wire_sphere(c + DVec3::new(6.0, 2.0, 0.0), 1.3, 32, [1.0, 1.0, 0.2, 1.0]);
        lines.aabb(
            c + DVec3::new(-3.1, -0.05, -7.1),
            c + DVec3::new(-0.9, 2.1, -4.9),
            [1.0, 0.5, 0.0, 1.0],
        );
    }

    /// Debug overlay text lines.
    pub fn overlay(
        &self,
        draws: &mut DrawList,
        fps: &FpsCounter,
        adapter: &str,
        captured: bool,
        keys: &str,
    ) {
        let p = self.camera.position;
        let heading = self.camera.yaw.to_degrees().rem_euclid(360.0);
        let white = [1.0, 1.0, 1.0, 1.0];
        let dim = [0.8, 0.85, 0.9, 1.0];
        draws.text(
            8.0,
            8.0,
            2.0,
            white,
            format!("A3-RUST  FPS {:.0}  {:.2} MS", fps.fps(), fps.frame_ms()),
        );
        draws.text(
            8.0,
            30.0,
            2.0,
            dim,
            format!(
                "POS {:.1} {:.1} {:.1}  DIR {:03.0}  {}",
                p.x, p.y, p.z, heading, adapter
            ),
        );
        let help = if captured {
            "WASD Q Z MOVE  SHIFT/CTRL FAST  MOUSE LOOK  TAB RELEASE  ESC QUIT"
        } else {
            "WASD Q Z MOVE  SHIFT/CTRL FAST  CLICK TO LOOK  ESC QUIT"
        };
        draws.text(8.0, 52.0, 2.0, dim, help);
        draws.text(8.0, 74.0, 2.0, dim, format!("KEYS {keys}"));
        if let Some(models) = &self.models {
            let line = match &self.orbit {
                Some(orbit) => format!(
                    "{}  {}",
                    orbit.describe(models),
                    stats_line(&models.lock().stats())
                ),
                None => stats_line(&models.lock().stats()),
            };
            draws.text(8.0, 118.0, 2.0, dim, line);
        }
        if let (Some(world), Some(stats)) = (&self.world, self.terrain_stats()) {
            let ground = world.heights.sample(p.x as f32, p.z as f32);
            draws.text(
                8.0,
                96.0,
                2.0,
                dim,
                format!(
                    "{}  ATL {:.0}  NODES {}  TRIS {}K  TILES {} (+{})",
                    world.name.to_uppercase(),
                    p.y - f64::from(ground.max(0.0)),
                    stats.nodes,
                    stats.triangles / 1000,
                    stats.resident_tiles,
                    stats.pending_tiles
                ),
            );
        }
    }
}

/// RGBA8 checkerboard with a full box-filtered mip chain.
pub fn checker_rgba8(size: u32, cells: u32, a: [u8; 4], b: [u8; 4]) -> TextureData {
    let cell = (size / cells).max(1);
    let mut level: Vec<u8> = (0..size * size)
        .flat_map(|i| {
            let (x, y) = (i % size, i / size);
            if ((x / cell) + (y / cell)) % 2 == 0 {
                a
            } else {
                b
            }
        })
        .collect();
    let mut mips = vec![level.clone()];
    let mut s = size;
    while s > 1 {
        let half = s / 2;
        let mut next = vec![0u8; (half * half * 4) as usize];
        for y in 0..half {
            for x in 0..half {
                for ch in 0..4 {
                    let at = |dx: u32, dy: u32| {
                        u32::from(level[(((2 * y + dy) * s + 2 * x + dx) * 4 + ch) as usize])
                    };
                    next[((y * half + x) * 4 + ch) as usize] =
                        ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4) as u8;
                }
            }
        }
        mips.push(next.clone());
        level = next;
        s = half;
    }
    TextureData {
        format: TextureFormat::Rgba8,
        width: size,
        height: size,
        mips,
    }
}

/// BC1 "crate" texture: 4x4-block checker of two wood tones with a dark border, plus mips.
pub fn checker_bc1(size: u32) -> TextureData {
    let light = rgb565(200, 140, 40);
    let dark = rgb565(60, 40, 20);
    let mut mips = Vec::new();
    let mut s = size;
    while s >= 4 {
        let blocks = s / 4;
        let mut bytes = Vec::with_capacity((blocks * blocks * 8) as usize);
        for by in 0..blocks {
            for bx in 0..blocks {
                let border = bx == 0 || by == 0 || bx == blocks - 1 || by == blocks - 1;
                let index = if border || (bx + by) % 2 == 1 { 1 } else { 0 };
                bytes.extend_from_slice(&bc1_block(light, dark, [[index; 4]; 4]));
            }
        }
        mips.push(bytes);
        s /= 2;
    }
    TextureData {
        format: TextureFormat::Bc1,
        width: size,
        height: size,
        mips,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_input::Dik;

    #[test]
    fn generated_textures_are_valid() {
        assert_eq!(checker_rgba8(64, 4, [0; 4], [255; 4]).validate(), Ok(()));
        assert_eq!(checker_rgba8(64, 4, [0; 4], [255; 4]).mips.len(), 7);
        assert_eq!(checker_bc1(64).validate(), Ok(()));
    }

    #[test]
    fn camera_actions_drive_free_fly_input() {
        let map = actions::default_map();
        let mut input = InputState::new();
        input.press(InputCode::Key(Dik::W));
        input.press(InputCode::Key(Dik::LSHIFT));
        input.mouse_motion(4.0, 0.0);
        let fly = free_fly_input(&map, &input, false, 0.01, 0.1);
        assert_eq!(
            (fly.forward, fly.yaw, fly.speed_multiplier),
            (1.0, 0.0, 5.0)
        );
        let fly = free_fly_input(&map, &input, true, 0.01, 0.1);
        assert_eq!(fly.yaw, 4.0, "raw mouse counts while captured");
        // Numpad 6 turns right at KEY_LOOK_RATE: 1.5 rad/s * 0.1 s / 0.01 rad per unit.
        input.press(InputCode::Key(Dik::NUMPAD6));
        let fly = free_fly_input(&map, &input, false, 0.01, 0.1);
        assert!((fly.yaw - 15.0).abs() < 1e-4, "{}", fly.yaw);
    }

    #[test]
    fn fps_counter_averages_over_half_a_second() {
        let mut f = FpsCounter::default();
        for _ in 0..31 {
            f.frame(1.0 / 60.0);
        }
        assert!((f.fps() - 60.0).abs() < 1e-6);
        assert!((f.frame_ms() - 16.666).abs() < 0.01);
    }
}
