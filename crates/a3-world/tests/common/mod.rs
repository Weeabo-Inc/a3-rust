//! Synthetic fixtures shared by the integration tests.
//!
//! - Moves, terrain and RTM headers for the Man family tests. The moves config is hand-written
//!   (never game data), so the tests run everywhere. It models one man: he stands, walks, runs,
//!   crawls and lies prone, and the action maps and the graph connect those moves the way
//!   `CfgMovesBasic` and `CfgMovesMaleSdr` do.
//! - Box models, in-memory `.bisurf` surfaces and a synthetic `CfgWeapons` / `CfgMagazines` /
//!   `CfgAmmo` for the ballistics tests ([`scene`], [`config`]). The models mirror
//!   `crates/a3-physics/tests/common`: a Fire Geometry LOD of `ComponentNN` selections with one
//!   material each.
#![allow(dead_code)]

use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_moves::Moves;
use a3_p3d::{
    Encoding, Face, Lod, LodResolution, Material, Model, ModelInfo, NamedSelection, Section,
    Vertices,
};
use a3_physics::{CollisionWorld, Interest, MemoryFiles, ModelBank, TerrainStatics};
use a3_world::{ClientId, Create, EntityId, EntityType, SimulationClass, World};
use a3_wrp::{Terrain, TerrainBuilder, Transform};
use glam::{DVec3, Vec3};

/// A moves type with two stances (stand, prone), their action maps and the moves the graph
/// walks between them. Real names are long (`AmovPercMstpSnonWnonDnon`); short ones keep the
/// assertions readable.
///
/// `file` values are the VFS paths the RTM headers are looked up by ([`moves`]).
pub const CONFIG: &str = r#"
class CfgMovesBasic {
    class Default {
        actions = "NoActions"; file = ""; looped = 1; speed = 0.5; minPlayTime = 0;
        interpolationSpeed = 6; equivalentTo = ""; relSpeedMin = 1; relSpeedMax = 1;
        connectAs = ""; connectFrom[] = {}; connectTo[] = {};
        interpolateTo[] = {}; interpolateFrom[] = {}; interpolateWith[] = {};
        variantsPlayer[] = {}; variantsAI[] = {}; variantAfter[] = {5, 10, 20};
    };
    class ManActions {
        Stop = ""; WalkF = ""; WalkB = ""; WalkL = ""; WalkR = ""; RunF = ""; Down = "";
        // A gesture action: the engine plays these on its action layer, not as moves.
        reloadMagazine[] = {"GestureReloadMagazine", "Gesture"};
    };
    class Actions {
        class NoActions: ManActions {
            turnSpeed = 2; upDegree = -1; stance = "ManStanceUndefined";
        };
    };
    class Interpolations {};
    transitionsInterpolated[] = {};
    transitionsSimple[] = {};
    transitionsDisabled[] = {};
};
class CfgMovesTest: CfgMovesBasic {
    skeletonName = "OFP2_ManSkeleton";
    gestures = "CfgGesturesMale";
    primaryActionMaps[] = {"StandActions", "ProneActions"};
    class States {
        class Stand: Default {
            actions = "StandActions"; file = "a3\anims\stand.rtm"; speed = 1; looped = 0;
            interpolateTo[] = {"Walk", 0.1, "WalkLeft", 0.1, "WalkRight", 0.1, "Run", 0.2, "StandDown", 0.1};
        };
        class Walk: Default {
            actions = "StandActions"; file = "a3\anims\walk.rtm"; speed = 0.85; looped = 1;
            interpolateTo[] = {"Stand", 0.1, "Run", 0.1, "WalkBack", 0.1, "StandDown", 0.2};
        };
        class WalkBack: Default {
            actions = "StandActions"; file = "a3\anims\walkback.rtm"; speed = 0.8; looped = 1;
            interpolateTo[] = {"Stand", 0.1, "Walk", 0.1};
        };
        class WalkLeft: Default {
            actions = "StandActions"; file = "a3\anims\walkleft.rtm"; speed = 0.5; looped = 1;
            interpolateTo[] = {"Stand", 0.1, "Walk", 0.1};
        };
        class WalkRight: Default {
            actions = "StandActions"; file = "a3\anims\walkright.rtm"; speed = 0.5; looped = 1;
            interpolateTo[] = {"Stand", 0.1, "Walk", 0.1};
        };
        class Run: Default {
            actions = "StandActions"; file = "a3\anims\run.rtm"; speed = 1; looped = 1;
            interpolateTo[] = {"Walk", 0.1, "Stand", 0.2};
        };
        class StandDown: Default {
            actions = "NoActions"; file = "a3\anims\standdown.rtm"; speed = -1; looped = 0;
            connectTo[] = {"Prone", 0.1};
        };
        class Prone: Default {
            actions = "ProneActions"; file = "a3\anims\prone.rtm"; speed = 1; looped = 0;
            // Getting up is a transition move of its own in the real data
            // (`AmovPpneMstpSnonWnonDnon_AmovPercMstpSnonWnonDnon`); the fixture blends straight
            // back up to the stand.
            interpolateTo[] = {"Crawl", 0.1, "Stand", 0.2};
        };
        class Crawl: Default {
            actions = "ProneActions"; file = "a3\anims\crawl.rtm"; speed = 0.5; looped = 1;
            interpolateTo[] = {"Prone", 0.1};
        };
    };
    class Actions: Actions {
        class StandActions: NoActions {
            Stop = "Stand"; WalkF = "Walk"; WalkB = "WalkBack"; WalkL = "WalkLeft";
            WalkR = "WalkRight"; RunF = "Run"; Down = "StandDown";
            stance = "ManStanceStand"; turnSpeed = 2;
        };
        class ProneActions: NoActions {
            Stop = "Prone"; WalkF = "Crawl"; stance = "ManStanceProne"; turnSpeed = 1;
        };
    };
};
"#;

