//! The [`Mission`] value: what a `mission.sqm` places, without a World.
//!
//! A mission has a brief (class `Intel`: briefing name, date, weather), groups of placed units,
//! units outside any group (`class Vehicles` at the top level), markers and triggers (`class
//! Sensors`). Nothing here touches a [`World`](a3_world::World); [`crate::spawn`] does that.
//!
//! Field names follow the SQM keys they come from, so the mapping stays checkable against a real
//! file: `azimut` is `getDir`-style degrees clockwise from north (0 = north, 90 = east; the 2D
//! editor calls the box "Azimut / Direction / Azimuth"), `text` is the editor's variable name,
//! `special` is the placement mode ("NONE", "FLY", "FORM", "CARGO"), `presence` is the editor's
//! probability-of-presence condition.

use a3_config::{Config, ConfigClass, EntryKind, Value};
use a3_sqf::Side;
use glam::DVec3;

/// The side a `side="..."` string names; [`Side::Unknown`] for anything else.
pub fn side_from_sqm(side: &str) -> Side {
    let side = side.trim();
    for candidate in [
        Side::West,
        Side::East,
        Side::Independent,
        Side::Civilian,
        Side::Logic,
        Side::Empty,
        Side::AmbientLife,
        Side::Enemy,
        Side::Friendly,
        Side::Unknown,
    ] {
        if side.eq_ignore_ascii_case(candidate.name()) {
            return candidate;
        }
    }
    match side.to_ascii_uppercase().as_str() {
        "AMBIENTLIFE" | "AMBIENT_LIFE" => Side::AmbientLife,
        _ => Side::Unknown,
    }
}

/// A parsed mission description.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mission {
    /// The virtual folder the mission was loaded from (empty when only a config was parsed).
    pub folder: String,
    /// The terrain the folder names, from its extension (`...\boot_m02.altis` → `altis`).
    pub terrain: Option<String>,
    /// `version` of the file (12 for Arma 3).
    pub version: i32,
    /// `addOns[]`: the addons the mission requires.
    pub addons: Vec<String>,
    /// `addOnsAuto[]`: the addons the editor detected.
    pub addons_auto: Vec<String>,
    /// `randomSeed`.
    pub random_seed: i32,
    /// `class Intel`.
    pub intel: Intel,
    /// `class Groups`: the placed groups, in file order.
    pub groups: Vec<MissionGroup>,
    /// `class Vehicles`: objects outside any group, in file order.
    pub objects: Vec<Unit>,
    /// `class Markers`.
    pub markers: Vec<Marker>,
    /// `class Sensors`: the placed triggers.
    pub triggers: Vec<Sensor>,
    /// The mission's `description.ext`, parsed, when it has one.
    pub description: Option<Config>,
}

/// `class Intel`: the brief.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Intel {
    /// `briefingName` (often a `$STR_` key into the stringtable).
    pub briefing_name: Option<String>,
    pub time_of_changes: Option<f64>,
    pub start_weather: Option<f64>,
    pub start_wind: Option<f64>,
    pub start_waves: Option<f64>,
    pub forecast_weather: Option<f64>,
    pub forecast_wind: Option<f64>,
    pub forecast_waves: Option<f64>,
    pub forecast_lightnings: Option<f64>,
    pub rain_forced: Option<f64>,
    pub lightnings_forced: Option<f64>,
    pub year: Option<i32>,
    pub month: Option<i32>,
    pub day: Option<i32>,
    pub hour: Option<i32>,
    pub minute: Option<i32>,
    pub start_fog_decay: Option<f64>,
    pub forecast_fog_decay: Option<f64>,
}

/// One entry of `class Groups`: a group and its waypoints.
#[derive(Debug, Clone, PartialEq)]
pub struct MissionGroup {
    /// `side`: the group's side, which the units inherit unless their own `side` differs
    /// (a soldier of another side riding in a vehicle, as in `boot_m02.altis`).
    pub side: Side,
    /// The group's `init`, when it has one (newer SQM versions).
    pub init: Option<String>,
    pub units: Vec<Unit>,
    pub waypoints: Vec<Waypoint>,
}

