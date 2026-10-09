//! The debug scene: free-fly camera over a procedural ground grid with a few test meshes, or
//! over a World's terrain when one is loaded (`--world`).

use std::sync::{Arc, Mutex};

use a3_input::{ActionMap, InputCode, InputState, MouseAxis, actions};
use a3_landscape_render::{HeightField, SeaConfig, SeaRenderer, TerrainRenderer, TerrainStats};
use a3_render::texture::{bc1_block, rgb565};
use a3_render::{
    Camera, ColorSpace, DrawList, FreeFlyController, FreeFlyInput, Gpu, MeshData, MeshDraw, MeshId,
    Renderer, TextureData, TextureFormat, TextureId,
};
use glam::{DAffine3, DQuat, DVec3, Vec3};

use crate::combat::Combat;
use crate::gear::{Loadout, ManGear, place_man};
use crate::man::ManAnimation;
use crate::models::{ModelSpec, Orbit, stats_line};
use crate::player::{CameraMode, Ground, Player, Stance, ground_lift};
use crate::world::{CameraSpec, LoadedWorld};
use a3_moves::Moves;
use a3_render_models::{ModelFeature, ModelId, ModelStats, PlacedObject};

/// Closest the free camera gets to the terrain surface, in metres.
const MIN_ALTITUDE: f64 = 1.5;

/// Closest the third-person camera boom gets to the terrain, in metres, so it does not end up
/// under a hill behind the player.
const THIRD_PERSON_CLEARANCE: f64 = 0.4;

/// A loaded World's terrain as the scene sees it.
struct WorldView {
    name: String,
    heights: HeightField,
    stats: Arc<Mutex<TerrainStats>>,
}

impl Ground for WorldView {
    fn height(&self, x: f64, z: f64) -> f32 {
        self.heights.sample(x as f32, z as f32)
    }
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

/// Raise `transform` by `lift` metres: where the Man stands, given the model's own ground offset.
///
/// The lift belongs to the placement, not to the Man's bones. A skinned bone's palette matrix is
/// the rest→posed frame, which is rotated by the pose; folding the lift into it (`bone * up`)
/// translates every posed bone along its own rotated up axis, dragging arms, head and hands up to
/// `lift` metres sideways out of the body (issue #247). Lifting the instance moves the whole posed
/// mesh uniformly instead, exactly as the unposed path has always done.
fn lift_placement(transform: DAffine3, lift: f32) -> DAffine3 {
    let mut transform = transform;
    transform.translation.y += f64::from(lift);
    transform
}

/// `transform` with the model-space point `ground` moved onto its origin: where a posed Man's
/// model goes so the ground under his feet is at the entity's position. Like the lift, a
/// placement offset, applied in the model's frame so it turns with the Man.
pub(crate) fn ground_placement(transform: DAffine3, ground: Vec3) -> DAffine3 {
    transform * DAffine3::from_translation(-ground.as_dvec3())
}

/// The player Man's own model renderer: `CfgVehicles >> B_Soldier_F >> model`, drawn at the
/// player's transform in third person and hidden in first person, like the engine hides the
/// body the eyes are in.
///
/// It is its own `ModelFeature` (rather than reusing the World objects' one) so the Man's single
/// model loads on a couple of threads and the view distance of the terrain's objects does not
/// decide whether he is drawn.
struct SoldierModel {
    feature: ModelFeature,
    model: ModelId,
    /// The Man's animation, when the game data has his Skeleton, the rest pivots and his Moves
    /// type. Behind a lock because placing is done from `draw`, which takes `&self`.
    man: Option<Mutex<ManAnimation>>,
    /// What he wears and holds, drawn on his pose.
    gear: Option<ManGear>,
    path: String,
}

impl SoldierModel {
    /// Register the renderer and start loading `path`. `moves` is the Moves type the Man's
    /// animation comes from; `None` (or a Moves type his rig cannot be posed with) draws him
    /// unposed.
    fn new(
        gpu: &Gpu,
        renderer: &mut Renderer,
        vfs: a3_vfs::Vfs,
        path: &str,
        moves: Option<Moves>,
        loadout: Option<&Loadout>,
    ) -> SoldierModel {
        let man = match moves {
            Some(moves) => match ManAnimation::load(&vfs, moves, path) {
                Some(mut man) => {
                    if let Some(face) = loadout.and_then(|l| l.head.as_ref()?.face.as_deref()) {
                        if !man.set_face(face) {
                            log::warn!("face animation {face} is not in the game data");
                        }
                    }
                    if loadout.is_some_and(|l| l.primary.is_some()) {
                        man.set_armed(true);
                    }
                    Some(Mutex::new(man))
                }
                None => {
                    log::warn!(
                        "cannot animate the player Man: {path}, its skeleton or its moves are \
                         missing from the game data; drawing him unposed"
                    );
                    None
                }
            },
            None => None,
        };
        let feature = crate::models::model_feature_with(gpu, renderer, vfs.clone(), 1024, 2);
        let (model, gear) = {
            let mut m = feature.lock();
            let model = m.model(path);
            m.preload(model);
            let gear = loadout.map(|l| ManGear::load(&vfs, &mut m, l, path));
            (model, gear)
        };
        SoldierModel {
            feature,
            model,
            man,
            gear,
            path: path.to_owned(),
        }
    }