/// The step per animation cycle (model space, metres; forward is −Z) of every move's RTM in
/// the fixture config, keyed by the VFS path of its `file`.
pub const STEPS: &[(&str, [f32; 3])] = &[
    (r"a3\anims\stand.rtm", [0.0, 0.0, 0.0]),
    (r"a3\anims\walk.rtm", [0.0, 0.0, -1.62]),
    (r"a3\anims\walkback.rtm", [0.0, 0.0, 1.12]),
    // A left strafe steps towards the model's +X (see `sim::man::moves` on the frame).
    (r"a3\anims\walkleft.rtm", [1.0, 0.0, 0.0]),
    (r"a3\anims\walkright.rtm", [-1.0, 0.0, 0.0]),
    (r"a3\anims\run.rtm", [0.0, 0.0, -4.0]),
    (r"a3\anims\standdown.rtm", [0.0, 0.0, -0.5]),
    (r"a3\anims\prone.rtm", [0.0, 0.0, 0.0]),
    (r"a3\anims\crawl.rtm", [0.0, 0.0, -0.5]),
];

/// The plain `RTM_0101` header of a move whose cycle moves the man by `step`: the signature,
/// the step vector, no bones and no frames (see `docs/re/rtm.md`).
pub fn rtm(step: [f32; 3]) -> Vec<u8> {
    let mut bytes = b"RTM_0101".to_vec();
    for v in step {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes
}

/// The fixture moves type with every move's RTM header loaded, so each move has its step.
pub fn moves() -> Arc<Moves> {
    let tree = ConfigTree::from_config(&parse_text(CONFIG).unwrap());
    let mut moves = Moves::from_config(&tree.root().get("CfgMovesTest")).unwrap();
    let report = moves.load_rtm_headers(|path| {
        STEPS
            .iter()
            .find(|(file, _)| *file == path)
            .map(|(_, step)| rtm(*step))
    });
    assert!(report.missing.is_empty(), "{:?}", report.missing);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    Arc::new(moves)
}

/// A level terrain of `height` metres above sea level: 4 land cells of 50 m, an 8x8 height grid.
pub fn flat_terrain(height: f32) -> Terrain {
    TerrainBuilder::new(4, 8, 50.0)
        .heights(|_, _| height)
        .build()
}

/// A level terrain of `height` carrying one Static object of `model` at `transform` (a bridge,
/// a platform, a house).
pub fn terrain_with(height: f32, model: &str, transform: Transform) -> Arc<Terrain> {
    Arc::new(
        TerrainBuilder::new(4, 8, 50.0)
            .heights(move |_, _| height)
            .object(model, transform)
            .build(),
    )
}

/// A WRP object transform at a world position (no rotation, unit scale).
pub fn at(x: f32, y: f32, z: f32) -> Transform {
    Transform::from_position(Vec3::new(x, y, z))
}

/// The reserved LOD resolution of a Roadway LOD: the surface an Object offers to walk on
/// (`docs/re/p3d.md`).
const ROADWAY: f32 = 3e15;

/// The reserved LOD resolution of a ViewGeometry LOD: what blocks line of sight
/// ([`a3_physics::CollisionWorld::visibility`]).
const VIEW_GEOMETRY: f32 = 6e15;

/// A Model of one LOD, as a synthetic P3D.
pub fn model(lods: Vec<Lod>) -> Model {
    Model {
        encoding: Encoding::Mlod,
        version: 257,
        info: ModelInfo::default(),
        skeleton: None,
        animations: Vec::new(),
        lods,
    }
}

/// The Roadway LOD of a flat platform: one quad from `min` to `max` (model space x/z) at model
/// height `y` — a bridge deck, a house floor, a kerb.
pub fn roadway_lod(min: [f32; 2], max: [f32; 2], y: f32) -> Lod {
    Lod {
        resolution: LodResolution(ROADWAY),
        vertices: Vertices {
            positions: vec![
                Vec3::new(min[0], y, min[1]),
                Vec3::new(min[0], y, max[1]),
                Vec3::new(max[0], y, max[1]),
                Vec3::new(max[0], y, min[1]),
            ],
            ..Default::default()
        },
        faces: vec![Face::quad(0, 1, 2, 3)],
        textures: vec![r"a3\data_f\surfaces\betonout.paa".to_owned()],
        sections: vec![Section {
            faces: 0..1,
            texture: Some(0),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// The ViewGeometry LOD of a wall: one vertical quad `half` metres either side of the model's
/// x axis, at model depth `z`, `height` metres tall — a hut wall, a fence, a container.
pub fn view_geometry_lod(half: f32, z: f32, height: f32) -> Lod {
    Lod {
        resolution: LodResolution(VIEW_GEOMETRY),
        vertices: Vertices {
            positions: vec![
                Vec3::new(-half, 0.0, z),
                Vec3::new(half, 0.0, z),
                Vec3::new(half, height, z),
                Vec3::new(-half, height, z),
            ],
            ..Default::default()
        },
        faces: vec![Face::quad(0, 1, 2, 3)],
        ..Default::default()
    }
}

/// The collision world of `terrain`: `models` (path, model) loadable, the terrain's Static
/// objects streamed around `interests` (empty for a terrain-only world, whose surface is
/// answered everywhere). See `docs/adr/0008-collision-world.md`.
pub fn collision_world(
    terrain: Arc<Terrain>,
    models: &[(&str, Model)],
    interests: &[Interest],
) -> CollisionWorld {
    let mut bank = ModelBank::new(Arc::new(MemoryFiles::default()), None);
    for (path, model) in models {
        bank.insert_model(path, model);
    }
    let mut world = CollisionWorld::new(bank);
    world.set_terrain(Some(terrain.clone()));
    world.stream(&TerrainStatics::new(terrain), interests);
    world
}

/// A World with the fixture terrain, moves and collision world, ready for a Man to walk in.
pub fn world() -> World {
    let terrain = Arc::new(flat_terrain(100.0));
    let mut world = World::new(ClientId::SERVER);
    world.load_terrain(terrain.clone()).unwrap();
    world.load_moves(moves());
    world.set_collision_world(collision_world(terrain, &[], &[]));
    world
}

/// A soldier of type `B_Soldier_F` at `position`, in that World.
pub fn create_man(world: &mut World, position: DVec3) -> EntityId {
    let ty = Arc::new(EntityType::new("B_Soldier_F", SimulationClass::Soldier));
    world.create(Create::new(ty, position)).unwrap()
}

/// The Fire Geometry LOD resolution (`a3-p3d`).
pub const FIRE: f32 = 7e15;

/// An axis-aligned box from `min` to `max` whose faces use the material with `surface`.
#[derive(Debug, Clone)]
pub struct BoxPart {
    pub min: Vec3,
    pub max: Vec3,
    pub surface: &'static str,
}

pub fn part(min: [f32; 3], max: [f32; 3], surface: &'static str) -> BoxPart {
    BoxPart {
        min: Vec3::from(min),
        max: Vec3::from(max),
        surface,
    }
}

/// A LOD of boxes, each a `ComponentNN` selection with its own section and material.
pub fn box_lod(resolution: f32, parts: &[BoxPart]) -> Lod {
    let mut lod = Lod {
        resolution: LodResolution(resolution),
        ..Default::default()
    };
    let mut positions = Vec::new();
    for (n, p) in parts.iter().enumerate() {
        let base = positions.len() as u32;
        for i in 0..8 {
            positions.push(Vec3::new(
                if i & 1 == 0 { p.min.x } else { p.max.x },
                if i & 2 == 0 { p.min.y } else { p.max.y },
                if i & 4 == 0 { p.min.z } else { p.max.z },
            ));
        }
        let first_face = lod.faces.len() as u32;
        for q in [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ] {
            lod.faces.push(Face::quad(
                base + q[0],
                base + q[1],
                base + q[2],
                base + q[3],
            ));
        }
        lod.materials.push(Material {
            name: format!("m{n}.rvmat"),
            surface: p.surface.to_owned(),
            ..Default::default()
        });
        lod.sections.push(Section {
            faces: first_face..first_face + 6,
            material: Some(n as u32),
            ..Default::default()
        });
        lod.named_selections.push(NamedSelection {
            name: format!("Component{:02}", n + 1),
            faces: (first_face..first_face + 6).collect(),
            vertices: (base..base + 8).collect(),
            ..Default::default()
        });
    }
    lod.vertices = Vertices {
        positions,
        ..Default::default()
    };
    lod
}

/// A one-box model centred on its origin: half extents `half`, all faces `surface`.
pub fn box_model(half: [f32; 3], surface: &'static str) -> Model {
    model(vec![box_lod(
        FIRE,
        &[part(
            [-half[0], -half[1], -half[2]],
            [half[0], half[1], half[2]],
            surface,
        )],
    )])
}

/// The surfaces the synthetic materials name. `plank` stops a rifle bullet by depth, `sheet` is a
/// 10 mm plate: `bulletPenetrability` in metres per second, `thickness` in millimetres
/// (`SurfaceInfo`).
pub const BISURFS: &[(&str, &str)] = &[
    (
        "plank.bisurf",
        "rough=0.1;soundEnviron=wood;bulletPenetrability=100;",
    ),
    (
        "sheet.bisurf",
        "rough=0.1;soundEnviron=metal;bulletPenetrability=100;thickness=10;",
    ),
];

const CFG_SURFACES: &str = r#"
class CfgSurfaces {
    class Default { files = "default"; soundEnviron = "dirt"; };
    class Wood: Default { files = "wood"; soundEnviron = "wood"; };
};
"#;

/// `CfgAmmo` / `CfgMagazines` / `CfgWeapons` for the ballistics tests. Every ammo sets
/// `simulationStep` so one frame of 0.05 s is exactly one simulation step.
pub const WEAPONS_CONFIG: &str = r#"
class Mode_SemiAuto {
    dispersion = 0;
    recoil = "recoil_semi";
};
class CfgAmmo {
    class Default { simulation = ""; };
    class BulletCore: Default { simulation = "shotBullet"; simulationStep = 0.05; };
    // The standard bullet: gravity and air friction, 15 degrees of ricochet.
    class B_Ball: BulletCore {
        hit = 8;
        caliber = 0.9;
        deflecting = 15;
        deflectionSlowDown = 1;
        deflectionDirDistribution = 0;
        penetrationDirDistribution = 0;
        airFriction = -0.0012;
        coefGravity = 1;
        typicalSpeed = 800;
        timeToLive = 6;
    };
    // No gravity and no air friction: the flight is a straight line.
    class B_Flat: BulletCore {
        hit = 8;
        caliber = 0.9;
        deflecting = 0;
        deflectionSlowDown = 1;
        deflectionDirDistribution = 0;
        penetrationDirDistribution = 0;
        airFriction = 0;
        coefGravity = 0;
        typicalSpeed = 800;
        timeToLive = 6;
    };
    // Strong drag, no gravity: the never-reverse clamp of the velocity integration.
    class B_Drag: BulletCore {
        hit = 1;
        caliber = 1;
        deflecting = 0;
        deflectionDirDistribution = 0;
        penetrationDirDistribution = 0;
        airFriction = -100;
        coefGravity = 0;
        typicalSpeed = 1;
        timeToLive = 6;
    };
    // Gravity only: the air friction term is zero, so the path is the exact Euler chain.
    class B_Grav: BulletCore {
        hit = 1;
        caliber = 1;
        deflecting = 0;
        deflectionDirDistribution = 0;
        penetrationDirDistribution = 0;
        airFriction = 0;
        coefGravity = 1;
        typicalSpeed = 800;
        timeToLive = 6;
    };
    // A shot that expires quickly.
    class B_Short: BulletCore {
        hit = 1;
        caliber = 1;
        deflecting = 0;
        airFriction = 0;
        coefGravity = 0;
        typicalSpeed = 800;
        timeToLive = 0.1;
    };
    // A shell that explodes in the air after `explosionTime` seconds of flight.
    class G_Timed: BulletCore {
        hit = 1;
        indirectHit = 1;
        indirectHitRange = 0;
        explosive = 0.5;
        caliber = 1;
        deflecting = 0;
        deflectionDirDistribution = 0;
        penetrationDirDistribution = 0;
        airFriction = 0;
        coefGravity = 0;
        typicalSpeed = 900;
        timeToLive = 20;
        simulation = "shotShell";
        explosionTime = 0.1;
        fuseDistance = 0;
    };
    // An explosive shell: a direct hit and a 2.5 m blast.
    class G_HE: BulletCore {
        hit = 30;
        indirectHit = 12;
        indirectHitRange = 2.5;
        explosive = 0.6;
        caliber = 2.5;
        deflecting = 0;
        deflectionDirDistribution = 0;
        penetrationDirDistribution = 0;
        airFriction = 0;
        coefGravity = 0;
        typicalSpeed = 900;
        timeToLive = 20;
        simulation = "shotShell";
        explosionTime = 0;
        fuseDistance = 0;
    };
};
class CfgMagazines {
    class Ball_Mag { ammo = "B_Ball"; count = 30; initSpeed = 900; };
    class Flat_Mag { ammo = "B_Flat"; count = 30; initSpeed = 900; };
    class Drag_Mag { ammo = "B_Drag"; count = 30; initSpeed = 1; };
    // `initSpeed = 0` and no muzzle override: a shot fired at rest.
    class Zero_Mag { ammo = "B_Drag"; count = 30; initSpeed = 0; };
    class Grav_Mag { ammo = "B_Grav"; count = 30; initSpeed = 900; };
    class Short_Mag { ammo = "B_Short"; count = 30; initSpeed = 900; };
    class Timed_Mag { ammo = "G_Timed"; count = 1; initSpeed = 100; };
    class HE_Mag { ammo = "G_HE"; count = 1; initSpeed = 300; };
    // Names an ammo class that does not exist.
    class Bad_Mag { ammo = "No_Ammo"; count = 1; initSpeed = 900; };
};
class CfgWeapons {
    class rifle_F {
        muzzles[] = { "this" };
        magazines[] = { "Ball_Mag", "Flat_Mag", "Drag_Mag", "Zero_Mag", "Grav_Mag", "Short_Mag" };
        modes[] = { "Single" };
        class Single: Mode_SemiAuto { dispersion = 0; };
    };
    class rifle_acc_F {
        muzzles[] = { "this" };
        magazines[] = { "Flat_Mag" };
        modes[] = { "Single" };
        class Single: Mode_SemiAuto { dispersion = 0.001; };
    };
    class launcher_timed_F {
        muzzles[] = { "this" };
        magazines[] = { "Timed_Mag" };
        modes[] = { "Single" };
        class Single: Mode_SemiAuto { dispersion = 0; };
    };
    class launcher_F {
        muzzles[] = { "this" };
        magazines[] = { "HE_Mag" };
        modes[] = { "Single" };
        class Single: Mode_SemiAuto { dispersion = 0; };
    };
    class rifle_bad_F {
        muzzles[] = { "this" };
        magazines[] = { "Bad_Mag" };
        modes[] = { "Single" };
        class Single: Mode_SemiAuto { dispersion = 0; };
    };
};
"#;

/// Surfaces config, so the model bank knows the surface files.
pub fn surface_config() -> ConfigTree {
    let config = parse_text(CFG_SURFACES).unwrap();
    ConfigTree::from_config(&config)
}

/// The weapons config of [`WEAPONS_CONFIG`].
pub fn config() -> Arc<ConfigTree> {
    let config = parse_text(WEAPONS_CONFIG).unwrap();
    Arc::new(ConfigTree::from_config(&config))
}

/// The in-memory VFS holding the synthetic `.bisurf` files.
pub fn files() -> Arc<MemoryFiles> {
    let mut f = MemoryFiles::default();
    for (path, text) in BISURFS {
        f.insert(path, text.as_bytes());
    }
    Arc::new(f)
}

/// A World with the weapons config, a flat terrain (`-5` m for room to miss it), the collision
/// world over `objects` (model path, model, position, all streamed in) and the terrain's Static
/// objects as colliders.
pub fn scene(objects: &[(&str, Model, DVec3)]) -> World {
    scene_with_models(objects, &[])
}

/// [`scene`] with extra models in the collision world's bank that no Static object places:
/// entity bodies, added to the world with
/// `collision_mut().add_body(ObjectRef::Entity(id).to_body_key(), arc, ..)`.
pub fn scene_with_models(objects: &[(&str, Model, DVec3)], extra: &[(&str, Model)]) -> World {
    // A flat terrain at -5 m, 8 land cells of 16 m (128 m square).
    let mut builder = TerrainBuilder::new(8, 32, 16.0).heights(|_, _| -5.0);
    for (path, _, position) in objects {
        builder = builder.object(
            path,
            at(position.x as f32, position.y as f32, position.z as f32),
        );
    }
    let terrain = Arc::new(builder.build());

    let mut bank = ModelBank::new(files(), Some(&surface_config()));
    for (path, m, _) in objects {
        bank.insert_model(path, m);
    }
    for (path, m) in extra {
        bank.insert_model(path, m);
    }

    let mut world = World::new(ClientId::SERVER);
    world.set_config(config());
    world.load_terrain(terrain.clone()).unwrap();

    let mut collision = CollisionWorld::new(bank);
    collision.set_terrain(Some(terrain));
    world.set_collision_world(collision);
    world.stream_collision(&[Interest {
        center: DVec3::splat(64.0),
        radius: 200.0,
    }]);
    world
}