impl Default for MissionGroup {
    fn default() -> Self {
        Self {
            side: Side::Unknown,
            init: None,
            units: Vec::new(),
            waypoints: Vec::new(),
        }
    }
}

/// One placed unit or object.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    /// `id`: the mission's own unit id (what `idVehicle=`, waypoint `synchronizations[]` and
    /// trigger `idVehicle=` refer to).
    pub id: i32,
    /// `vehicle`: the config class (CfgVehicles) to create.
    pub class: String,
    /// `side` of the entry; [`Side::Unknown`] when absent.
    pub side: Side,
    /// `position[]` in world space ([ADR 0003](../../../docs/adr/0003-coordinates-and-precision.md)):
    /// `{x, y, z}` is east, height above sea level, north. A two-element position has no height
    /// and sets [`Unit::on_surface`].
    pub position: DVec3,
    /// The position gave no height; place the Entity on the terrain surface.
    pub on_surface: bool,
    /// `azimut`: heading in degrees clockwise from north (`getDir`).
    pub azimut: Option<f64>,
    /// `placement`, e.g. `"CAN_COLLIDE"`.
    pub placement: Option<String>,
    /// `special`: "NONE", "FLY", "FORM" or "CARGO" _(uncertain: how the engine applies it)_.
    pub special: Option<String>,
    /// `player`: `"PLAYER COMMANDER"` and friends mark the player-controlled unit.
    pub player: Option<String>,
    /// `leader=1`: this unit is the group's leader.
    pub leader: bool,
    pub rank: Option<String>,
    pub skill: Option<f64>,
    /// `text`: the editor's variable name, e.g. `BIS_lacey`.
    pub text: Option<String>,
    /// `init`: the unit's init expression, run with `this` set to the unit.
    pub init: Option<String>,
    /// `presence`: editor probability of presence (0 = absent).
    pub presence: Option<f64>,
    /// `lock`: "LOCKED", "UNLOCKED", "DEFAULT".
    pub lock: Option<String>,
}

impl Unit {
    /// Whether the editor's presence condition excludes the unit (`presence=0`, as in the
    /// campaign missions' briefing units).
    pub fn is_absent(&self) -> bool {
        self.presence == Some(0.0)
    }
}

/// One entry of a group's `class Waypoints`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Waypoint {
    /// `position[]`, as [`Unit::position`]. `[0, 0, 0]` when the waypoint has none.
    pub position: DVec3,
    pub placement: Option<String>,
    /// `type`: "MOVE", "CYCLE", "HOLD", ... (absent = "MOVE").
    pub kind: Option<String>,
    pub speed: Option<String>,
    pub combat_mode: Option<String>,
    pub behaviour: Option<String>,
    pub formation: Option<String>,
    pub description: Option<String>,
    pub show_wp: Option<String>,
    pub timeout: Option<f64>,
    /// `synchronizations[]`: the `syncId`s this waypoint is synchronized with.
    pub synchronizations: Vec<i32>,
    /// `syncId`: this waypoint's own synchronization id.
    pub sync_id: Option<i32>,
}

/// One entry of `class Markers`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Marker {
    /// `name`: the marker's variable name.
    pub name: String,
    /// `position[]`, as [`Unit::position`]; the marker's height is ignored on the map.
    pub position: DVec3,
    /// `text`: the label.
    pub text: Option<String>,
    /// `markerType`: the icon class (`mil_dot`, ...).
    pub marker_type: Option<String>,
    /// `type`: "Icon", "Rectangle", "Ellipse" or "Empty".
    pub kind: Option<String>,
    pub color: Option<String>,
    pub fill: Option<String>,
    /// `a`/`b`: the zone's semi-axes in metres, east-west and north-south at `angle = 0`.
    pub a: Option<f64>,
    pub b: Option<f64>,
    /// `angle`: degrees clockwise from north.
    pub angle: Option<f64>,
    /// `drawBorder`.
    pub draw_border: bool,
}