    /// Whether the model's geometry is loaded and safe to place.
    fn ready(&self) -> bool {
        self.feature.lock().model_bounds(self.model).is_some()
    }

    /// Whether the loader still has work (used to hold a screenshot until the Man is in).
    fn loading(&self) -> bool {
        !self.feature.lock().is_idle()
    }

    /// Advance the Man's animation by one frame of the player's movement.
    fn advance(&self, player: &Player, dt: f32) {
        let Some(man) = &self.man else { return };
        match man.lock() {
            Ok(mut man) => man.advance(player.stance, player.motion, dt),
            Err(_) => log::warn!("the Man's animation lock is poisoned; he stands still"),
        }
    }

    /// Put the Man at `transform` for this frame, or take him out of the frame with `None`
    /// (first person, or before the model is ready).
    ///
    /// Posed, the Man's palette keeps his mesh in the model's own space with the ground under
    /// his feet at [`ManAnimation::ground`]; that point goes on `transform`
    /// ([`ground_placement`]). Unposed, his rest mesh is stored below its model origin, so he is
    /// lifted by [`ground_lift`](crate::player::ground_lift) instead. Either offset is part of
    /// the placement, never of the bone palette (see [`lift_placement`]).
    fn place(&self, transform: Option<DAffine3>) {
        let mut m = self.feature.lock();
        m.clear_dynamic();
        m.clear_skinned();
        let Some(transform) = transform else { return };
        if let Some(man) = &self.man {
            let posed = match man.lock() {
                Ok(man) => place_man(&mut m, self.model, &man, self.gear.as_ref(), transform),
                Err(_) => false,
            };
            if posed {
                return;
            }
        }
        let lift = m
            .model_box(self.model)
            .map_or(0.0, |(lowest, _)| ground_lift(lowest.as_dvec3()) as f32);
        let transform = lift_placement(transform, lift);
        m.add_dynamic(PlacedObject {
            model: self.model,
            transform,
        });
    }

