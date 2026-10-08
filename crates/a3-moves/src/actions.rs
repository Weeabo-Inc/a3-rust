//! Action maps: the classes of a moves type's `Actions`, which turn requests (`WalkF`, `Down`,
//! `Stop`, `ReloadMagazine`, ...) into the move or gesture to play in the current stance.

use std::collections::HashMap;

use crate::MoveId;

/// Index of an action map in its moves type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActionMapId(pub u32);

/// What an action asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionTarget {
    /// A move of the same moves type (`walkF = "AmovPercMwlkSrasWrflDf"`).
    Move(MoveId),
    /// A gesture of the moves type's gestures class (`reloadMagazine[] = {"Gesture...",
    /// "Gesture"}`).
    Gesture(String),
}

/// `upDegree`: the unit's posture class while in the map's moves (the config enum `ManPos*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManPos {
    Dead,
    Weapon,
    BinocLying,
    LyingNoWeapon,
    Lying,
    HandGunLying,
    Crouch,
    HandGunCrouch,
    Combat,
    HandGunStand,
    Stand,
    Swimming,
    NoWeapon,
    Binoc,
    BinocStand,
}

impl ManPos {
    /// From the config value: an enum name (`"ManPosStand"`) or its number (the root
    /// config's enum, numbers as listed on the community wiki; `-1` and unknown give `None`).
    pub fn parse(text: &str, number: Option<f32>) -> Option<ManPos> {
        const ALL: [(&str, ManPos); 15] = [
            ("manposdead", ManPos::Dead),
            ("manposweapon", ManPos::Weapon),
            ("manposbinoclying", ManPos::BinocLying),
            ("manposlyingnoweapon", ManPos::LyingNoWeapon),
            ("manposlying", ManPos::Lying),
            ("manposhandgunlying", ManPos::HandGunLying),
            ("manposcrouch", ManPos::Crouch),
            ("manposhandguncrouch", ManPos::HandGunCrouch),
            ("manposcombat", ManPos::Combat),
            ("manposhandgunstand", ManPos::HandGunStand),
            ("manposstand", ManPos::Stand),
            ("manposswimming", ManPos::Swimming),
            ("manposnoweapon", ManPos::NoWeapon),
            ("manposbinoc", ManPos::Binoc),
            ("manposbinocstand", ManPos::BinocStand),
        ];
        if let Some(n) = number {
            return usize::try_from(n as i64)
                .ok()
                .and_then(|i| ALL.get(i))
                .map(|p| p.1);
        }
        let key = text.to_ascii_lowercase();
        ALL.iter().find(|(name, _)| *name == key).map(|p| p.1)
    }
}

/// `stance` of an action map: the stance the unit counts as being in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Stance {
    #[default]
    Undefined,
    Stand,
    Crouch,
    Prone,
}

impl Stance {
    pub fn parse(text: &str) -> Stance {
        match text.to_ascii_lowercase().as_str() {
            "manstancestand" => Stance::Stand,
            "manstancecrouch" => Stance::Crouch,
            "manstanceprone" => Stance::Prone,
            _ => Stance::Undefined,
        }
    }
}

/// One class of a moves type's `Actions`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionMap {
    /// The class name in its original case.
    pub name: String,
    /// `turnSpeed`.
    pub turn_speed: f32,
    /// `limitFast`: the fastest speed (m/s) the map allows _(meaning from the wiki)_.
    pub limit_fast: f32,
    pub up_degree: Option<ManPos>,
    pub stance: Stance,
    /// `useFastMove`.
    pub use_fast_move: bool,
    /// Lean limits: `leanLRot`, `leanRRot`, `leanLShift`, `leanRShift`.
    pub lean: [f32; 4],
    pub(crate) actions: HashMap<String, ActionTarget>,
}

impl ActionMap {
    /// The target of action `name` (ignoring case), unless empty.
    pub fn get(&self, name: &str) -> Option<&ActionTarget> {
        self.actions.get(&name.to_ascii_lowercase())
    }

    /// The move of action `name`, unless empty or a gesture.
    pub fn get_move(&self, name: &str) -> Option<MoveId> {
        match self.get(name)? {
            ActionTarget::Move(m) => Some(*m),
            ActionTarget::Gesture(_) => None,
        }
    }

    /// Every non-empty action, by lower-case name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ActionTarget)> {
        self.actions.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Number of non-empty actions.
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}
