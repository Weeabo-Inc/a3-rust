//! Per-cell geography flags.

/// The packed 16-bit geography record of one land cell: water depth, forest and road flags,
/// object counts and slope. Bit layout (least significant first): see `docs/re/wrp.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Geography(pub u16);

impl Geography {
    fn bits(self, shift: u32, width: u32) -> u8 {
        ((self.0 >> shift) & ((1 << width) - 1)) as u8
    }

    /// Minimum water depth class, 0 (dry) to 3 (deep). Bits 0-1.
    pub fn min_water_depth(self) -> u8 {
        self.bits(0, 2)
    }

    /// The cell is fully covered (by forest or objects). Bit 2.
    pub fn full(self) -> bool {
        self.bits(2, 1) != 0
    }

    /// The cell holds forest. Bit 3.
    pub fn forest(self) -> bool {
        self.bits(3, 1) != 0
    }

    /// A road crosses the cell. Bit 4.
    pub fn road(self) -> bool {
        self.bits(4, 1) != 0
    }

    /// Maximum water depth class, 0 (dry) to 3 (deep). Bits 5-6.
    pub fn max_water_depth(self) -> u8 {
        self.bits(5, 2)
    }

    /// Object count class, 0-3. Bits 7-8.
    pub fn how_many_objects(self) -> u8 {
        self.bits(7, 2)
    }

    /// Hard (collidable) object count class, 0-3. Bits 9-10.
    pub fn how_many_hard_objects(self) -> u8 {
        self.bits(9, 2)
    }

    /// Slope class, 0 (flat) to 7 (steep). Bits 11-13.
    pub fn gradient(self) -> u8 {
        self.bits(11, 3)
    }

    /// Some object in the cell has a roadway (walkable) LOD. Bit 14.
    pub fn some_roadway(self) -> bool {
        self.bits(14, 1) != 0
    }

    /// The cell holds some objects. Bit 15.
    pub fn some_objects(self) -> bool {
        self.bits(15, 1) != 0
    }
}
