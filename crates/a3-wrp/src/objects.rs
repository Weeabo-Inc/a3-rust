//! Placed objects, static entities and the road net.

use glam::{Affine3A, Mat3A, Vec3, Vec3A};

/// A 4x3 transform as stored by the engine: the orientation's three columns (the object's
/// x "aside", y "up" and z "direction" axes, scale included), then the position.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Transform(pub [f32; 12]);

impl Transform {
    /// The identity transform at `position`.
    pub fn from_position(position: Vec3) -> Self {
        let p = position;
        Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, p.x, p.y, p.z])
    }

    /// World position (`y` up).
    pub fn position(&self) -> Vec3 {
        Vec3::new(self.0[9], self.0[10], self.0[11])
    }

    /// The transform as an affine matrix.
    pub fn to_affine(&self) -> Affine3A {
        let m = &self.0;
        Affine3A {
            matrix3: Mat3A::from_cols(
                Vec3A::new(m[0], m[1], m[2]),
                Vec3A::new(m[3], m[4], m[5]),
                Vec3A::new(m[6], m[7], m[8]),
            ),
            translation: Vec3A::new(m[9], m[10], m[11]),
        }
    }

    /// Heading in degrees clockwise from north (world +z), the convention of SQF `getDir`.
    pub fn heading_degrees(&self) -> f32 {
        let dir = Vec3::new(self.0[6], self.0[7], self.0[8]);
        dir.x.atan2(dir.z).to_degrees().rem_euclid(360.0)
    }
}

/// One placed static object. 60 bytes in the file.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ObjectInstance {
    /// The object id, unique within the terrain (sequential from 0 in shipped terrains).
    pub id: u32,
    /// Index into [`Terrain::models`](crate::Terrain::models), 0-based.
    pub model_index: u32,
    /// Placement.
    pub transform: Transform,
    /// A per-object shape parameter _(meaning uncertain; 2 in most shipped objects)_.
    pub shape_param: u32,
}

/// A static object that the engine creates as a config-class entity (lamps, runway lights,
/// ...) rather than a plain model.
#[derive(Debug, Clone, PartialEq)]
pub struct StaticEntity {
    /// The CfgVehicles class name.
    pub class_name: String,
    /// The model path.
    pub shape: String,
    /// World position.
    pub position: Vec3,
    /// The encoded object id (see `docs/re/wrp.md`, "Object id").
    pub object_id: u32,
}

/// One connection end of a road part.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RoadConnection {
    /// World position of the end.
    pub position: Vec3,
    /// The connection type (version 24+; 0 before) _(meaning uncertain)_.
    pub kind: u8,
}

/// One road segment model.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadPart {
    /// The land cell `(x, z)` that lists it.
    pub cell: (u16, u16),
    /// The (encoded) id of the road object.
    pub object_id: u32,
    /// Index into [`RoadNet::models`].
    pub model_index: u32,
    /// Placement of the road model (version 16+; identity before).
    pub transform: Transform,
    /// Range of this part's ends in [`RoadNet::connections`].
    pub connections: std::ops::Range<u32>,
}

/// The road net: every road part, grouped by land cell.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoadNet {
    /// Every road part, in file order (land cells with `x` outer, `z` inner).
    pub parts: Vec<RoadPart>,
    /// Connection ends of all parts; each part refers to a range.
    pub connections: Vec<RoadConnection>,
    /// Distinct road model paths, as written in the file.
    pub models: Vec<String>,
}

impl RoadNet {
    /// The ends of `part`.
    pub fn connections_of(&self, part: &RoadPart) -> &[RoadConnection] {
        &self.connections[part.connections.start as usize..part.connections.end as usize]
    }

    /// The model path of `part`.
    pub fn model_of(&self, part: &RoadPart) -> &str {
        &self.models[part.model_index as usize]
    }
}
