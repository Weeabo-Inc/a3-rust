//! Synthetic Moves, terrain and RTM headers for the Man family tests.
//!
//! The moves config is hand-written (never game data), so the tests run everywhere. It models
//! one man: he stands, walks, runs, crawls and lies prone, and the action maps and the graph
//! connect those moves the way `CfgMovesBasic` and `CfgMovesMaleSdr` do.
#![allow(dead_code)]

use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_moves::Moves;
use a3_p3d::{Encoding, Face, Lod, LodResolution, Model, ModelInfo, Section, Vertices};
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
