//! What a flight model needs from the aircraft's model (ODOL): mass, centre of mass, inertia,
//! the bounding-sphere radius every torque arm scales with, the Geometry LOD points that touch
//! the ground, and the memory points that place the wheels.
//!
//! **Model space.** A raw P3D faces -Z with its left side at +X (`docs/re/p3d-odol.md`), while
//! the flight model's formulas push along +Z and put the tail rotor at -Z
//! (`docs/re/sim-air.md`, "Frames"). [`Airframe::from_model`] therefore turns the raw data half
//! a turn about Y, `(x, y, z) → (-x, y, -z)`: the nose to +Z, the right side to +X. A renderer
//! draws the raw model with the same half turn on top of the body's orientation.

use std::collections::BTreeMap;

use a3_p3d::{LodKind, Model};
use glam::{DMat3, DVec3};

/// The mass and shape of one aircraft model, in model space (X right, Y up, Z forward).
#[derive(Debug, Clone, PartialEq)]
pub struct Airframe {
    /// Mass, kg (`ModelInfo.mass`; the engine's `GetMass`).
    pub mass: f64,
    /// Centre of mass, model space.
    pub center_of_mass: DVec3,
    /// Inertia tensor about the centre of mass, model space.
    pub inertia: DMat3,
    /// Bounding-sphere radius, m (`ModelInfo.bounding_sphere`, the shape's `+0x78`; `sizeOf` is
    /// twice it). Every rotor and control-surface torque arm is a multiple of it.
    pub bounding_radius: f64,
    /// The Geometry LOD's points, model space: what touches the ground (`docs/re/sim-air.md`
    /// §2.6).
    pub contact_points: Vec<DVec3>,
    /// The memory LOD's named points (lower-case names), model space.
    pub memory_points: BTreeMap<String, DVec3>,
}

impl Airframe {
    /// A uniform box of `mass` kg and `size` m centred on the origin, with its eight corners as
    /// contact points: a stand-in for tests and models without data.
    pub fn uniform_box(mass: f64, size: DVec3) -> Airframe {
        let h = size * 0.5;
        let inertia = DMat3::from_diagonal(DVec3::new(
            mass * (size.y * size.y + size.z * size.z) / 12.0,
            mass * (size.x * size.x + size.z * size.z) / 12.0,
            mass * (size.x * size.x + size.y * size.y) / 12.0,
        ));
        let mut contact_points = Vec::with_capacity(8);
        for x in [-h.x, h.x] {
            for y in [-h.y, h.y] {
                for z in [-h.z, h.z] {
                    contact_points.push(DVec3::new(x, y, z));
                }
            }
        }
        Airframe {
            mass,
            center_of_mass: DVec3::ZERO,
            inertia,
            bounding_radius: h.length(),
            contact_points,
            memory_points: BTreeMap::new(),
        }
    }

    /// Reads a model: mass, centre of mass and inertia from `ModelInfo` (a model without an
    /// inertia tensor gets a box's from its bounding box), the Geometry LOD's points, and the
    /// memory LOD's single-vertex named selections.
    pub fn from_model(model: &Model) -> Airframe {
        let info = &model.info;
        let mass = f64::from(info.mass).max(1.0);
        let size = (info.bbox_max - info.bbox_min).as_dvec3();
        let inv = DMat3::from_cols_array(&info.inv_inertia.to_cols_array().map(f64::from));
        let inertia = if inv.determinant().abs() > 1e-20 {
            let turn = DMat3::from_diagonal(DVec3::new(-1.0, 1.0, -1.0));
            turn * inv.inverse() * turn
        } else {
            Airframe::uniform_box(mass, size).inertia
        };
        let lod_of = |index: Option<u8>, kind: LodKind| {
            index
                .map(usize::from)
                .filter(|&i| i < model.lods.len())
                .or_else(|| model.lods.iter().position(|l| l.resolution.kind() == kind))
                .map(|i| &model.lods[i])
        };
        let mut contact_points: Vec<DVec3> = Vec::new();
        if let Some(lod) = lod_of(info.special_lods.geometry, LodKind::Geometry) {
            for p in &lod.vertices.positions {
                let p = engine_space(*p);
                if !contact_points.iter().any(|q| q.distance_squared(p) < 1e-6) {
                    contact_points.push(p);
                }
            }
        }
        let mut memory_points = BTreeMap::new();
        if let Some(lod) = lod_of(info.special_lods.memory, LodKind::Memory) {
            for selection in &lod.named_selections {
                if let Some(&v) = selection.vertices.first()
                    && let Some(p) = lod.vertices.positions.get(v as usize)
                {
                    memory_points.insert(selection.name.to_ascii_lowercase(), engine_space(*p));
                }
            }
        }
        Airframe {
            mass,
            center_of_mass: engine_space(info.center_of_mass),
            inertia,
            bounding_radius: f64::from(info.bounding_sphere),
            contact_points,
            memory_points,
        }
    }

    /// A memory point by name (any case).
    pub fn memory_point(&self, name: &str) -> Option<DVec3> {
        self.memory_points.get(&name.to_ascii_lowercase()).copied()
    }

    /// The lowest contact point's height in model space, or the bottom of the bounding sphere.
    pub fn bottom(&self) -> f64 {
        self.contact_points
            .iter()
            .map(|p| p.y)
            .reduce(f64::min)
            .unwrap_or(-self.bounding_radius)
    }
}

/// A raw model point in the flight model's space: half a turn about Y.
pub fn engine_space(p: glam::Vec3) -> DVec3 {
    DVec3::new(-f64::from(p.x), f64::from(p.y), -f64::from(p.z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uniform_box_has_the_textbook_inertia_and_its_corners_as_contacts() {
        let a = Airframe::uniform_box(12.0, DVec3::new(1.0, 2.0, 3.0));
        assert_eq!(a.inertia.x_axis.x, 12.0 * (4.0 + 9.0) / 12.0);
        assert_eq!(a.contact_points.len(), 8);
        assert_eq!(a.bottom(), -1.0);
    }

    #[test]
    fn raw_points_turn_half_round_so_the_nose_is_at_plus_z() {
        // The Offroad's front lights are at raw z = -3.1 on its left side, raw x > 0.
        let light = engine_space(glam::Vec3::new(0.8, 0.5, -3.1));
        assert!(light.z > 0.0 && light.x < 0.0);
    }
}