/// One entry of `class Sensors`: a trigger.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sensor {
    /// `name`: the trigger's variable name.
    pub name: Option<String>,
    /// `text`: the trigger's label in the editor.
    pub text: Option<String>,
    /// `position[]`, as [`Unit::position`].
    pub position: DVec3,
    /// `a`/`b`: the zone's semi-axes in metres.
    pub a: Option<f64>,
    pub b: Option<f64>,
    pub angle: Option<f64>,
    /// `type`: "SWITCH", "MUSIC", "END1".."END6", "GUARDED BY ..." ; absent = a plain trigger.
    pub kind: Option<String>,
    /// `activationBy`: "VEHICLE", "GROUP", "ANYONE", ...
    pub activation_by: Option<String>,
    /// `activationType`: "PRESENT", "NOT PRESENT", a side name, ...
    pub activation_type: Option<String>,
    pub repeating: bool,
    pub interruptable: bool,
    /// `age`: "UNKNOWN", "LEADER", ...
    pub age: Option<String>,
    /// `idVehicle`: the unit id the trigger is attached to.
    pub id_vehicle: Option<i32>,
    /// `statements` (or the older `expActiv`): the "On Activation" expression.
    pub exp_activ: Option<String>,
    /// `expCond`: the condition expression.
    pub exp_cond: Option<String>,
    /// `expDesactiv`: the "On Deactivation" expression.
    pub exp_desactiv: Option<String>,
    /// `synchronizations[]`: the `syncId`s this trigger is synchronized with.
    pub synchronizations: Vec<i32>,
    pub sync_id: Option<i32>,
}

/// Errors from building a [`Mission`].
#[derive(Debug, thiserror::Error)]
pub enum MissionError {
    /// The bytes are not a parsable `mission.sqm` ([`Mission::parse`]).
    #[error("{0}")]
    Sqm(#[from] crate::SqmError),
    /// The config has no `class Mission`.
    #[error("mission.sqm has no class Mission")]
    NoMission,
    /// A unit has no parsable `position[]={x, y, z}`.
    #[error("unit id {id} (class {class}) has no usable position[]")]
    NoPosition {
        /// The unit's `id`.
        id: i32,
        /// The unit's `vehicle` class.
        class: String,
    },
    /// A unit has no `vehicle` class name.
    #[error("unit id {id} has no vehicle class")]
    NoClass {
        /// The unit's `id`.
        id: i32,
    },
}

impl Mission {
    /// Parses the bytes of a `mission.sqm` into a [`Mission`] (no VFS, no
    /// `description.ext`).
    pub fn parse(bytes: &[u8]) -> Result<Mission, MissionError> {
        Mission::from_config(&crate::parse_sqm(bytes)?)
    }

    /// Reads the `mission.sqm`, `description.ext` and folder name of the mission in the virtual
    /// folder `folder` (e.g. `a3\missions_f_bootcamp\campaign\missions\boot_m02.altis`).
    /// `description.ext` is preprocessed and parsed through [`a3_gamedata::load_text_config`],
    /// so `__EXEC` runs on `vm`.
    pub fn load<H: a3_sqf::Host>(
        vfs: &a3_vfs::Vfs,
        folder: &str,
        vm: &mut a3_sqf::Vm<H>,
    ) -> Result<Mission, crate::LoadError> {
        crate::load::load_mission(vfs, folder, vm)
    }

    /// Builds a [`Mission`] from a parsed `mission.sqm` config.
    pub fn from_config(config: &Config) -> Result<Mission, MissionError> {
        let mission = config
            .root
            .class("Mission")
            .ok_or(MissionError::NoMission)?;
        let mut out = Mission {
            version: integer(&config.root, "version").unwrap_or(0) as i32,
            addons: strings(mission, "addOns"),
            addons_auto: strings(mission, "addOnsAuto"),
            random_seed: integer(mission, "randomSeed").unwrap_or(0) as i32,
            intel: intel(mission),
            groups: Vec::new(),
            objects: Vec::new(),
            markers: Vec::new(),
            triggers: Vec::new(),
            ..Mission::default()
        };
        if let Some(groups) = mission.class("Groups") {
            for item in items(groups) {
                out.groups.push(group(item)?);
            }
        }
        if let Some(objects) = mission.class("Vehicles") {
            for item in items(objects) {
                out.objects.push(unit(item)?);
            }
        }
        if let Some(markers) = mission.class("Markers") {
            for item in items(markers) {
                out.markers.push(marker(item));
            }
        }
        if let Some(sensors) = mission.class("Sensors") {
            for item in items(sensors) {
                out.triggers.push(sensor(item));
            }
        }
        Ok(out)
    }

