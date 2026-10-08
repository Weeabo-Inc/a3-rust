//! Collision layers: which LOD of an Object answers a query.

/// One of the special LODs an Object collides through. Each is a separate query layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    /// Geometry LOD: solid collision and mass; what Entities bump into.
    Geometry = 0,
    /// Fire Geometry LOD: what bullets and shells hit (falls back to View Geometry, then
    /// Geometry, when a model has none, as the ODOL special LOD index does).
    FireGeometry = 1,
    /// View Geometry LOD: what blocks AI and player line of sight (falls back to Geometry).
    ViewGeometry = 2,
    /// Roadway LOD: the walkable and drivable surfaces of buildings and bridges.
    Roadway = 3,
}

impl Layer {
    pub const ALL: [Layer; 4] = [
        Layer::Geometry,
        Layer::FireGeometry,
        Layer::ViewGeometry,
        Layer::Roadway,
    ];

    pub(crate) fn from_index(i: u32) -> Option<Layer> {
        Layer::ALL.get(i as usize).copied()
    }

    pub fn mask(self) -> LayerMask {
        LayerMask(1 << self as u8)
    }
}

/// A set of [`Layer`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LayerMask(pub u8);

impl LayerMask {
    pub const NONE: LayerMask = LayerMask(0);
    pub const GEOMETRY: LayerMask = LayerMask(1);
    pub const FIRE: LayerMask = LayerMask(2);
    pub const VIEW: LayerMask = LayerMask(4);
    pub const ROADWAY: LayerMask = LayerMask(8);
    pub const ALL: LayerMask = LayerMask(15);

    pub fn contains(self, layer: Layer) -> bool {
        self.0 & layer.mask().0 != 0
    }
}

impl std::ops::BitOr for LayerMask {
    type Output = LayerMask;
    fn bitor(self, rhs: LayerMask) -> LayerMask {
        LayerMask(self.0 | rhs.0)
    }
}

impl From<Layer> for LayerMask {
    fn from(layer: Layer) -> LayerMask {
        layer.mask()
    }
}
