//! Character moves: one CfgMoves class (`CfgMovesMaleSdr`, `CfgMovesDog`, ...) as the engine's
//! moves type.
//!
//! - [`Moves::from_config`] reads the States (one [`Move`] each: RTM file, speed, looping,
//!   interpolation, minimum play time, variants, flags), the Action maps ([`ActionMap`]: action
//!   name → move or gesture, plus turn speed, stance) and builds the **move graph**: per move,
//!   the [`Edge`]s to the moves that may follow it, each a [`EdgeKind::Connect`] (after the move
//!   ends) or an [`EdgeKind::Interpolate`] (blend at once) with an integer cost.
//! - [`Moves::find_path`] is the engine's move path search over that graph: which moves to
//!   play, in order, to get from the current move to a requested one.
//! - [`Moves::load_rtm_headers`] reads the move vector (`step`) and step-sound keystones of
//!   every move's RTM, which movement speed and footsteps need.
//!
//! Engine behaviour (`arma3_x64.exe` 2.22): `docs/re/moves.md`.

mod actions;
mod graph;
mod load;

use std::collections::HashMap;

pub use actions::{ActionMap, ActionMapId, ActionTarget, ManPos, Stance};
pub use graph::{Edge, EdgeKind};

use glam::Vec3;

/// Index of a move (a class of the moves type's `States`), in config order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MoveId(pub u32);

impl MoveId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Errors reading a moves type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The config class is missing or has no `States` class.
    #[error("{0:?} is not a moves class (no States)")]
    NotAMovesClass(String),
    /// More moves than the engine's 16-bit move graph can address.
    #[error("{0} moves; the move graph addresses at most 32767")]
    TooManyMoves(usize),
}

/// One move: an RTM animation with the parameters the moves type gives it (a `States` class).
#[derive(Debug, Clone, PartialEq)]
pub struct Move {
    /// The class name in its original case.
    pub name: String,
    /// The RTM path in the VFS (`file`), lower case, without a leading backslash; empty for
    /// moves without an animation.
    pub file: String,
    /// Animation phase per second. Config `speed`; a negative value `-s` means a duration of
    /// `s` seconds, i.e. a speed of `1 / s`, as the engine converts it on load.
    pub speed: f32,
    /// `skillSpeedCoef` (default 1): the effective rate is `speed * (1 + (coef - 1) * s)` with
    /// `s` a global tunable, probably unit skill (`docs/re/sim-man-movement.md` §1).
    pub skill_speed_coef: f32,
    pub looped: bool,
    /// How fast the blend into this move goes, in blend weight per second.
    pub interpolation_speed: f32,
    /// `interpolationRestart` (0, 1 or 2).
    pub interpolation_restart: i32,
    /// Phase the move must reach before an interpolated transition may leave it (clamped to
    /// `0..=1`), unless the edge ignores it (`ignoreMinPlayTime[]`).
    pub min_play_time: f32,
    /// Fatigue gain while in the move (negative recovers).
    pub duty: f32,
    /// Range of the relative speed the move may be played at (`relSpeedMin`, `relSpeedMax`).
    pub rel_speed_min: f32,
    pub rel_speed_max: f32,
    /// `terminal`: a death move, nothing leaves it.
    pub terminal: bool,
    /// `equivalentTo`.
    pub equivalent_to: Option<MoveId>,
    /// The action map (`actions`) that maps player and AI requests while in this move.
    pub actions: Option<ActionMapId>,
    /// Random idle variants: `variantsPlayer[]` / `variantsAI[]` as (move, probability).
    pub variants_player: Vec<(MoveId, f32)>,
    pub variants_ai: Vec<(MoveId, f32)>,
    /// `variantAfter[]`: min, mid, max seconds before a variant plays.
    pub variant_after: [f32; 3],
    /// `limitGunMovement` (0..1; `true` is 1).
    pub limit_gun_movement: f32,
    /// `aimingBody`: BlendAnims entry name.
    pub aiming_body: String,
    pub can_pull_trigger: bool,
    pub disable_weapons: bool,
    pub disable_weapons_long: bool,
    pub enable_optics: bool,
    pub on_ladder: bool,
    pub on_land_beg: bool,
    pub on_land_end: bool,
    pub sound_enabled: bool,
    /// `soundOverride`: the step sound class to use instead of the surface's.
    pub sound_override: String,
    /// `soundEdge[]`: phases at which the move's sound plays.
    pub sound_edge: Vec<f32>,
    /// `collisionShape`: the collision model while in this move.
    pub collision_shape: String,
    /// `visibleSize`, for AI spotting.
    pub visible_size: f32,
    /// `aimPrecision`.
    pub aim_precision: f32,
    /// From the RTM header ([`Moves::load_rtm_headers`]): distance moved per animation cycle in
    /// model space. Forward moves have a negative Z (`docs/re/rtm.md`). Zero until loaded.
    pub step: Vec3,
    /// From the RTM header: phases of the `StepSound` keystones (footsteps).
    pub step_sounds: Vec<f32>,
}

