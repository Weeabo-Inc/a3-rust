//! Formation geometry: `cfgFormations` and the engine's slot computation
//! (`AICenter_LoadCfgFormations`, `AISubgroup_UpdateFormationPos`; `docs/re/ai.md` §4.1).
//!
//! Each formation is a list of *fixed* entries for the first slots and a *pattern* repeated
//! for the rest. An entry places its slot relative to a reference slot, in formation units: the
//! average `formationX`/`formationZ` of the two units' types (5 m between two men). Slot 0 is
//! the leader's place in the group's ID order; positions are relative to the leader's slot and
//! turned by the formation direction.

use a3_config::{ConfigRef, Value};
use glam::DVec3;

use super::Formation;

/// One `FormationPositionInfoN[] = {reference, x, z, angle}` entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FormationEntry {
    /// The slot this one is placed from: absolute in the fixed list, relative to the entry's
    /// own block in the pattern; negative places the slot at `(x, z)` itself, unscaled.
    pub reference: i32,
    /// To the right of the formation direction, in formation units.
    pub x: f64,
    /// Along the formation direction (ahead positive), in formation units.
    pub z: f64,
    /// Where the unit in this slot watches, in radians from the formation direction.
    pub angle: f64,
}

/// One formation: its fixed entries and its repeating pattern.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FormationShape {
    pub fixed: Vec<FormationEntry>,
    pub pattern: Vec<FormationEntry>,
}

/// The nine formations of a side, in the engine's formation order (COLUMN, STAG COLUMN, WEDGE,
/// ECH LEFT, ECH RIGHT, VEE, LINE, DIAMOND, FILE).
#[derive(Debug, Clone, PartialEq)]
pub struct FormationTable {
    shapes: [FormationShape; 9],
}

/// A unit's place in a formation: where, relative to slot 0, and where he watches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FormationSlot {
    /// Right (`x`) and ahead (`z`) of slot 0, in metres.
    pub offset: DVec3,
    /// Watch direction relative to the formation direction, in radians.
    pub angle: f64,
}

fn entry(reference: i32, x: f64, z: f64, angle: f64) -> FormationEntry {
    FormationEntry {
        reference,
        x,
        z,
        angle,
    }
}

impl Formation {
    /// The formation's index in the engine's enum and in `cfgFormations` (read by position).
    pub fn engine_index(self) -> usize {
        match self {
            Formation::Column => 0,
            Formation::StagColumn => 1,
            Formation::Wedge => 2,
            Formation::EchLeft => 3,
            Formation::EchRight => 4,
            Formation::Vee => 5,
            Formation::Line => 6,
            Formation::Diamond => 7,
            Formation::File => 8,
        }
    }
}

impl Default for FormationTable {
    fn default() -> Self {
        Self::shipped()
    }
}

impl FormationTable {
    /// The table `cfgFormations` ships (every side has the same one).
    pub fn shipped() -> Self {
        // The config holds single-precision numbers; keep them so, so a table read from the
        // shipped config compares equal.
        let single = |x: f64| f64::from(x as f32);
        let (q, half, pi) = (
            single(std::f64::consts::FRAC_PI_4),
            single(std::f64::consts::FRAC_PI_2),
            single(std::f64::consts::PI),
        );
        // COLUMN, STAG COLUMN, ECH LEFT/RIGHT: four fixed slots, then the same four as a block.
        let column = |x: [f64; 4], a: [f64; 4]| FormationShape {
            fixed: vec![
                entry(-1, 0.0, 0.0, 0.0),
                entry(0, x[1], -1.0, a[1]),
                entry(1, x[2], -1.0, a[2]),
                entry(2, x[3], -1.0, a[3]),
            ],
            pattern: vec![
                entry(-1, x[0], -1.0, a[0]),
                entry(0, x[1], -1.0, a[1]),
                entry(1, x[2], -1.0, a[2]),
                entry(2, x[3], -1.0, a[3]),
            ],
        };
        // WEDGE, VEE, LINE: two fixed slots, then pairs placed from the slot two back.
        let pairs = |z0: f64, z: f64, a0: f64, a: [f64; 2]| FormationShape {
            fixed: vec![entry(-1, 0.0, 0.0, a0), entry(0, 1.0, z0, a[1])],
            pattern: vec![entry(-2, -1.0, z, a[0]), entry(-1, 1.0, z, a[1])],
        };
        Self {
            shapes: [
                column([0.0, 0.0, 0.0, 0.0], [0.0, q, -q, pi]),
                column([-1.0, 1.0, -1.0, 1.0], [0.0, q, -q, pi]),
                pairs(-1.0, -1.0, 0.0, [-q, q]),
                column([-1.0, -1.0, -1.0, -1.0], [0.0, -q, -q, -half]),
                column([1.0, 1.0, 1.0, 1.0], [0.0, q, q, half]),
                pairs(0.0, 1.0, -q, [-q, q]),
                pairs(0.0, 0.0, 0.0, [0.0, 0.0]),
                FormationShape {
                    fixed: vec![entry(-1, 0.0, 0.0, 0.0)],
                    pattern: vec![
                        entry(-1, 0.5, -0.5, q),
                        entry(0, -1.0, 0.0, -q),
                        entry(1, 0.5, -0.5, 0.0),
                    ],
                },
                FormationShape {
                    fixed: vec![entry(-1, 0.0, 0.0, 0.0)],
                    pattern: vec![entry(-1, 0.0, -0.5, -q), entry(0, 0.0, -0.5, q)],
                },
            ],
        }
    }

