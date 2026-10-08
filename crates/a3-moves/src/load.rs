//! Reading a moves type from config, in the engine's order (`0x140606300`, `0x1405fe3d0`).

use std::collections::HashMap;

use a3_config::{ConfigRef, NodeId, Value};
use glam::Vec3;

use crate::actions::{ActionMap, ActionMapId, ActionTarget, ManPos, Stance};
use crate::graph::{EdgeKind, stored_cost};
use crate::{Error, Move, MoveId, Moves};

/// Action map entries that are parameters, not actions.
const ACTION_PARAMETERS: &[&str] = &[
    "access",
    "turnspeed",
    "limitfast",
    "updegree",
    "downdegree",
    "stance",
    "usefastmove",
    "leanlrot",
    "leanrrot",
    "leanlshift",
    "leanrshift",
];

fn number(v: &Value) -> Option<f32> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f32),
        Value::Int64(i) => Some(*i as f32),
        Value::String(s) | Value::Expression(s) => s.trim().parse().ok(),
        Value::Array(_) => None,
    }
}

fn text(v: &Value) -> Option<&str> {
    match v {
        Value::String(s) | Value::Expression(s) => Some(s),
        _ => None,
    }
}

fn flag(cfg: &ConfigRef<'_>, name: &str) -> bool {
    cfg.get(name).number() != 0.0
}

/// A move's RTM path as the VFS knows it.
fn rtm_path(file: &str) -> String {
    let mut p = file.trim().trim_start_matches('\\').to_ascii_lowercase();
    if !p.is_empty() && !p.rsplit('\\').next().unwrap_or("").contains('.') {
        p.push_str(".rtm");
    }
    p
}

impl Moves {
    /// Reads a moves type from its config class (`configFile >> "CfgMovesMaleSdr"`).
    pub fn from_config(cfg: &ConfigRef<'_>) -> Result<Moves, Error> {
        let states_cfg = cfg.get("States");
        if !states_cfg.is_class() {
            return Err(Error::NotAMovesClass(cfg.name().to_owned()));
        }
        let states: Vec<ConfigRef<'_>> = states_cfg
            .entries()
            .into_iter()
            .filter(ConfigRef::is_class)
            .collect();
        if states.len() > i16::MAX as usize {
            return Err(Error::TooManyMoves(states.len()));
        }
        let mut moves = Moves {
            name: cfg.name().to_owned(),
            skeleton_name: cfg.get("skeletonName").text(),
            gestures: cfg.get("gestures").text(),
            moves: Vec::with_capacity(states.len()),
            by_name: HashMap::with_capacity(states.len()),
            edges: vec![Vec::new(); states.len()],
            action_maps: Vec::new(),
            action_maps_by_name: HashMap::new(),
            primary_action_maps: Vec::new(),
            warnings: Vec::new(),
        };
        for (i, s) in states.iter().enumerate() {
            moves
                .by_name
                .entry(s.name().to_ascii_lowercase())
                .or_insert(MoveId(i as u32));
        }
        moves.read_action_maps(cfg);
        for s in &states {
            let m = moves.read_move(s);
            moves.moves.push(m);
        }
        moves.build_graph(cfg, &states);
        Ok(moves)
    }

    /// The move named by config text, warning (as the engine logs) when unknown.
    fn resolve(&mut self, name: &str, context: impl FnOnce() -> String) -> Option<MoveId> {
        if name.is_empty() {
            return None;
        }
        let id = self.find(name);
        if id.is_none() {
            let context = context();
            self.warnings
                .push(format!("unknown move {name:?} in {context}"));
        }
        id
    }

    fn read_action_maps(&mut self, cfg: &ConfigRef<'_>) {
        let classes: Vec<ConfigRef<'_>> = cfg
            .get("Actions")
            .entries_with_inherited()
            .into_iter()
            .filter(ConfigRef::is_class)
            .collect();
        for (i, c) in classes.iter().enumerate() {
            self.action_maps_by_name
                .insert(c.name().to_ascii_lowercase(), ActionMapId(i as u32));
        }
        let mut resolved = HashMap::new();
        for c in &classes {
            let up = c.get("upDegree");
            let up_degree = if up.is_number() {
                ManPos::parse("", Some(up.number()))
            } else {
                ManPos::parse(&up.text(), None)
            };
            let mut map = ActionMap {
                name: c.name().to_owned(),
                turn_speed: c.get("turnSpeed").number(),
                limit_fast: c.get("limitFast").number(),
                up_degree,
                stance: Stance::parse(&c.get("stance").text()),
                use_fast_move: flag(c, "useFastMove"),
                lean: [
                    c.get("leanLRot").number(),
                    c.get("leanRRot").number(),
                    c.get("leanLShift").number(),
                    c.get("leanRShift").number(),
                ],
                actions: HashMap::new(),
            };
            map.actions = self.resolved_actions(c, &mut resolved);
            self.action_maps.push(map);
        }
        for v in cfg.get("primaryActionMaps").array() {
            if let Some(id) = text(&v).and_then(|n| self.find_action_map(n)) {
                self.primary_action_maps.push(id);
            }
        }
    }