/// A moves type: every [`Move`], the move graph and the action maps of one CfgMoves class.
#[derive(Debug, Clone)]
pub struct Moves {
    name: String,
    skeleton_name: String,
    gestures: String,
    moves: Vec<Move>,
    by_name: HashMap<String, MoveId>,
    edges: Vec<Vec<Edge>>,
    action_maps: Vec<ActionMap>,
    action_maps_by_name: HashMap<String, ActionMapId>,
    primary_action_maps: Vec<ActionMapId>,
    warnings: Vec<String>,
}

/// What [`Moves::load_rtm_headers`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RtmLoadReport {
    /// Distinct RTM files read.
    pub loaded: usize,
    /// Files the loader did not find.
    pub missing: Vec<String>,
    /// Files that did not parse, with the error.
    pub failed: Vec<(String, String)>,
}

impl Moves {
    /// The config class name (`CfgMovesMaleSdr`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `skeletonName`: the CfgSkeletons class the RTMs animate.
    pub fn skeleton_name(&self) -> &str {
        &self.skeleton_name
    }

    /// `gestures`: the CfgGestures class of masked upper-body animations.
    pub fn gestures(&self) -> &str {
        &self.gestures
    }

    /// Number of moves.
    pub fn len(&self) -> usize {
        self.moves.len()
    }

    pub fn is_empty(&self) -> bool {
        self.moves.is_empty()
    }

    /// The move named `name`, ignoring case.
    pub fn find(&self, name: &str) -> Option<MoveId> {
        self.by_name.get(&name.to_ascii_lowercase()).copied()
    }

    /// The move `id`. Panics on an id from another moves type that is out of range.
    pub fn get(&self, id: MoveId) -> &Move {
        &self.moves[id.index()]
    }

    /// Every move, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (MoveId, &Move)> {
        self.moves
            .iter()
            .enumerate()
            .map(|(i, m)| (MoveId(i as u32), m))
    }

    /// Every edge leaving `from`, in the order the engine added them.
    pub fn edges(&self, from: MoveId) -> &[Edge] {
        self.edges.get(from.index()).map_or(&[], Vec::as_slice)
    }

    /// The edge from `from` to `to`, if the graph has one.
    pub fn edge(&self, from: MoveId, to: MoveId) -> Option<Edge> {
        self.edges(from).iter().copied().find(|e| e.to == to)
    }

    /// Number of edges in the move graph.
    pub fn edge_count(&self) -> usize {
        self.edges.iter().map(Vec::len).sum()
    }

    /// The action map `id`.
    pub fn action_map(&self, id: ActionMapId) -> &ActionMap {
        &self.action_maps[id.0 as usize]
    }

    /// The action map named `name` (a class of the moves type's `Actions`), ignoring case.
    pub fn find_action_map(&self, name: &str) -> Option<ActionMapId> {
        self.action_maps_by_name
            .get(&name.to_ascii_lowercase())
            .copied()
    }

    /// Every action map, in id order.
    pub fn action_maps(&self) -> &[ActionMap] {
        &self.action_maps
    }

    /// `primaryActionMaps[]`.
    pub fn primary_action_maps(&self) -> &[ActionMapId] {
        &self.primary_action_maps
    }

    /// What action `action` (`WalkF`, `Down`, `Stop`, ...) does in move `current`, through the
    /// move's action map. `None` when the map has no such action or leaves it empty.
    pub fn action(&self, current: MoveId, action: &str) -> Option<&ActionTarget> {
        let map = self.get(current).actions?;
        self.action_map(map).get(action)
    }

    /// The move action `action` asks for in move `current`, unless empty or a gesture.
    pub fn action_move(&self, current: MoveId, action: &str) -> Option<MoveId> {
        match self.action(current, action)? {
            ActionTarget::Move(m) => Some(*m),
            ActionTarget::Gesture(_) => None,
        }
    }

    /// Problems found while reading the config (unknown move names and the like), as the
    /// engine would log them.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Reads the move vector and the step-sound keystones of every move's RTM. `open` returns
    /// a file's bytes by VFS path, or `None` if it is missing. Each distinct file is read once,
    /// and only its header.
    pub fn load_rtm_headers(
        &mut self,
        mut open: impl FnMut(&str) -> Option<Vec<u8>>,
    ) -> RtmLoadReport {
        let mut report = RtmLoadReport::default();
        let mut cache: HashMap<String, Option<(Vec3, Vec<f32>)>> = HashMap::new();
        for m in &mut self.moves {
            if m.file.is_empty() {
                continue;
            }
            let entry = cache.entry(m.file.clone()).or_insert_with(|| {
                let Some(bytes) = open(&m.file) else {
                    report.missing.push(m.file.clone());
                    return None;
                };
                match a3_rtm::Animation::read_header(&bytes) {
                    Ok(h) => {
                        report.loaded += 1;
                        let sounds = h
                            .keystones
                            .iter()
                            .filter(|k| k.name.eq_ignore_ascii_case("StepSound"))
                            .map(|k| k.phase)
                            .collect();
                        Some((h.step, sounds))
                    }
                    Err(e) => {
                        report.failed.push((m.file.clone(), e.to_string()));
                        None
                    }
                }
            });
            if let Some((step, sounds)) = entry {
                m.step = *step;
                m.step_sounds.clone_from(sounds);
            }
        }
        report
    }
}