    /// The player-controlled unit, if the mission has one (`player=` set on a unit).
    pub fn player(&self) -> Option<&Unit> {
        self.units().find(|u| u.player.is_some())
    }

    /// Every unit of the mission: the groups' units, then the ungrouped objects.
    pub fn units(&self) -> impl Iterator<Item = &Unit> {
        self.groups
            .iter()
            .flat_map(|g| g.units.iter())
            .chain(self.objects.iter())
    }

    /// The unit with this `id`.
    pub fn unit(&self, id: i32) -> Option<&Unit> {
        self.units().find(|u| u.id == id)
    }

    /// The group a unit id belongs to, if any.
    pub fn group_of(&self, id: i32) -> Option<&MissionGroup> {
        self.groups
            .iter()
            .find(|g| g.units.iter().any(|u| u.id == id))
    }

    /// The variables the mission defines: each named unit's `text` (the editor's variable name),
    /// and each marker's `name`. Returns `(name, kind)`, with `kind` `"unit"` (id) or `"marker"`.
    pub fn variables(&self) -> Vec<(String, MissionVariable)> {
        let mut out = Vec::new();
        for unit in self.units() {
            if let Some(name) = &unit.text {
                out.push((name.clone(), MissionVariable::Unit(unit.id)));
            }
        }
        for marker in &self.markers {
            if !marker.name.is_empty() {
                out.push((
                    marker.name.clone(),
                    MissionVariable::Marker(marker.name.clone()),
                ));
            }
        }
        out
    }
}

/// What a mission variable (a named unit or marker) is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissionVariable {
    /// A unit, by its `id`.
    Unit(i32),
    /// A marker, by its name.
    Marker(String),
}

/// The `class Intel` of a mission.
fn intel(mission: &ConfigClass) -> Intel {
    let Some(c) = mission.class("Intel") else {
        return Intel::default();
    };
    Intel {
        briefing_name: text(c, "briefingName"),
        time_of_changes: number(c, "timeOfChanges"),
        start_weather: number(c, "startWeather"),
        start_wind: number(c, "startWind"),
        start_waves: number(c, "startWaves"),
        forecast_weather: number(c, "forecastWeather"),
        forecast_wind: number(c, "forecastWind"),
        forecast_waves: number(c, "forecastWaves"),
        forecast_lightnings: number(c, "forecastLightnings"),
        rain_forced: number(c, "rainForced"),
        lightnings_forced: number(c, "lightningsForced"),
        year: integer(c, "year").map(|v| v as i32),
        month: integer(c, "month").map(|v| v as i32),
        day: integer(c, "day").map(|v| v as i32),
        hour: integer(c, "hour").map(|v| v as i32),
        minute: integer(c, "minute").map(|v| v as i32),
        start_fog_decay: number(c, "startFogDecay"),
        forecast_fog_decay: number(c, "forecastFogDecay"),
    }
}

fn group(item: &ConfigClass) -> Result<MissionGroup, MissionError> {
    let mut out = MissionGroup {
        side: item
            .get("side")
            .and_then(|e| entry_text(&e.kind))
            .map_or(Side::Unknown, |s| side_from_sqm(&s)),
        init: text(item, "init"),
        units: Vec::new(),
        waypoints: Vec::new(),
    };
    if let Some(vehicles) = item.class("Vehicles") {
        for unit_item in items(vehicles) {
            out.units.push(unit(unit_item)?);
        }
    }
    if let Some(waypoints) = item.class("Waypoints") {
        for wp in items(waypoints) {
            out.waypoints.push(waypoint(wp));
        }
    }
    Ok(out)
}

