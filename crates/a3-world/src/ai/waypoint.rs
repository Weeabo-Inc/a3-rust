//! Group waypoints: what a group walks towards, and the orders a waypoint carries
//! (`docs/re/ai.md`, `docs/re/world-object-model.md`).
//!
//! A group holds a queue of waypoints and one index into it — the waypoint it is working on. The
//! first waypoint of an editor-placed group is its starting position, already completed; a group
//! made by `createGroup` starts with an empty queue. `currentWaypoint` is that index, one-based,
//! and is one past the end once every waypoint is done (0 with no waypoints at all).

use glam::DVec3;

/// How the group moves, how alert it is, and how hard it tries not to be seen (`setBehaviour`).
///
/// Behaviour overrides the combat mode, the formation and the speed while it says so: a CARELESS
/// group walks in a cluster and never opens fire, a COMBAT group moves with its weapons up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Behaviour {
    /// Non-combat moves, safest and slowest; the group does not react to contact.
    Careless,
    /// Like CARELESS, but the group turns AWARE when it detects an enemy.
    Safe,
    /// The default: keep in formation, use cover now and then.
    #[default]
    Aware,
    /// Weapon up, bounding moves, cover; the group's head is always on a swivel.
    Combat,
    /// Cover to cover, prone or crouched, slow and quiet.
    Stealth,
}

impl Behaviour {
    pub fn from_config_name(name: &str) -> Option<Behaviour> {
        Some(match name.to_ascii_uppercase().as_str() {
            "CARELESS" => Behaviour::Careless,
            "SAFE" => Behaviour::Safe,
            "AWARE" => Behaviour::Aware,
            "COMBAT" => Behaviour::Combat,
            "STEALTH" => Behaviour::Stealth,
            _ => return None,
        })
    }

    /// The name SQF and the config use (`behaviour`, `setWaypointBehaviour`).
    pub fn name(self) -> &'static str {
        match self {
            Behaviour::Careless => "CARELESS",
            Behaviour::Safe => "SAFE",
            Behaviour::Aware => "AWARE",
            Behaviour::Combat => "COMBAT",
            Behaviour::Stealth => "STEALTH",
        }
    }

    /// How far the group looks out, as a fraction of its full view range. A careless group walks
    /// with its head down and a combat group sweeps the ground ahead; `docs/re/ai.md` has the
    /// numbers and how sure we are.
    pub fn view_scale(self) -> f64 {
        match self {
            Behaviour::Careless => 0.2,
            Behaviour::Safe => 0.6,
            Behaviour::Aware => 1.0,
            Behaviour::Combat => 1.0,
            Behaviour::Stealth => 0.9,
        }
    }

    /// Whether the group keeps its weapon up: it crouches while standing and turns to face what
    /// it knows about.
    pub fn is_combat(self) -> bool {
        matches!(self, Behaviour::Combat)
    }
}

/// What the group does about an enemy it knows about (`setCombatMode`), the leader's rules of
/// engagement. YELLOW is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CombatMode {
    /// Never fire, keep formation.
    Blue,
    /// Fire only when fired upon, keep formation.
    Green,
    /// Hold fire, but move to where firing is possible.
    White,
    /// Fire at will, keep formation.
    #[default]
    Yellow,
    /// Fire at will and leave formation to hunt.
    Red,
}

impl CombatMode {
    pub fn from_config_name(name: &str) -> Option<CombatMode> {
        Some(match name.to_ascii_uppercase().as_str() {
            "BLUE" => CombatMode::Blue,
            "GREEN" => CombatMode::Green,
            "WHITE" => CombatMode::White,
            "YELLOW" => CombatMode::Yellow,
            "RED" => CombatMode::Red,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            CombatMode::Blue => "BLUE",
            CombatMode::Green => "GREEN",
            CombatMode::White => "WHITE",
            CombatMode::Yellow => "YELLOW",
            CombatMode::Red => "RED",
        }
    }

    /// Whether the group leaves its formation to close with the enemy (`engage at will`).
    pub fn breaks_formation(self) -> bool {
        matches!(self, CombatMode::Red)
    }

    /// Whether the group moves towards the enemy it knows about.
    pub fn pursues(self) -> bool {
        matches!(self, CombatMode::Red | CombatMode::White)
    }
}

/// How fast the group moves (`setSpeedMode`): LIMITED walks, NORMAL runs, FULL sprints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpeedMode {
    /// Walk — `ManInput::sprint` off.
    Limited,
    /// The default jog.
    #[default]
    Normal,
    /// As fast as the move graph goes; the same run as NORMAL until the sprint moves exist.
    Full,
}