    /// The actions of class `c`: its base class's (memoised by config path, since thousands of
    /// maps share long base chains) overridden by its own entries.
    fn resolved_actions(
        &mut self,
        c: &ConfigRef<'_>,
        memo: &mut HashMap<NodeId, HashMap<String, ActionTarget>>,
    ) -> HashMap<String, ActionTarget> {
        if let Some(done) = c.node_path().last().and_then(|id| memo.get(id)) {
            return done.clone();
        }
        let base = c.inherits_from();
        let mut actions = if base.is_null() {
            HashMap::new()
        } else {
            self.resolved_actions(&base, memo)
        };
        for e in c.entries() {
            let key = e.name().to_ascii_lowercase();
            if e.is_class() || ACTION_PARAMETERS.contains(&key.as_str()) {
                continue;
            }
            let target = if e.is_array() {
                let items = e.array();
                let first = items.first().and_then(text).unwrap_or("").to_owned();
                let gesture = items
                    .get(1)
                    .and_then(text)
                    .is_some_and(|t| t.eq_ignore_ascii_case("Gesture"));
                if first.is_empty() {
                    None
                } else if gesture {
                    Some(ActionTarget::Gesture(first))
                } else {
                    self.resolve(&first, || format!("action {}.{}", c.name(), e.name()))
                        .map(ActionTarget::Move)
                }
            } else if e.is_text() {
                let name = e.text();
                self.resolve(&name, || format!("action {}.{}", c.name(), e.name()))
                    .map(ActionTarget::Move)
            } else {
                None
            };
            match target {
                Some(t) => {
                    actions.insert(key, t);
                }
                None => {
                    actions.remove(&key);
                }
            }
        }
        if let Some(&id) = c.node_path().last() {
            memo.insert(id, actions.clone());
        }
        actions
    }

    fn variants(&mut self, s: &ConfigRef<'_>, key: &str) -> Vec<(MoveId, f32)> {
        let items = s.get(key).array();
        let mut out = Vec::new();
        for pair in items.chunks(2) {
            let Some(name) = text(&pair[0]) else { continue };
            let p = pair.get(1).and_then(number).unwrap_or(0.0);
            if let Some(id) = self.resolve(name, || format!("{}.{key}", s.name())) {
                out.push((id, p));
            }
        }
        out
    }

    fn read_move(&mut self, s: &ConfigRef<'_>) -> Move {
        let mut speed = s.get("speed").number();
        if speed < 0.0 {
            speed = -1.0 / speed;
        }
        let actions_name = s.get("actions").text();
        let actions = self.find_action_map(&actions_name);
        if actions.is_none() && !actions_name.is_empty() {
            self.warnings.push(format!(
                "unknown action map {actions_name:?} in {}",
                s.name()
            ));
        }
        let equivalent = s.get("equivalentTo").text();
        let equivalent_to = self.resolve(&equivalent, || format!("{}.equivalentTo", s.name()));
        let variants_player = self.variants(s, "variantsPlayer");
        let variants_ai = self.variants(s, "variantsAI");
        let after: Vec<f32> = s
            .get("variantAfter")
            .array()
            .iter()
            .filter_map(number)
            .collect();
        let variant_after = [
            after.first().copied().unwrap_or(0.0),
            after.get(1).copied().unwrap_or(0.0),
            after.get(2).copied().unwrap_or(0.0),
        ];
        Move {
            name: s.name().to_owned(),
            file: rtm_path(&s.get("file").text()),
            speed,
            skill_speed_coef: {
                let c = s.get("skillSpeedCoef");
                if c.is_null() { 1.0 } else { c.number() }
            },
            looped: flag(s, "looped"),
            interpolation_speed: s.get("interpolationSpeed").number(),
            interpolation_restart: s.get("interpolationRestart").number() as i32,
            min_play_time: s.get("minPlayTime").number().clamp(0.0, 1.0),
            duty: s.get("duty").number(),
            rel_speed_min: s.get("relSpeedMin").number(),
            rel_speed_max: s.get("relSpeedMax").number(),
            terminal: flag(s, "terminal"),
            equivalent_to,
            actions,
            variants_player,
            variants_ai,
            variant_after,
            limit_gun_movement: s.get("limitGunMovement").number(),
            aiming_body: s.get("aimingBody").text(),
            can_pull_trigger: flag(s, "canPullTrigger"),
            disable_weapons: flag(s, "disableWeapons"),
            disable_weapons_long: flag(s, "disableWeaponsLong"),
            enable_optics: flag(s, "enableOptics"),
            on_ladder: flag(s, "onLadder"),
            on_land_beg: flag(s, "onLandBeg"),
            on_land_end: flag(s, "onLandEnd"),
            sound_enabled: flag(s, "soundEnabled"),
            sound_override: s.get("soundOverride").text(),
            sound_edge: s
                .get("soundEdge")
                .array()
                .iter()
                .filter_map(number)
                .collect(),
            collision_shape: s.get("collisionShape").text(),
            visible_size: s.get("visibleSize").number(),
            aim_precision: s.get("aimPrecision").number(),
            step: Vec3::ZERO,
            step_sounds: Vec::new(),
        }
    }

