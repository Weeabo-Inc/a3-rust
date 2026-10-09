//! Per-object state scripts set and read: captive, unit position, AI feature switches, skill,
//! rank, fuel and supply cargo, locks, engine, texture and material overrides, and the cargo of
//! vehicles and boxes. See `docs/re/sqf-object-state.md`.
//!
//! The values live in a side table of the World ([`ObjectState`], one per Entity, created on
//! first change); the defaults come from the Entity's config where the original reads them
//! there.

use crate::inventory::Magazine;
use crate::{EntityId, World};

/// `unitPos` / `setUnitPos`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnitPos {
    #[default]
    Auto,
    Up,
    Down,
    Middle,
}

impl UnitPos {
    pub fn name(self) -> &'static str {
        match self {
            UnitPos::Auto => "Auto",
            UnitPos::Up => "Up",
            UnitPos::Down => "Down",
            UnitPos::Middle => "Middle",
        }
    }

    /// The enum value of a name, any case.
    pub fn parse(name: &str) -> Option<UnitPos> {
        Some(match name.to_ascii_uppercase().as_str() {
            "AUTO" => UnitPos::Auto,
            "UP" => UnitPos::Up,
            "DOWN" => UnitPos::Down,
            "MIDDLE" => UnitPos::Middle,
            _ => return None,
        })
    }
}

/// The AI feature bits of `disableAI` / `enableAI` / `checkAIFeature` (the enum registered at
/// 0x1400e7b30).
pub const AI_FEATURES: [(&str, u32); 21] = [
    ("TARGET", 0x1),
    ("MOVE", 0x2),
    ("AUTOTARGET", 0x4),
    ("ANIM", 0x8),
    ("TEAMSWITCH", 0x10),
    ("FSM", 0x40),
    ("WEAPONAIM", 0x80),
    ("AIMINGERROR", 0x100),
    ("SUPPRESSION", 0x200),
    ("CHECKVISIBLE", 0x400),
    ("COVER", 0x800),
    ("AUTOCOMBAT", 0x1000),
    ("PATH", 0x2000),
    ("MINEDETECTION", 0x4000),
    ("NVG", 0x8000),
    ("LIGHTS", 0x10000),
    ("RADIOPROTOCOL", 0x20000),
    ("FIREWEAPON", 0x40000),
    ("COMMAND", 0x80000),
    ("HEARING", 0x100000),
    ("ALL", 0xffff_ffff),
];

/// The bits of an AI feature name, any case.
pub fn ai_feature(name: &str) -> Option<u32> {
    AI_FEATURES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|&(_, bits)| bits)
}

/// Unit ranks (`rank`, `rankId`, `setRank`).
pub const RANKS: [&str; 7] = [
    "PRIVATE",
    "CORPORAL",
    "SERGEANT",
    "LIEUTENANT",
    "CAPTAIN",
    "MAJOR",
    "COLONEL",
];

/// A vehicle's or box's cargo: items, magazines, weapons and backpacks, each in the order added.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cargo {
    pub items: Vec<String>,
    pub magazines: Vec<Magazine>,
    pub weapons: Vec<String>,
    pub backpacks: Vec<String>,
}

