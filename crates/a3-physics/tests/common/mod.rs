//! Synthetic models and terrains for the collision tests.
#![allow(dead_code)]

use std::sync::Arc;

use a3_p3d::{
    Encoding, Face, Lod, LodResolution, Material, Model, ModelInfo, NamedSelection, Section,
    Vertices,
};
use a3_physics::{CollisionWorld, MemoryFiles, ModelBank, TerrainStatics};
use a3_wrp::{Terrain, TerrainBuilder, Transform};
use glam::Vec3;

pub const GEOMETRY: f32 = 1e13;
pub const FIRE: f32 = 7e15;
pub const VIEW: f32 = 6e15;
pub const ROADWAY: f32 = 3e15;

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

/// A LOD of boxes, each a `componentNN` selection with its own section and material.
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

/// A flat Roadway LOD: one quad from `min` to `max` (x/z) at height `y`, textured `texture`.
pub fn roadway_lod(min: [f32; 2], max: [f32; 2], y: f32, texture: &str) -> Lod {
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
        textures: vec![texture.to_owned()],
        sections: vec![Section {
            faces: 0..1,
            texture: Some(0),
            ..Default::default()
        }],
        ..Default::default()
    }
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

pub fn model_with_mass(lods: Vec<Lod>, mass: f32) -> Model {
    let mut m = model(lods);
    m.info.mass = mass;
    m
}

pub const BISURFS: &[(&str, &str)] = &[
    (
        "concrete.bisurf",
        "rough=0.1;soundEnviron=concrete;bulletPenetrability=80;",
    ),
    (
        "metal.bisurf",
        "rough=0;soundEnviron=metal;bulletPenetrability=10;",
    ),
    (
        "glass.bisurf",
        "rough=0;soundEnviron=glass;transparency=0.5;",
    ),
    (
        "leaves.bisurf",
        "rough=0;soundEnviron=grass;transparency=0.8;",
    ),
];

const CFG_SURFACES: &str = r#"
class CfgSurfaces {
    class Default { files = "default"; soundEnviron = "dirt"; };
    class Concrete: Default { files = "betonout"; soundEnviron = "concrete"; };
};
"#;

pub fn files() -> Arc<MemoryFiles> {
    let mut f = MemoryFiles::default();
    for (path, text) in BISURFS {
        f.insert(path, text.as_bytes());
    }
    Arc::new(f)
}

/// A flat terrain at height `h`: 8 land cells of 16 m (128 m square), heights every 4 m.
pub fn flat_terrain(h: f32) -> TerrainBuilder {
    TerrainBuilder::new(8, 32, 16.0).heights(move |_, _| h)
}

pub fn at(x: f32, y: f32, z: f32) -> Transform {
    Transform::from_position(Vec3::new(x, y, z))
}

/// A collision world over `terrain` whose models are `models` (path, model).
pub fn world(terrain: Terrain, models: &[(&str, Model)]) -> (CollisionWorld, TerrainStatics) {
    let config = a3_config::parse_text(CFG_SURFACES).unwrap();
    let tree = a3_config::ConfigTree::from_config(&config);
    let mut bank = ModelBank::new(files(), Some(&tree));
    for (path, m) in models {
        bank.insert_model(path, m);
    }
    let terrain = Arc::new(terrain);
    let mut world = CollisionWorld::new(bank);
    world.set_terrain(Some(terrain.clone()));
    (world, TerrainStatics::new(terrain))
}
