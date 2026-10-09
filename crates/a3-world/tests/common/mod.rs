//! Synthetic models, terrain, surfaces and config for the ballistics tests.
//!
//! The model and terrain fixtures mirror `crates/a3-physics/tests/common`: a box LOD of
//! `ComponentNN` selections with one material each, a flat terrain, and an in-memory VFS holding
//! the `.bisurf` files. On top of them sits the in-memory `CfgAmmo` / `CfgMagazines` / `CfgWeapons`
//! the World fires from.
#![allow(dead_code)]

use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_p3d::{
    Encoding, Face, Lod, LodResolution, Material, Model, ModelInfo, NamedSelection, Section,
    Vertices,
};
use a3_physics::{CollisionWorld, Interest, MemoryFiles, ModelBank};
use a3_world::{ClientId, World};
use a3_wrp::{TerrainBuilder, Transform};
use glam::{DVec3, Vec3};

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
pub const CONFIG: &str = r#"
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

/// The weapons config of [`CONFIG`].
pub fn config() -> Arc<ConfigTree> {
    let config = parse_text(CONFIG).unwrap();
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

/// A flat terrain at `h`, 8 land cells of 16 m (128 m square).
pub fn flat_terrain(h: f32) -> TerrainBuilder {
    TerrainBuilder::new(8, 32, 16.0).heights(move |_, _| h)
}

pub fn at(x: f64, y: f64, z: f64) -> Transform {
    Transform::from_position(Vec3::new(x as f32, y as f32, z as f32))
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
    let mut builder = flat_terrain(-5.0);
    for (path, _, position) in objects {
        builder = builder.object(path, at(position.x, position.y, position.z));
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
    world.set_collision(collision);
    world.stream_collision(&[Interest {
        center: DVec3::splat(64.0),
        radius: 200.0,
    }]);
    world
}