/// Script-set state of one Entity. `None` fields have their default (from the config).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ObjectState {
    /// `captiveNum`; captive when not 0.
    pub captive: i32,
    pub unit_pos: UnitPos,
    /// Disabled AI feature bits.
    pub ai_disabled: u32,
    /// General skill; `None` is 0.5.
    pub skill: Option<f32>,
    /// Sub-skill overrides (`setSkill [name, value]`), lower-case names.
    pub sub_skills: Vec<(String, f32)>,
    pub rank: usize,
    pub fuel: Option<f32>,
    pub fuel_cargo: Option<f32>,
    pub ammo_cargo: Option<f32>,
    pub repair_cargo: Option<f32>,
    /// `locked` value; `None` is 1 (default).
    pub lock: Option<i32>,
    pub driver_locked: bool,
    /// All cargo seats locked (`lockCargo true`), with per-seat exceptions.
    pub cargo_locked: bool,
    pub cargo_seat_locks: Vec<(i32, bool)>,
    pub engine_on: bool,
    /// Texture and material overrides per hidden selection index.
    pub textures: Vec<(usize, String)>,
    pub materials: Vec<(usize, String)>,
    /// Cargo, created from the config on first use.
    pub cargo: Option<Cargo>,
    /// `createSimpleObject`: a local object with no simulation and no network (the engine's simple
    /// objects are a client-side render path; we keep an ordinary Entity that does not simulate).
    pub simple: bool,
    /// `enableStamina`: stored, nothing consumes it yet (there is no stamina model).
    pub stamina: bool,
    /// `enableAttack`: stored, nothing consumes it yet.
    pub attack: bool,
    /// `createDiaryRecord`: `[subject, text]` in the order added; no diary UI or handle table yet.
    pub diary: Vec<(String, String)>,
    /// `animate`: the phase of each selection animation, by name.
    pub animations: Vec<(String, f32)>,
    /// `animateSource`: the phase of each named animation source.
    pub sources: Vec<(String, f32)>,
    /// `animateDoor`: the phase of each door.
    pub doors: Vec<(String, f32)>,
    /// `action`: the action the object was told to play, when it is not a Man whose move state
    /// machine plays it.
    pub action: Option<String>,
    /// `moveInDriver` / `moveInGunner`: the unit in the seat.
    pub driver: Option<EntityId>,
    pub gunner: Option<EntityId>,
    /// `moveInCargo`: the unit in each cargo seat, by seat index.
    pub cargo_seats: Vec<(i32, EntityId)>,
    /// `assignAsCargo`: the seat a unit boards when he gets in.
    pub assigned_cargo: Vec<(EntityId, i32)>,
    /// `allowCrewInImmobile`: the whole crew, or (`_cargo`) the cargo seats only.
    pub crew_in_immobile: bool,
    pub crew_in_immobile_cargo: bool,
    /// `setVehicleAmmo`: the ammo fraction, 0..1.
    pub vehicle_ammo: Option<f32>,
}

impl ObjectState {
    /// `skill unit`.
    pub fn skill(&self) -> f32 {
        self.skill.unwrap_or(0.5)
    }

    /// `unit skill name`: the override, else the general skill.
    pub fn sub_skill(&self, name: &str) -> f32 {
        self.sub_skills
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map_or(self.skill(), |&(_, v)| v)
    }

    pub fn set_sub_skill(&mut self, name: &str, value: f32) {
        let name = name.to_ascii_lowercase();
        match self.sub_skills.iter_mut().find(|(n, _)| *n == name) {
            Some(entry) => entry.1 = value,
            None => self.sub_skills.push((name, value)),
        }
    }

    /// Whether cargo seat `index` is locked.
    pub fn cargo_seat_locked(&self, index: i32) -> bool {
        self.cargo_seat_locks
            .iter()
            .rev()
            .find(|(i, _)| *i == index)
            .map_or(self.cargo_locked, |&(_, l)| l)
    }

    pub fn set_override(list: &mut Vec<(usize, String)>, index: usize, value: String) {
        match list.iter_mut().find(|(i, _)| *i == index) {
            Some(entry) => entry.1 = value,
            None => list.push((index, value)),
        }
    }

    pub fn override_at(list: &[(usize, String)], index: usize) -> Option<&str> {
        list.iter()
            .find(|(i, _)| *i == index)
            .map(|(_, v)| v.as_str())
    }

    /// `animate` (`Animation`), `animateSource` (`Source`) or `animateDoor` (`Door`): the phase of
    /// a named animation, the last one set winning.
    pub fn set_phase(list: &mut Vec<(String, f32)>, name: &str, phase: f32) {
        match list.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some(entry) => entry.1 = phase,
            None => list.push((name.to_owned(), phase)),
        }
    }

    /// The phase `animate`/`animateSource`/`animateDoor` set, 0 without one (`animationPhase`,
    /// `animationSourcePhase`, `doorPhase`).
    pub fn phase_at(list: &[(String, f32)], name: &str) -> f32 {
        list.iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map_or(0.0, |&(_, phase)| phase)
    }
}

impl World {
    /// The unit's `captiveNum` (`setCaptive`): above 0 while it is captive, 0 otherwise. A
    /// captive unit's `side` is civilian; its group keeps its side.
    pub fn captive(&self, unit: EntityId) -> i32 {
        self.object_state(unit).map_or(0, |s| s.captive)
    }
}