    /// Overlay line describing the Man's model: its state, his Move, and what the renderer did
    /// with it last frame.
    fn describe(&self) -> String {
        let m = self.feature.lock();
        let stats = m.stats();
        let state = if m.model_failed(self.model) {
            "FAILED TO LOAD (SEE LOG)".to_owned()
        } else if let Some((_, lods)) = m.model_bounds(self.model) {
            format!("READY  {lods} LODS")
        } else {
            "LOADING, STAND-IN DRAWN".to_owned()
        };
        let move_ = match &self.man {
            Some(man) => match man.lock() {
                Ok(man) => format!("  MOVE {}", man.move_name()),
                Err(_) => "  ANIMATION LOCK POISONED".to_owned(),
            },
            None => "  NO ANIMATION".to_owned(),
        };
        format!(
            "MAN {}  {state}{move_}  INST {} DRAWS {}",
            self.path.to_uppercase().replace('\\', "/"),
            stats.instances,
            stats.draw_calls,
        )
    }
}

/// Everything the debug view shows, independent of window or offscreen output.
pub struct DebugScene {
    pub camera: Camera,
    pub controller: FreeFlyController,
    pub actions: ActionMap,
    /// The player Man, when playing (`--play`) instead of flying free.
    pub player: Option<Player>,
    assets: Option<SceneAssets>,
    world: Option<WorldView>,
    /// Placed objects of the World, or the model viewer's model.
    models: Option<ModelFeature>,
    /// The player Man's own model, when the game data has one.
    soldier: Option<SoldierModel>,
    orbit: Option<Orbit>,
    environment: Option<crate::environment::SceneEnvironment>,
    /// The player's in-game HUD, when playing over the game data.
    hud: Option<crate::hud::Hud>,
    /// SQF the HUD runs when play starts (`--exec`).
    pub hud_exec: Option<String>,
    sim_time: f64,
    /// The player's weapon and its shots, when playing.
    combat: Option<Combat>,
    /// The world's placed lights (lamps), discovered from `CfgVehicles >> Reflectors`. Not drawn
    /// yet; the shading pass is a separate change (`docs/re/render-materials.md` §3.4).
    lights: Vec<a3_environment::PlacedLight>,
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
            actions: crate::keys::client_default_map(),
            player: None,
            assets: None,
            world: None,
            models: None,
            soldier: None,
            orbit: None,
            environment: None,
            hud: None,
            hud_exec: None,
            sim_time: 0.0,
            combat: None,
            lights: Vec::new(),
        }
    }

    /// Show `world`'s terrain instead of the test meshes and place the camera: at `spec`, or
    /// above the world's `centerPosition`. With `play` set, a player Man spawns on the terrain
    /// at the same spot instead, seen through the given camera mode.
    pub fn load_world(
        &mut self,
        gpu: &Gpu,
        renderer: &mut Renderer,
        world: LoadedWorld,
        spec: Option<CameraSpec>,
        environment: &crate::environment::EnvironmentSpec,
        play: Option<CameraMode>,
    ) {
        self.environment = Some(crate::environment::SceneEnvironment::new(
            gpu,
            renderer,
            world.environment.clone(),
            world.sky_noise.as_ref(),
            environment,
        ));
        if let Some(env) = &mut self.environment {
            env.set_sky_texture(sky_texture(&world).as_ref());
        }
        self.lights = world_lights(&world);
        let terrain =
            TerrainRenderer::new(gpu, renderer, &world.landscape, Some(world.reader.clone()));
        let sea_config = SeaConfig::new(world.sea.waves, world.sea.water_ex);
        let (sea, sea_params) = SeaRenderer::new(
            gpu,
            renderer,
            &world.landscape.heights,
            world.landscape.world_size,
            sea_config,
            Some(world.reader),
        );
        renderer.add_feature(Box::new(sea));
        if let Some(environment) = &mut self.environment {
            environment.attach_sea(sea_params, &world.sea);
        }
        if let Some(objects) = world.objects {
            self.models = Some(objects.attach(gpu, renderer));
        }
        let stats = terrain.stats();
        renderer.add_feature(Box::new(terrain));
        if let Some(roads) = &world.roads {
            match crate::roads::feature(gpu, renderer, roads) {
                Ok(feature) => renderer.add_feature(Box::new(feature)),
                Err(e) => log::warn!("roads not drawn: {e:#}"),
            }
        }
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
        let view = WorldView {
            name: world.name,
            heights,
            stats,
        };
        if let Some(mode) = play {
            let mut player = Player::new(DVec3::new(
                spec.east,
                f64::from(view.height(spec.east, spec.north)).max(0.0),
                spec.north,
            ));
            player.yaw = spec.heading.to_radians();
            player.pitch = spec.pitch.to_radians();
            player.switch_mode(mode);
            log::info!(
                "playing as a Man at {:.0} {:.0} ({}), {:.0} m above sea level",
                spec.east,
                spec.north,
                match mode {
                    CameraMode::FirstPerson => "first person",
                    CameraMode::ThirdPerson => "third person",
                },
                player.position.y
            );
            let strings = crate::hud::load_strings(&world.vfs);
            self.hud = Some(crate::hud::Hud::new(
                gpu,
                renderer,
                Arc::clone(&world.config),
                world.vfs.clone(),
                strings,
                crate::player::PLAYER_CLASS,
                self.hud_exec.take(),
            ));
            if let Some(path) = &world.player_model {
                log::info!("player Man model: {path}");
                let loadout = crate::gear::loadout(&world.config, crate::player::PLAYER_CLASS);
                self.soldier = Some(SoldierModel::new(
                    gpu,
                    renderer,
                    world.vfs.clone(),
                    path,
                    crate::man::moves_of(&world.config),
                    Some(&loadout),
                ));
            } else {
                log::warn!(
                    "no model for {} in the game data; drawing the stand-in Man",
                    crate::player::PLAYER_CLASS
                );
            }
            match Combat::new(
                world.terrain.clone(),
                world.vfs.clone(),
                world.config.clone(),
                crate::player::PLAYER_CLASS,
                player.position,
            ) {
                Ok(combat) => self.combat = Some(combat),
                Err(e) => log::warn!("the player is unarmed: {e:#}"),
            }
            self.camera = player.camera();
            self.player = Some(player);
        }
        self.world = Some(view);
    }

    /// Show one model with an orbit camera (the model viewer), posed as a Man in
    /// `ModelSpec::pose` when `moves` (the Moves type) is given.
    pub fn load_model(
        &mut self,
        gpu: &Gpu,
        renderer: &mut Renderer,
        vfs: a3_vfs::Vfs,
        spec: ModelSpec,
        config: Option<&a3_config::ConfigTree>,
    ) {
        let loadout = match (&spec.loadout, config) {
            (Some(class), Some(config)) => Some(crate::gear::loadout(config, class)),
            _ => None,
        };
        let moves = config.and_then(crate::man::moves_of);
        let man = match (&spec.pose, moves) {
            (Some((name, phase)), Some(moves)) => {
                match ManAnimation::load(&vfs, moves, &spec.path) {
                    Some(mut man) => {
                        let face = loadout.as_ref().and_then(|l| l.head.as_ref()?.face.clone());
                        if let Some(face) = face {
                            man.set_face(&face);
                        }
                        if !man.switch_move(name, *phase) {
                            log::warn!("no Move {name} in CfgMovesMaleSdr; the Man stands idle");
                        }
                        Some(man)
                    }
                    None => {
                        log::warn!("{} cannot be posed as a Man; drawn unposed", spec.path);
                        None
                    }
                }
            }
            _ => None,
        };
        let models = crate::models::model_feature(gpu, renderer, vfs.clone(), 4096);
        let mut orbit = Orbit::new(&models, spec, WORLD_CENTRE);
        if let Some(man) = man {
            let gear =
                loadout.map(|l| ManGear::load(&vfs, &mut models.lock(), &l, &orbit.spec.path));
            orbit = orbit.with_man(man, gear);
        }
        self.orbit = Some(orbit);
        self.models = Some(models);
    }

    /// Whether models or textures are still loading (the player Man included, so a screenshot
    /// waits for his body).
    pub fn models_loading(&self) -> bool {
        self.models.as_ref().is_some_and(|m| !m.lock().is_idle())
            || self.soldier.as_ref().is_some_and(SoldierModel::loading)
    }

    /// Model renderer statistics of the last frame.
    pub fn model_stats(&self) -> Option<ModelStats> {
        self.models.as_ref().map(|m| m.lock().stats())
    }

    /// Update the renderer's lighting, fog and sky from the World's environment.
    pub fn prepare_render(&mut self, renderer: &mut Renderer, dt: f32) {
        if let Some(env) = &mut self.environment {
            env.apply(renderer, self.camera.position.y as f32, dt);
        }
    }

    /// Brings the player's HUD up to date for a `size` output; call once per frame before
    /// rendering.
    pub fn update_hud(&mut self, size: (u32, u32), dt: f64) {
        if let (Some(hud), Some(player)) = (&mut self.hud, &self.player) {
            if let Some(combat) = &self.combat {
                let (loaded, spare) = combat.rounds();
                hud.set_rounds(loaded, spare);
            }
            hud.frame(player, size, dt);
        }
    }

    /// Terrain statistics of the loaded World.
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
        if self.player.is_some() {
            self.update_player(input, mouse_look, dt);
            return;
        }
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

    /// One frame of play: mouse look, movement over the terrain, and the camera on the man.
    fn update_player(&mut self, input: &InputState, mouse_look: bool, dt: f64) {
        let Some(world) = &self.world else { return };
        let Some(player) = &mut self.player else {
            return;
        };
        if mouse_look {
            // Raw mouse motion, like the free camera: right turns clockwise, up looks up.
            let m = |axis| input.value(InputCode::MouseAxis(axis));
            let sensitivity = self.controller.look_sensitivity;
            player.look(
                (m(MouseAxis::Right) - m(MouseAxis::Left)) * sensitivity,
                (m(MouseAxis::Up) - m(MouseAxis::Down)) * sensitivity,
            );
        }
        player.update(&self.actions, input, world, dt);
        // The Man plays the Move his movement just called for, one frame further on.
        if let Some(soldier) = &self.soldier {
            soldier.advance(player, dt as f32);
        }
        if let Some(combat) = &mut self.combat {
            let fire = mouse_look && self.actions.is_active(input, actions::DEFAULT_ACTION);
            combat.update(
                player.position,
                f64::from(player.yaw),
                player.eye(),
                crate::player::look_direction(player.yaw, player.pitch),
                player.mode == CameraMode::FirstPerson,
                fire,
                self.actions.just_triggered(input, actions::RELOAD_MAGAZINE),
                self.actions
                    .just_triggered(input, crate::player::NEXT_WEAPON),
                dt,
            );
        }
        let mut camera = player.camera();
        // The third-person boom may hang over a slope behind the player: keep it above the
        // terrain there.
        let ground = f64::from(world.height(camera.position.x, camera.position.z)).max(0.0);
        camera.position.y = camera.position.y.max(ground + THIRD_PERSON_CLEARANCE);
        self.camera = camera;
    }

    /// Fill `draws` with this frame's meshes and lines.
    pub fn draw(&self, draws: &mut DrawList) {
        if let (Some(orbit), Some(models)) = (&self.orbit, &self.models) {
            orbit.draw(draws, models);
            return;
        }
        if let (Some(player), Some(a)) = (&self.player, &self.assets) {
            self.draw_man(draws, a, player);
        }
        if let Some(combat) = &self.combat {
            combat.draw(
                draws,
                self.assets.as_ref().map(|a| a.cube),
                self.camera.position,
            );
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

    /// Draw the player Man for this frame.
    ///
    /// With the game's own Man model loaded, he is placed at the body transform in third person
    /// and taken out of the frame in first person. Until the model is ready (or when the game
    /// data has none) the stand-in below is drawn instead, so the player is never a hole in the
    /// terrain.
    fn draw_man(&self, draws: &mut DrawList, assets: &SceneAssets, player: &Player) {
        if player.mode == CameraMode::FirstPerson {
            if let Some(soldier) = &self.soldier {
                // The engine hides the body the eyes are in.
                soldier.place(None);
            }
            return;
        }
        if let Some(soldier) = &self.soldier {
            if soldier.ready() {
                soldier.place(Some(player.transform()));
                return;
            }
        }
        self.draw_soldier(draws, assets, player);
    }

    /// Draw the player, seen from outside.
    ///
    /// STAND-IN: until the Man's own model is loaded (or when the game data has none), the player
    /// is built from the test meshes, sized to a soldier (1.8 m tall, shoulders at 1.4 m) and
    /// rotated with the body. First person hides it, like the engine hides the body the eyes are
    /// in.
    fn draw_soldier(&self, draws: &mut DrawList, assets: &SceneAssets, player: &Player) {
        if player.mode == CameraMode::FirstPerson {
            return;
        }
        let feet = player.position;
        let rotation = DQuat::from_rotation_y(player.yaw as f64);
        let boxed = |offset: DVec3, size: DVec3| {
            DAffine3::from_scale_rotation_translation(size, rotation, feet + rotation * offset)
        };
        // Torso centre and height, hip centre, and head centre for the stance.
        let (torso, hips, head) = match player.stance {
            Stance::Stand => ((1.05, 0.62), 0.72, 1.52),
            Stance::Crouch => ((0.72, 0.55), 0.45, 1.05),
            Stance::Prone => ((0.25, 0.4), 0.22, 0.5),
        };
        let olive = [0.24, 0.30, 0.18, 1.0];
        let part = |offset: DVec3, size: DVec3, color| MeshDraw {
            mesh: assets.cube,
            texture: None,
            transform: boxed(offset, size),
            color,
            transparent: false,
        };
        draws.mesh(part(
            DVec3::new(0.0, torso.0, 0.0),
            DVec3::new(0.44, torso.1, 0.26),
            olive,
        ));
        draws.mesh(part(
            DVec3::new(0.0, hips, 0.0),
            DVec3::new(0.36, 0.35, 0.24),
            olive,
        ));
        draws.mesh(MeshDraw {
            mesh: assets.sphere,
            texture: None,
            transform: boxed(DVec3::new(0.0, head, 0.0), DVec3::splat(0.26)),
            color: [0.55, 0.45, 0.35, 1.0],
            transparent: false,
        });
        for side in [-1.0, 1.0] {
            // Leg and arm.
            draws.mesh(part(
                DVec3::new(0.11 * side, torso.0 - torso.1 * 0.5 - 0.35, 0.0),
                DVec3::new(0.15, 0.7, 0.18),
                [0.20, 0.26, 0.15, 1.0],
            ));
            draws.mesh(part(
                DVec3::new(0.28 * side, torso.0 - 0.05, 0.0),
                DVec3::new(0.13, 0.6, 0.16),
                olive,
            ));
        }
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
        let help = match (self.player.is_some(), captured) {
            (true, true) => {
                "WASD MOVE  SHIFT SPRINT  CTRL WALK  C CROUCH  Z PRONE  X STAND  \
                 NUM ENTER VIEW  MOUSE LOOK  TAB RELEASE  ESC QUIT"
            }
            (true, false) => {
                "WASD MOVE  C CROUCH  Z PRONE  X STAND  NUM ENTER VIEW  \
                 CLICK TO LOOK  ESC QUIT"
            }
            (false, true) => "WASD Q Z MOVE  SHIFT/CTRL FAST  MOUSE LOOK  TAB RELEASE  ESC QUIT",
            (false, false) => "WASD Q Z MOVE  SHIFT/CTRL FAST  CLICK TO LOOK  ESC QUIT",
        };
        draws.text(8.0, 52.0, 2.0, dim, help);
        draws.text(8.0, 74.0, 2.0, dim, format!("KEYS {keys}"));
        // Play mode reports the player first; the World objects line follows it.
        let mut y: f32 = 118.0;
        if let Some(p) = &self.player {
            draws.text(
                8.0,
                y,
                2.0,
                dim,
                format!(
                    "PLAYER {} {}",
                    match p.mode {
                        CameraMode::FirstPerson => "FIRST PERSON",
                        CameraMode::ThirdPerson => "THIRD PERSON",
                    },
                    match p.stance {
                        Stance::Stand => "STANDING",
                        Stance::Crouch => "CROUCHED",
                        Stance::Prone => "PRONE",
                    }
                ),
            );
            y += 22.0;
            let man = match &self.soldier {
                Some(soldier) => soldier.describe(),
                None => format!(
                    "MAN {}  NO MODEL IN THE GAME DATA, STAND-IN DRAWN",
                    crate::player::PLAYER_CLASS
                ),
            };
            draws.text(8.0, y, 2.0, dim, man);
            y += 22.0;
            if let Some(combat) = &self.combat {
                for line in combat.overlay() {
                    draws.text(8.0, y, 2.0, dim, line);
                    y += 22.0;
                }
            }
        }
        if let Some(models) = &self.models {
            let line = match &self.orbit {
                Some(orbit) => format!(
                    "{}  {}",
                    orbit.describe(models),
                    stats_line(&models.lock().stats())
                ),
                None => stats_line(&models.lock().stats()),
            };
            draws.text(8.0, y, 2.0, dim, line);
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

/// The World's placed lights: every Static object whose `CfgVehicles` class carries
/// `Reflectors`, at the position and direction its p3d's Memory LOD names
/// (`docs/re/render-materials.md` §3.4).
///
/// The WRP holds a model path per object and the config holds a model path per class, so the
/// two are matched on the path ([`a3_environment::normalise_path`]). Each lamp model is decoded
/// once and its memory points cached; a class whose model is missing, and an object whose
/// memory point does not resolve, is skipped rather than guessed at.
fn world_lights(world: &LoadedWorld) -> Vec<a3_environment::PlacedLight> {
    let Some(objects) = &world.objects else {
        return Vec::new();
    };
    let classes = a3_environment::lamp_classes(&world.config.root());
    if classes.is_empty() {
        return Vec::new();
    }
    let cache =
        std::cell::RefCell::new(std::collections::HashMap::<String, Option<a3_p3d::Model>>::new());
    let locate = |model: &str, point: &str| -> Option<Vec3> {
        let mut cache = cache.borrow_mut();
        let entry = cache.entry(model.to_owned()).or_insert_with(|| {
            let path = model.to_owned();
            let bytes = world.vfs.open(&path).ok()?;
            a3_p3d::Model::from_bytes(&bytes)
                .map_err(|e| log::warn!("lamp model {path}: {e}"))
                .ok()
        });
        entry.as_ref()?.memory_point(point)
    };
    let lights = a3_environment::placed_lights(&classes, &objects.models, &objects.placed, locate);
    let spots = lights.iter().filter(|l| l.spot).count();
    log::info!(
        "lights: {} lamp classes, {} placed lights ({} spot) at the first {:?}",
        classes.len(),
        lights.len(),
        spots,
        lights.first().map(|l| (
            l.class.as_str(),
            [l.position.x, l.position.y, l.position.z],
            l.reflector.attenuation.hard_limit_end
        ))
    );
    lights
}

/// The World's `skyTexture` (`CfgWorlds >> skyTexture`), decoded for the sky dome's ramp: the
/// `Sky` textures the engine ships are 8x8 ramps, so only the first mip is read.
///
/// The channels come back in the layout the engine's shaders expect (`Swizzle::restore`): a
/// `Sky` texture keeps its alpha in the inverted green channel and its green in the inverted
/// alpha one (`docs/re/render-atmosphere.md` §4.1), so the raw mip's green is not the colour.
fn sky_texture(world: &LoadedWorld) -> Option<TextureData> {
    let class = world.config.root().get("CfgWorlds").get(&world.name);
    let path = class.get("skyTexture");
    if !path.is_text() {
        return None;
    }
    let path = path.text();
    let bytes = world.vfs.open(&path).ok()?;
    let texture = a3_paa::Texture::read(&bytes)
        .map_err(|e| log::warn!("sky texture {path}: {e}"))
        .ok()?;
    let mip = texture.mips.first()?;
    let mut pixels = a3_paa::decode_rgba8(texture.format, mip)
        .map_err(|e| log::warn!("sky texture {path}: {e}"))
        .ok()?;
    if let Some(swizzle) = texture.swizzle {
        swizzle.restore(&mut pixels);
    }
    Some(TextureData {
        format: a3_render::TextureFormat::Rgba8,
        width: u32::from(texture.width()),
        height: u32::from(texture.height()),
        mips: vec![pixels],
    })
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
    use glam::{Affine3A, Quat};

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

    /// A posed Man's ground point lands on the entity's position, turned with him.
    #[test]
    fn the_ground_point_lands_on_the_placement() {
        let spin = DAffine3::from_rotation_translation(
            DQuat::from_rotation_y(0.4),
            DVec3::new(15_000.0, 3.0, 15_000.0),
        );
        // B_Soldier_F's bounding centre: the ground is at minus it in model space.
        let ground = -Vec3::new(0.305, 0.836, -0.401);
        let placed = ground_placement(spin, ground);
        let at = placed.transform_point3(ground.as_dvec3());
        assert!((at - spin.translation).length() < 1e-9, "{at:?}");
        assert_eq!(placed.matrix3, spin.matrix3);
    }

    /// The Man's ground lift must translate his whole posed mesh, not each bone along its own
    /// rotated up axis (issue #247: a bone-relative lift tore the soldier into a spike-ball).
    #[test]
    fn the_ground_lift_moves_the_instance_not_the_bones() {
        // The lift measured for B_Soldier_F: its mesh sits 1.852 m below the model origin.
        let lift = 1.852_f32;
        let spin = DAffine3::from_rotation_translation(
            DQuat::from_rotation_y(0.4),
            DVec3::new(15_000.0, 3.0, 15_000.0),
        );
        let placed = lift_placement(spin, lift);
        assert_eq!(placed.translation.y, spin.translation.y + f64::from(lift));
        assert_eq!(
            placed.matrix3, spin.matrix3,
            "the lift must not touch the placement's rotation or scale"
        );

        // A posed bone: rotated 90 deg, so its own up axis is the world -x axis.
        let bone = Affine3A::from_rotation_translation(
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(0.3, 1.0, 0.1),
        );
        let vertex = Vec3::new(-0.2, 1.4, 0.05);
        let local = DVec3::from(bone.transform_point3(vertex));
        let rise =
            lift_placement(spin, lift).transform_point3(local) - spin.transform_point3(local);
        assert!(
            (rise - DVec3::new(0.0, f64::from(lift), 0.0)).length() < 1e-6,
            "every vertex must rise by exactly the lift, got {rise:?}"
        );

        // The old form folded the lift into each bone: `bone * up` maps the lift through the
        // bone's rotation, so the same vertex moves sideways by the lift instead of upwards.
        let up = Affine3A::from_translation(Vec3::new(0.0, lift, 0.0));
        let per_bone = (bone * up).transform_point3(vertex) - bone.transform_point3(vertex);
        assert!(
            (per_bone - Vec3::new(-lift, 0.0, 0.0)).length() < 1e-4,
            "a bone-relative lift is rotated by the bone, got {per_bone:?}"
        );
    }
}