impl SpeedMode {
    pub fn from_config_name(name: &str) -> Option<SpeedMode> {
        Some(match name.to_ascii_uppercase().as_str() {
            "LIMITED" => SpeedMode::Limited,
            "NORMAL" => SpeedMode::Normal,
            "FULL" => SpeedMode::Full,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            SpeedMode::Limited => "LIMITED",
            SpeedMode::Normal => "NORMAL",
            SpeedMode::Full => "FULL",
        }
    }
}

/// The shape a group moves in (`setFormation`). WEDGE is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Formation {
    /// A single file, one man behind the other — the tightest column.
    Column,
    /// A column with each man half a step to alternate sides.
    StagColumn,
    /// The default: an arrowhead pointing the way the leader faces.
    #[default]
    Wedge,
    /// An echelon trailing to the leader's left.
    EchLeft,
    /// An echelon trailing to the leader's right.
    EchRight,
    /// A V, open end towards the enemy.
    Vee,
    /// A line abreast.
    Line,
    /// One man wide, as COLUMN but with everyone on the leader's track.
    File,
    /// A diamond around the leader.
    Diamond,
}

/// The distance between two men of a formation, in metres. The original spreads a group over
/// roughly this much; the exact spacing depends on the men and their weapons.
pub const FORMATION_SPACING: f64 = 5.0;

impl Formation {
    pub fn from_config_name(name: &str) -> Option<Formation> {
        Some(match name.to_ascii_uppercase().as_str() {
            "COLUMN" => Formation::Column,
            "STAG COLUMN" => Formation::StagColumn,
            "WEDGE" => Formation::Wedge,
            "ECH LEFT" => Formation::EchLeft,
            "ECH RIGHT" => Formation::EchRight,
            "VEE" => Formation::Vee,
            "LINE" => Formation::Line,
            "FILE" => Formation::File,
            "DIAMOND" => Formation::Diamond,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Formation::Column => "COLUMN",
            Formation::StagColumn => "STAG COLUMN",
            Formation::Wedge => "WEDGE",
            Formation::EchLeft => "ECH LEFT",
            Formation::EchRight => "ECH RIGHT",
            Formation::Vee => "VEE",
            Formation::Line => "LINE",
            Formation::File => "FILE",
            Formation::Diamond => "DIAMOND",
        }
    }

    /// Where unit `index` (0 is the leader) stands, in the leader's frame: `x` to his right, `z`
    /// ahead of him. The leader is always at the origin.
    pub fn offset(self, index: usize) -> DVec3 {
        if index == 0 {
            return DVec3::ZERO;
        }
        let step = FORMATION_SPACING;
        // Followers number from 1; the wings of a symmetric shape take them two at a time.
        let pair = index.div_ceil(2) as f64;
        let side = if index % 2 == 1 { -1.0 } else { 1.0 };
        match self {
            Formation::Column | Formation::File => DVec3::new(0.0, 0.0, -step * index as f64),
            Formation::StagColumn => DVec3::new(side * step * 0.5, 0.0, -step * index as f64),
            Formation::Wedge => DVec3::new(side * step * pair, 0.0, -step * pair),
            Formation::Vee => DVec3::new(side * step * pair, 0.0, -step * pair + step),
            Formation::EchLeft => DVec3::new(-step * index as f64, 0.0, -step * index as f64),
            Formation::EchRight => DVec3::new(step * index as f64, 0.0, -step * index as f64),
            Formation::Line => DVec3::new(side * step * pair, 0.0, 0.0),
            Formation::Diamond => match index % 4 {
                // Ahead, right, left, behind — the leader's own slot is the empty middle.
                1 => DVec3::new(0.0, 0.0, step),
                2 => DVec3::new(step, 0.0, 0.0),
                3 => DVec3::new(-step, 0.0, 0.0),
                _ => DVec3::new(0.0, 0.0, -step),
            },
        }
    }
}

/// What a waypoint asks a group to do when it is reached (`setWaypointType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WaypointType {
    /// Walk there. MOVE completes when the group leader arrives.
    #[default]
    Move,
    /// Move in, then destroy what is at the position.
    Destroy,
    /// Board a vehicle at the position.
    GetIn,
    /// Search and destroy: move there, then sweep for enemies.
    Sad,
    /// Join the group of the unit the waypoint is attached to.
    Join,
    /// Take over as leader.
    Leader,
    /// Leave the vehicle.
    GetOut,
    /// Back to the first waypoint of the queue once this one is done.
    Cycle,
    /// Load cargo into a vehicle.
    Load,
    /// Unload cargo from a vehicle.
    Unload,
    /// Transfer between vehicles.
    TrUnload,
    /// Wait here, indefinitely.
    Hold,
    /// Watch the waypoint's surroundings from here, indefinitely.
    Sentry,
    /// Guard here, chasing what comes close.
    Guard,
    /// Talk to the unit the waypoint is attached to.
    Talk,
    /// Run the waypoint's script.
    Scripted,
    /// Support a unit.
    Support,
    /// Board the nearest vehicle that can take the group.
    GetInNearest,
    /// Dismiss the group (milSim: the men leave the group).
    Dismiss,
    /// Loiter at the position: on foot like MOVE, in the air a circle.
    Loiter,
    /// Hook cargo.
    Hook,
    /// Unhook cargo.
    Unhook,
    /// AND: continue when this and the next waypoint's conditions hold.
    And,
    /// OR: continue when either this or the next waypoint's conditions hold.
    Or,
}