fn unit(item: &ConfigClass) -> Result<Unit, MissionError> {
    let id = integer(item, "id").unwrap_or(0) as i32;
    let class = text(item, "vehicle").unwrap_or_default();
    if class.is_empty() {
        return Err(MissionError::NoClass { id });
    }
    let (position, on_surface) = position(item).ok_or_else(|| MissionError::NoPosition {
        id,
        class: class.clone(),
    })?;
    Ok(Unit {
        id,
        class,
        side: text(item, "side").map_or(Side::Unknown, |s| side_from_sqm(&s)),
        position,
        on_surface,
        azimut: number(item, "azimut"),
        placement: text(item, "placement"),
        special: text(item, "special"),
        player: text(item, "player"),
        leader: integer(item, "leader") == Some(1),
        rank: text(item, "rank"),
        skill: number(item, "skill"),
        text: text(item, "text"),
        init: text(item, "init"),
        presence: number(item, "presence"),
        lock: text(item, "lock"),
    })
}

fn waypoint(item: &ConfigClass) -> Waypoint {
    Waypoint {
        position: position(item).map_or(DVec3::ZERO, |(p, _)| p),
        placement: text(item, "placement"),
        kind: text(item, "type"),
        speed: text(item, "speed"),
        combat_mode: text(item, "combatMode"),
        behaviour: text(item, "behaviour"),
        formation: text(item, "formation"),
        description: text(item, "description"),
        show_wp: text(item, "showWP"),
        timeout: number(item, "timeout"),
        synchronizations: integers(item, "synchronizations")
            .into_iter()
            .map(|v| v as i32)
            .collect(),
        sync_id: integer(item, "syncId").map(|v| v as i32),
    }
}

fn marker(item: &ConfigClass) -> Marker {
    Marker {
        name: text(item, "name").unwrap_or_default(),
        position: position(item).map_or(DVec3::ZERO, |(p, _)| p),
        text: text(item, "text"),
        marker_type: text(item, "markerType").or_else(|| text(item, "type")),
        kind: text(item, "type"),
        color: text(item, "colorName"),
        fill: text(item, "fillName"),
        a: number(item, "a"),
        b: number(item, "b"),
        angle: number(item, "angle"),
        draw_border: flag(item, "drawBorder"),
    }
}

fn sensor(item: &ConfigClass) -> Sensor {
    Sensor {
        name: text(item, "name"),
        text: text(item, "text"),
        position: position(item).map_or(DVec3::ZERO, |(p, _)| p),
        a: number(item, "a"),
        b: number(item, "b"),
        angle: number(item, "angle"),
        kind: text(item, "type"),
        activation_by: text(item, "activationBy"),
        activation_type: text(item, "activationType"),
        repeating: flag(item, "repeating"),
        interruptable: flag(item, "interruptable"),
        age: text(item, "age"),
        id_vehicle: integer(item, "idVehicle").map(|v| v as i32),
        exp_activ: text(item, "statements").or_else(|| text(item, "expActiv")),
        exp_cond: text(item, "expCond"),
        exp_desactiv: text(item, "expDesactiv"),
        synchronizations: integers(item, "synchronizations")
            .into_iter()
            .map(|v| v as i32)
            .collect(),
        sync_id: integer(item, "syncId").map(|v| v as i32),
    }
}

/// The `class ItemK` entries of a list class, in order; the `items=N` count is ignored in favour
/// of the entries actually present.
fn items(list: &ConfigClass) -> impl Iterator<Item = &ConfigClass> {
    list.entries.iter().filter_map(|e| match &e.kind {
        EntryKind::Class(c)
            if e.name
                .get(..4)
                .is_some_and(|p| p.eq_ignore_ascii_case("Item")) =>
        {
            Some(c)
        }
        _ => None,
    })
}