    /// The table of one side class of `cfgFormations` (`cfgFormations >> West`): its
    /// formation classes by position. A side with fewer than nine keeps the engine default
    /// (a LINE) for the missing ones.
    pub fn from_config(side: &ConfigRef<'_>) -> Self {
        let line = FormationShape {
            fixed: vec![entry(-1, 0.0, 0.0, 0.0), entry(0, 1.0, 0.0, 0.0)],
            pattern: vec![entry(-2, -1.0, 0.0, 0.0), entry(-1, 1.0, 0.0, 0.0)],
        };
        let mut shapes: [FormationShape; 9] = std::array::from_fn(|_| line.clone());
        let classes = side.entries().into_iter().filter(ConfigRef::is_class);
        for (shape, class) in shapes.iter_mut().zip(classes) {
            *shape = FormationShape {
                fixed: entries(&class.get("Fixed")),
                pattern: entries(&class.get("Pattern")),
            };
        }
        Self { shapes }
    }

    /// The shape of `formation`.
    pub fn shape(&self, formation: Formation) -> &FormationShape {
        &self.shapes[formation.engine_index()]
    }

    /// The slots of `formation` for units whose types have the given
    /// `(formationX, formationZ)`, one per slot in ID order (`None` for an empty slot, which
    /// counts as one formation unit). Offsets are relative to slot 0.
    pub fn slots(&self, formation: Formation, sizes: &[Option<(f64, f64)>]) -> Vec<FormationSlot> {
        let shape = self.shape(formation);
        let size = |i: usize| sizes.get(i).copied().flatten().unwrap_or((1.0, 1.0));
        let mut slots: Vec<FormationSlot> = Vec::with_capacity(sizes.len());
        for i in 0..sizes.len() {
            let (info, reference) = if i < shape.fixed.len() {
                let info = shape.fixed[i];
                (info, info.reference)
            } else if shape.pattern.is_empty() {
                (entry(-1, 0.0, 0.0, 0.0), -1)
            } else {
                let k = (i - shape.fixed.len()) % shape.pattern.len();
                let info = shape.pattern[k];
                (info, info.reference - k as i32 + i as i32)
            };
            let offset = match usize::try_from(reference).ok().filter(|&r| r < i) {
                None => DVec3::new(info.x, 0.0, info.z),
                Some(r) => {
                    let (rx, rz) = size(r);
                    let (ix, iz) = size(i);
                    let base = slots[r].offset;
                    DVec3::new(
                        base.x + info.x * (ix + rx) / 2.0,
                        0.0,
                        base.z + info.z * (iz + rz) / 2.0,
                    )
                }
            };
            slots.push(FormationSlot {
                offset,
                angle: info.angle,
            });
        }
        slots
    }
}

/// The entries of a `Fixed` / `Pattern` class: its arrays in order; a non-array entry is all
/// zero, as in the engine.
fn entries(class: &ConfigRef<'_>) -> Vec<FormationEntry> {
    class
        .entries()
        .into_iter()
        .filter(|e| !e.is_class())
        .map(|e| {
            let items = e.array();
            let n = |i: usize| match items.get(i) {
                Some(Value::Float(f)) => f64::from(*f),
                Some(Value::Int(v)) => f64::from(*v),
                _ => 0.0,
            };
            entry(n(0) as i32, n(1), n(2), n(3))
        })
        .collect()
}