impl WaypointType {
    /// The type named by a string, ignoring case, as `setWaypointType` accepts it.
    pub fn from_config_name(name: &str) -> Option<WaypointType> {
        Some(match name.to_ascii_uppercase().as_str() {
            "MOVE" => WaypointType::Move,
            "DESTROY" => WaypointType::Destroy,
            "GETIN" => WaypointType::GetIn,
            "SAD" => WaypointType::Sad,
            "JOIN" => WaypointType::Join,
            "LEADER" => WaypointType::Leader,
            "GETOUT" => WaypointType::GetOut,
            "CYCLE" => WaypointType::Cycle,
            "LOAD" => WaypointType::Load,
            "UNLOAD" => WaypointType::Unload,
            "TR UNLOAD" => WaypointType::TrUnload,
            "HOLD" => WaypointType::Hold,
            "SENTRY" => WaypointType::Sentry,
            "GUARD" => WaypointType::Guard,
            "TALK" => WaypointType::Talk,
            "SCRIPTED" => WaypointType::Scripted,
            "SUPPORT" => WaypointType::Support,
            "GETIN NEAREST" => WaypointType::GetInNearest,
            "DISMISS" => WaypointType::Dismiss,
            "LOITER" => WaypointType::Loiter,
            "HOOK" => WaypointType::Hook,
            "UNHOOK" => WaypointType::Unhook,
            "AND" => WaypointType::And,
            "OR" => WaypointType::Or,
            _ => return None,
        })
    }

    /// The name SQF uses (`waypointType`).
    pub fn name(self) -> &'static str {
        match self {
            WaypointType::Move => "MOVE",
            WaypointType::Destroy => "DESTROY",
            WaypointType::GetIn => "GETIN",
            WaypointType::Sad => "SAD",
            WaypointType::Join => "JOIN",
            WaypointType::Leader => "LEADER",
            WaypointType::GetOut => "GETOUT",
            WaypointType::Cycle => "CYCLE",
            WaypointType::Load => "LOAD",
            WaypointType::Unload => "UNLOAD",
            WaypointType::TrUnload => "TR UNLOAD",
            WaypointType::Hold => "HOLD",
            WaypointType::Sentry => "SENTRY",
            WaypointType::Guard => "GUARD",
            WaypointType::Talk => "TALK",
            WaypointType::Scripted => "SCRIPTED",
            WaypointType::Support => "SUPPORT",
            WaypointType::GetInNearest => "GETIN NEAREST",
            WaypointType::Dismiss => "DISMISS",
            WaypointType::Loiter => "LOITER",
            WaypointType::Hook => "HOOK",
            WaypointType::Unhook => "UNHOOK",
            WaypointType::And => "AND",
            WaypointType::Or => "OR",
        }
    }

    /// Whether reaching the position is enough to finish the waypoint. HOLD, SENTRY, GUARD and
    /// SUPPORT are never finished by arriving — the group waits there until something else (a
    /// timeout, a statement, a script) moves it on.
    pub fn completes_on_arrival(self) -> bool {
        match self {
            WaypointType::Hold
            | WaypointType::Sentry
            | WaypointType::Guard
            | WaypointType::Support
            | WaypointType::And
            | WaypointType::Or => false,
            // Vehicle waypoints wait for a vehicle that this engine does not have yet
            // (#124/#127); the group waits on the spot rather than completing them.
            WaypointType::GetIn
            | WaypointType::GetOut
            | WaypointType::Load
            | WaypointType::Unload
            | WaypointType::TrUnload
            | WaypointType::Hook
            | WaypointType::Unhook
            | WaypointType::GetInNearest => false,
            _ => true,
        }
    }

    /// Waypoints SAD finishes only once there is nothing left to hunt nearby.
    pub fn needs_a_clear_area(self) -> bool {
        matches!(self, WaypointType::Sad | WaypointType::Destroy)
    }
}

/// Where a LOITER waypoint sends a flyer (`setWaypointLoiterType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoiterType {
    /// Straight legs through the position.
    #[default]
    Linear,
    /// A circle around the position.
    Circle,
}