    /// Pairs `(name, cost)` of a state's transition list, resolved.
    fn pairs(&mut self, s: &ConfigRef<'_>, key: &str) -> Vec<(MoveId, i16)> {
        let items = s.get(key).array();
        let mut out = Vec::new();
        let mut i = 0;
        while i + 1 < items.len() {
            let cost = number(&items[i + 1]).unwrap_or(0.0);
            if let Some(name) = text(&items[i]) {
                if let Some(id) = self.resolve(name, || format!("{}.{key}", s.name())) {
                    out.push((id, stored_cost(cost)));
                }
            }
            i += 2;
        }
        out
    }

    /// Triples `(from, to, cost)` of a class-level transition list.
    fn triples(&mut self, cfg: &ConfigRef<'_>, key: &str) -> Vec<(MoveId, MoveId, i16)> {
        let items = cfg.get(key).array();
        let mut out = Vec::new();
        for t in items.chunks_exact(3) {
            let (Some(a), Some(b)) = (text(&t[0]), text(&t[1])) else {
                continue;
            };
            match (self.find(a), self.find(b)) {
                (Some(x), Some(y)) => out.push((x, y, stored_cost(number(&t[2]).unwrap_or(0.0)))),
                _ => self
                    .warnings
                    .push(format!("bad {key} transition from {a} to {b}")),
            }
        }
        out
    }

    fn build_graph(&mut self, cfg: &ConfigRef<'_>, states: &[ConfigRef<'_>]) {
        use EdgeKind::{Connect, Interpolate};

        // Per state: the moves an interpolation to which may skip `minPlayTime`.
        let mut ignore_min: Vec<Vec<MoveId>> = Vec::with_capacity(states.len());
        for s in states {
            let mut list = Vec::new();
            for v in s.get("ignoreMinPlayTime").array() {
                if let Some(id) = text(&v).and_then(|n| self.find(n)) {
                    list.push(id);
                }
            }
            ignore_min.push(list);
        }
        let ignores = |from: MoveId, to: MoveId| ignore_min[from.index()].contains(&to);

        // Interpolation groups: `{cost, move, move, ...}`, every ordered pair.
        for group in cfg.get("Interpolations").entries() {
            let items = group.array();
            let Some(cost) = items.first().and_then(number) else {
                continue;
            };
            let ids: Vec<MoveId> = items[1..]
                .iter()
                .filter_map(|v| text(v).and_then(|n| self.find(n)))
                .collect();
            for &a in &ids {
                for &b in &ids {
                    self.set_edge(b, a, Interpolate, stored_cost(cost), ignores(b, a));
                }
            }
        }
        for (a, b, c) in self.triples(cfg, "transitionsInterpolated") {
            self.set_edge(a, b, Interpolate, c, ignores(a, b));
        }
        for (a, b, c) in self.triples(cfg, "transitionsSimple") {
            self.set_edge(a, b, Connect, c, false);
        }

        for (i, s) in states.iter().enumerate() {
            let me = MoveId(i as u32);
            for (other, c) in self.pairs(s, "connectFrom") {
                self.set_edge(other, me, Connect, c, false);
            }
            for (other, c) in self.pairs(s, "connectTo") {
                self.set_edge(me, other, Connect, c, false);
            }
            for (other, c) in self.pairs(s, "interpolateWith") {
                self.set_edge(me, other, Interpolate, c, ignores(me, other));
                self.set_edge(other, me, Interpolate, c, ignores(other, me));
            }
            for (other, c) in self.pairs(s, "interpolateTo") {
                self.set_edge(me, other, Interpolate, c, ignores(me, other));
            }
            for (other, c) in self.pairs(s, "interpolateFrom") {
                self.set_edge(other, me, Interpolate, c, ignores(other, me));
            }
        }

        // `connectAs`: take over the edges of another move where this one has none.
        for (i, s) in states.iter().enumerate() {
            let me = MoveId(i as u32);
            let name = s.get("connectAs").text();
            if name.is_empty() {
                continue;
            }
            let Some(model) = self.resolve(&name, || format!("{}.connectAs", s.name())) else {
                continue;
            };
            for t in 0..states.len() {
                let t = MoveId(t as u32);
                if let Some(e) = self.edge(model, t) {
                    if self.edge(me, t).is_none() {
                        self.set_edge(me, t, e.kind, e.cost, false);
                    }
                }
            }
            for u in 0..states.len() {
                let u = MoveId(u as u32);
                if let Some(e) = self.edge(u, model) {
                    if self.edge(u, me).is_none() {
                        self.set_edge(u, me, e.kind, e.cost, false);
                    }
                }
            }
        }

        let disabled = cfg.get("transitionsDisabled").array();
        for pair in disabled.chunks_exact(2) {
            let (Some(a), Some(b)) = (text(&pair[0]), text(&pair[1])) else {
                continue;
            };
            match (self.find(a), self.find(b)) {
                (Some(x), Some(y)) => self.remove_edge(x, y),
                _ => self
                    .warnings
                    .push(format!("bad disabled transition from {a} to {b}")),
            }
        }
    }
}