/// A `position[]` entry as world space and whether it had no height (`{x, y}`).
fn position(c: &ConfigClass) -> Option<(DVec3, bool)> {
    match entry_numbers(c, "position")?.as_slice() {
        [x, y, z, ..] => Some((DVec3::new(*x, *z, *y), false)),
        [x, y] => Some((DVec3::new(*x, 0.0, *y), true)),
        _ => None,
    }
}

fn entry_text(kind: &EntryKind) -> Option<String> {
    match kind {
        EntryKind::Value(Value::String(s) | Value::Expression(s)) => Some(s.clone()),
        _ => None,
    }
}

/// The text of `name`, or `None`.
fn text(c: &ConfigClass, name: &str) -> Option<String> {
    entry_text(&c.get(name)?.kind)
}

/// The number of `name` (float, integer, or a string that parses as a number).
fn number(c: &ConfigClass, name: &str) -> Option<f64> {
    entry_number(&c.get(name)?.kind)
}

fn entry_number(kind: &EntryKind) -> Option<f64> {
    match kind {
        EntryKind::Value(v) => match v {
            Value::Float(f) => Some(f64::from(*f)),
            Value::Int(i) => Some(f64::from(*i)),
            Value::Int64(i) => Some(*i as f64),
            Value::String(s) | Value::Expression(s) => s.trim().parse().ok(),
            Value::Array(_) => None,
        },
        _ => None,
    }
}

/// The integer of `name`.
fn integer(c: &ConfigClass, name: &str) -> Option<i64> {
    entry_integer(&c.get(name)?.kind)
}

fn entry_integer(kind: &EntryKind) -> Option<i64> {
    match kind {
        EntryKind::Value(v) => match v {
            Value::Int(i) => Some(i64::from(*i)),
            Value::Int64(i) => Some(*i),
            Value::Float(f) => Some(*f as i64),
            Value::String(s) | Value::Expression(s) => s.trim().parse().ok(),
            Value::Array(_) => None,
        },
        _ => None,
    }
}

/// Whether `name` is a non-zero number or `"true"`.
fn flag(c: &ConfigClass, name: &str) -> bool {
    match c.get(name).map(|e| &e.kind) {
        Some(EntryKind::Value(Value::String(s))) => {
            s.eq_ignore_ascii_case("true") || s.trim().parse::<f64>().is_ok_and(|v| v != 0.0)
        }
        Some(kind) => entry_number(kind).is_some_and(|v| v != 0.0),
        None => false,
    }
}

/// The strings of the `name[]={...}` array.
fn strings(c: &ConfigClass, name: &str) -> Vec<String> {
    let Some(EntryKind::Value(Value::Array(items))) = c.get(name).map(|e| &e.kind) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|v| match v {
            Value::String(s) | Value::Expression(s) => Some(s.clone()),
            Value::Float(f) => Some(f.to_string()),
            Value::Int(i) => Some(i.to_string()),
            Value::Int64(i) => Some(i.to_string()),
            Value::Array(_) => None,
        })
        .collect()
}

/// The numbers of the `name[]={...}` array.
fn entry_numbers(c: &ConfigClass, name: &str) -> Option<Vec<f64>> {
    let EntryKind::Value(Value::Array(items)) = &c.get(name)?.kind else {
        return None;
    };
    items
        .iter()
        .map(|v| match v {
            Value::Float(f) => Some(f64::from(*f)),
            Value::Int(i) => Some(f64::from(*i)),
            Value::Int64(i) => Some(*i as f64),
            Value::String(s) | Value::Expression(s) => s.trim().parse().ok(),
            Value::Array(_) => None,
        })
        .collect()
}

/// The integers of the `name[]={...}` array.
fn integers(c: &ConfigClass, name: &str) -> Vec<i64> {
    let Some(EntryKind::Value(Value::Array(items))) = c.get(name).map(|e| &e.kind) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|v| match v {
            Value::Int(i) => Some(i64::from(*i)),
            Value::Int64(i) => Some(*i),
            Value::Float(f) => Some(*f as i64),
            Value::String(s) | Value::Expression(s) => s.trim().parse().ok(),
            Value::Array(_) => None,
        })
        .collect()
}