impl LoiterType {
    pub fn from_config_name(name: &str) -> Option<LoiterType> {
        Some(match name.to_ascii_uppercase().as_str() {
            "LINEAR" => LoiterType::Linear,
            "CIRCLE" => LoiterType::Circle,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            LoiterType::Linear => "LINEAR",
            LoiterType::Circle => "CIRCLE",
        }
    }
}

/// The radius a MOVE waypoint uses when the mission asked for none: the group stops a few metres
/// short instead of trying to stand on the exact spot (`docs/re/ai.md`).
pub const DEFAULT_COMPLETION_RADIUS: f64 = 3.0;

/// One waypoint of a group's queue.
///
/// The mode fields are `None` when the waypoint says "no change" (`UNCHANGED`, or simply unset):
/// only the ones it does set are applied to the group when the waypoint becomes active.
#[derive(Debug, Clone, PartialEq)]
pub struct Waypoint {
    pub waypoint_type: WaypointType,
    /// Where on the map it is, in world metres.
    pub position: DVec3,
    /// `setWaypointBehaviour`.
    pub behaviour: Option<Behaviour>,
    /// `setWaypointCombatMode`.
    pub combat_mode: Option<CombatMode>,
    /// `setWaypointSpeed`.
    pub speed_mode: Option<SpeedMode>,
    /// `setWaypointFormation`.
    pub formation: Option<Formation>,
    /// How close the group has to get, in metres; 0 means the engine picks
    /// ([`DEFAULT_COMPLETION_RADIUS`] for infantry).
    pub completion_radius: f64,
    /// `setWaypointTimeout [min, mid, max]`, in seconds. The group finishes the waypoint after
    /// `mid` seconds on it; the middle value is the engine's expectation, and this engine has no
    /// random source to spread between the three (yet).
    pub timeout: [f64; 3],
    /// `setWaypointDescription`, on the map.
    pub description: String,
    /// `setWaypointName`.
    pub name: String,
    /// `setWaypointVisible`: whether the waypoint is shown on the map.
    pub visible: bool,
    /// `setWaypointScript`: the file to run when the waypoint behaves as set, kept as the path
    /// the mission gave (this engine does not run scripts from the AI tick yet).
    pub script: String,
    /// `setWaypointStatements [condition, statement, ...]`: pairs of SQF source text. Kept as
    /// text — the compiled form belongs to the VM that runs it.
    pub statements: Vec<(String, String)>,
    /// `setWaypointHousePosition`: the position within the building the group should take.
    pub house_position: i64,
    /// `setWaypointLoiterRadius`.
    pub loiter_radius: f64,
    /// `setWaypointLoiterType`.
    pub loiter_type: LoiterType,
}

impl Default for Waypoint {
    fn default() -> Self {
        Self {
            waypoint_type: WaypointType::default(),
            position: DVec3::ZERO,
            behaviour: None,
            combat_mode: None,
            speed_mode: None,
            formation: None,
            completion_radius: 0.0,
            timeout: [0.0, 0.0, 0.0],
            description: String::new(),
            name: String::new(),
            visible: true,
            script: String::new(),
            statements: Vec::new(),
            house_position: 0,
            loiter_radius: 0.0,
            loiter_type: LoiterType::default(),
        }
    }
}

impl Waypoint {
    /// A waypoint of `waypoint_type` at `position`, with everything else at its default.
    pub fn new(waypoint_type: WaypointType, position: DVec3) -> Waypoint {
        Waypoint {
            waypoint_type,
            position,
            ..Default::default()
        }
    }

    /// How close the group has to get: the radius the mission set, or the engine's default when
    /// it set none.
    pub fn effective_completion_radius(&self) -> f64 {
        if self.completion_radius > 0.0 {
            self.completion_radius
        } else {
            DEFAULT_COMPLETION_RADIUS
        }
    }

    /// Whether the waypoint's timeout has run out after `elapsed` seconds on it. A timeout of
    /// zero (the default) never runs out.
    pub fn timed_out(&self, elapsed: f64) -> bool {
        self.timeout[1] > 0.0 && elapsed >= self.timeout[1]
    }
}

/// What a group's waypoint queue is doing: which waypoint it is on, and whether that waypoint is
/// still the one it started on.
#[derive(Debug, Clone, PartialEq)]
pub struct WaypointQueue {
    /// Index of the waypoint the group is working on; equal to the queue length once every
    /// waypoint is done.
    pub current: usize,
    /// When the group started on the current waypoint, in World time — the clock the waypoint's
    /// timeout runs on.
    pub started: f64,
}

impl Default for WaypointQueue {
    fn default() -> Self {
        WaypointQueue {
            current: 0,
            started: 0.0,
        }
    }
}
