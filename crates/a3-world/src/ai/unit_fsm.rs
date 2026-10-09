//! The unit's formation FSM: the native `CfgFSMs` FSM his type's `fsmFormation` names (the
//! soldiers' `Formation`), stepped in his think, and the engine's Man / Entity functions it
//! calls (`docs/re/ai-fsm.md` §2, §3.4).
//!
//! Without a cover search (not traced), `coverReached` only holds when cover is switched off
//! (`disableAI "COVER"`), as in the engine; so a man in COMBAT cycles through his path search
//! and stays in formation, as an engine man without cover nearby does.

use a3_fsm::{Action, Condition, Driver};
use glam::DVec3;

use super::{AiFeatures, Behaviour, PlanningMode, UnitFsm, UnitPos};
use crate::{EntityId, GroupId, World};

/// What the formation FSM keeps about covering (the `AIUnit+0xd614..` fields).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CoverState {
    /// When he went into cover (`ProvideCover`).
    pub since: f64,
    /// When he last picked a target to cover (`randomDelay` counts from here).
    pub target_time: f64,
    /// The least time he stays in cover, s (`3 + 5u`).
    pub min_time: f64,
    /// The most time he stays in cover, s (`min + 5 + 10u`).
    pub max_time: f64,
    /// How long `randomDelay` waits, s.
    pub delay: f64,
    /// The five points he covers (aim points ahead of him).
    pub aim_points: Vec<DVec3>,
    /// Which aim point he covers now.
    pub aim: usize,
    /// The FSM-driven fire/cover state (`EntityAI+0xa14`: 0 hold, 1 after clean-up, 2
    /// covering).
    pub mode: u8,
    /// The cover search radii `searchPath` set (`AIUnit+0xd4c8`, `+0xd4cc`).
    pub search: (f64, f64),
    /// A cover position reached and not yet reported (`Man+0xf92`, read once).
    pub reached: bool,
}

impl World {
    /// The unit's think, FSM part: creates his formation FSM on first use and steps it, unless
    /// `disableAI "FSM"` (`AIUnit_Think`).
    pub(crate) fn think_unit(&mut self, group: GroupId, unit: EntityId) {
        let Some(entity) = self.entity(unit) else {
            return;
        };
        if !entity.is_alive() || !entity.is_local() {
            return;
        }
        let name = entity.entity_type().ai().fsm_formation.to_ascii_lowercase();
        if self.ai_disabled(unit).has(AiFeatures::FSM) {
            return;
        }
        let Some(man) = self.man(unit) else {
            return;
        };
        if !man.ai.fsm_looked_up {
            let fsm = self.ai.native_fsms.get(&name).cloned();
            let man = self.man_mut(unit).expect("checked");
            man.ai.fsm_looked_up = true;
            man.ai.fsm = fsm.map(|fsm| UnitFsm {
                machine: a3_fsm::Machine::new(&fsm),
                fsm,
            });
        }
        let Some(mut running) = self.man_mut(unit).and_then(|m| m.ai.fsm.take()) else {
            return;
        };
        let mut driver = ManDriver {
            world: self,
            group,
            unit,
        };
        running.machine.step(&running.fsm, &mut driver);
        if let Some(man) = self.man_mut(unit) {
            man.ai.fsm = Some(running);
        }
    }

    fn unit_behaviour(&self, group: GroupId) -> Behaviour {
        self.group(group)
            .map_or(Behaviour::default(), |g| g.ai.behaviour)
    }

    /// Signed distance of a unit behind his formation slot along the formation direction
    /// (`FUN_141347350`; positive behind).
    fn behind_slot(&self, group: GroupId, unit: EntityId) -> f64 {
        let Some(g) = self.group(group) else {
            return 0.0;
        };
        let units = g.units.clone();
        let Some(index) = units.iter().position(|u| *u == unit) else {
            return 0.0;
        };
        let slots = self.formation_positions(group, &units);
        let direction = self.group_direction(group).unwrap_or(DVec3::Z);
        match (slots.get(index), self.entity(unit)) {
            (Some(slot), Some(e)) => (*slot - e.position()).dot(direction),
            _ => 0.0,
        }
    }
}

/// Runs the formation FSM's functions against the World for one unit.
struct ManDriver<'a> {
    world: &'a mut World,
    group: GroupId,
    unit: EntityId,
}

impl ManDriver<'_> {
    fn now(&self) -> f64 {
        self.world.time()
    }

    fn rand(&mut self) -> f64 {
        self.world.random()
    }

    fn cover(&mut self) -> Option<&mut CoverState> {
        self.world.man_mut(self.unit).map(|m| &mut m.ai.cover)
    }

    fn set_fsm_pos(&mut self, pos: UnitPos) {
        if let Some(man) = self.world.man_mut(self.unit) {
            man.ai.fsm_unit_pos = pos;
        }
    }

    fn current_pos(&self) -> UnitPos {
        self.world.effective_unit_pos(self.unit)
    }

    fn set_mode(&mut self, mode: u8) {
        if let Some(cover) = self.cover() {
            cover.mode = mode;
        }
    }

    fn behaviour(&self) -> Behaviour {
        self.world.unit_behaviour(self.group)
    }

    /// `FUN_141347a10`: 1 when the group met danger in the last 30 s, else 0.5.
    fn danger_factor(&self) -> f64 {
        let now = self.now();
        let recent = self
            .world
            .group(self.group)
            .and_then(|g| g.ai.last_danger)
            .is_some_and(|t| t > now - 30.0);
        if recent { 1.0 } else { 0.5 }
    }

    /// The five aim points `dist` metres ahead, `height` up, `spread` to the sides.
    fn aim_points(&self, dist: f64, height: f64, spread: f64) -> Vec<DVec3> {
        let Some(e) = self.world.entity(self.unit) else {
            return Vec::new();
        };
        let leader_dir = self.world.group_direction(self.group);
        let dir = leader_dir.unwrap_or_else(|| super::movement::direction_of(e.heading()));
        let perp = DVec3::new(-dir.z, 0.0, dir.x);
        let up = DVec3::Y * height;
        let c = e.position() + dir * dist + up;
        vec![
            c,
            c + perp * spread + up,
            c - perp * spread + up,
            c + perp * spread - up,
            c - perp * spread - up,
        ]
    }

    // ---- Conditions (`Man_CreateFSMConditionFunc`, then Entity). ----

    fn condition_value(&mut self, name: &str, parameters: &[f32]) -> f32 {
        let b = |v: bool| if v { 1.0 } else { 0.0 };
        match name.to_ascii_lowercase().as_str() {
            "true" => 1.0,
            "false" => 0.0,
            "const" => parameters.first().copied().unwrap_or(0.0),
            "fsmfinished" => 1.0,
            "behaviourcombat" => b(self.behaviour().is_combat()),
            // On foot only: no vehicles yet.
            "vehicleair" | "vehicle" => 0.0,
            "formationisleader" => b(self.world.man(self.unit).is_some_and(|m| {
                matches!(
                    m.ai.planning,
                    PlanningMode::LeaderDirect | PlanningMode::VehiclePlanned
                )
            })),
            // No weapons yet (#127): nothing to reload.
            "reloadneeded" => 0.0,
            "coverreached" => {
                let cover_off = self.world.ai_disabled(self.unit).has(AiFeatures::COVER);
                let reached = self.cover().is_some_and(|c| std::mem::take(&mut c.reached));
                b(cover_off || reached)
            }
            "randomdelay" => {
                let now = self.now();
                b(self
                    .world
                    .man(self.unit)
                    .is_some_and(|m| now - m.ai.cover.target_time >= m.ai.cover.delay))
            }
            "formationcanleavecover" => b(self.can_leave_cover()),
            _ => 0.0,
        }
    }

    /// `AIUnit::FSMFormationCanLeaveCover` in outline (`docs/re/ai-fsm.md` §2.3): out of cover
    /// after the minimum time scaled by how far behind his slot he is, or after three times the
    /// maximum.
    fn can_leave_cover(&self) -> bool {
        let Some(e) = self.world.entity(self.unit) else {
            return true;
        };
        let f = e.entity_type().ai().formation_z;
        let behind = self.world.behind_slot(self.group, self.unit);
        let t = (1.0 - behind / (8.0 * f)).clamp(0.1, 1.0);
        let Some(man) = self.world.man(self.unit) else {
            return true;
        };
        let in_cover = self.now() - man.ai.cover.since;
        in_cover >= t * man.ai.cover.min_time || in_cover >= 3.0 * man.ai.cover.max_time
    }

    // ---- Actions (`Man_CreateFSMActionFunc`, then Entity; every exit is empty). ----

    fn run_action(&mut self, name: &str, parameters: &[f32]) {
        match name.to_ascii_lowercase().as_str() {
            "formationexcluded" => {
                self.set_fsm_pos(UnitPos::Auto);
                self.set_mode(0);
            }
            "searchpath" => self.search_path(parameters),
            "formationprovidecover" => self.provide_cover(),
            "formationnexttarget" => self.next_target(),
            "formationhideincover" => {
                self.next_target();
                self.set_mode(0);
            }
            "formationleader" => self.set_mode(0),
            "setunitpostodown" => self.set_fsm_pos(UnitPos::Down),
            // Reloading needs weapons (#127).
            "reload" => self.set_mode(0),
            "formationcleanup" => {
                self.set_fsm_pos(UnitPos::Auto);
                self.set_mode(1);
                if let Some(cover) = self.cover() {
                    cover.aim_points.clear();
                }
            }
            // `formationInit`, `nothing`, `createFSM`, `deleteFSM` and unknown names do nothing.
            _ => {}
        }
    }

    /// `searchPath {a, b}`: the cover search radii, scaled up with the distance behind the
    /// slot, and a new path to the slot. The cover query itself is not traced, so no cover is
    /// found.
    fn search_path(&mut self, parameters: &[f32]) {
        let a = f64::from(parameters.first().copied().unwrap_or(0.0));
        let b = f64::from(parameters.get(1).copied().unwrap_or(0.0));
        let d = self.world.behind_slot(self.group, self.unit);
        let fa = if d < 30.0 {
            1.0
        } else if d <= 300.0 {
            1.0 + (d - 30.0) * 0.040_740_74
        } else {
            12.0
        };
        let fb = if d < 30.0 {
            1.0
        } else if d <= 300.0 {
            1.0 + (d - 30.0) * 0.003_703_703_6
        } else {
            2.0
        };
        if let Some(man) = self.world.man_mut(self.unit) {
            man.ai.cover.search = (a * fa, b * fb);
            if man.ai.cover.mode == 2 {
                man.ai.cover.mode = 1;
            }
        }
    }

    /// `formationProvideCover`: into cover for a random time, a stance and five points to
    /// cover ahead.
    fn provide_cover(&mut self) {
        let now = self.now();
        let min_time = 3.0 + 5.0 * self.rand();
        let max_time = min_time + 5.0 + 10.0 * self.rand();
        let down = self.current_pos() == UnitPos::Down || self.rand() <= 0.5;
        let (pos, height, spread) = if down {
            (UnitPos::Down, 0.5, 4.0)
        } else {
            (UnitPos::Middle, 1.5, 6.0)
        };
        self.set_fsm_pos(pos);
        let points = self.aim_points(15.0, height, spread);
        let delay = (0.5 + 4.0 * self.rand()) * self.danger_factor() + 0.5;
        if let Some(cover) = self.cover() {
            cover.since = now;
            cover.target_time = now;
            cover.min_time = min_time;
            cover.max_time = max_time;
            cover.aim_points = points;
            cover.aim = 0;
            cover.delay = delay;
            cover.mode = 2;
        }
    }

    /// `formationNextTarget`: maybe down from a crouch, and the next point to cover.
    fn next_target(&mut self) {
        let now = self.now();
        if self.current_pos() == UnitPos::Middle && self.rand() > 0.7 {
            self.set_fsm_pos(UnitPos::Down);
            let points = self.aim_points(15.0, 0.5, 4.0);
            if let Some(cover) = self.cover() {
                cover.aim_points = points;
            }
        }
        let pick = (5.0 * self.rand()).round().clamp(0.0, 4.0) as usize;
        if let Some(cover) = self.cover() {
            cover.target_time = now;
            cover.aim = pick;
            cover.mode = 2;
        }
    }
}

impl Driver for ManDriver<'_> {
    fn action(&mut self, action: &Action) {
        // `script:` actions need the VM, which the World does not own.
        if let Action::Native(native) = action {
            self.run_action(&native.function, &native.parameters);
        }
    }

    fn precondition(&mut self, _code: &str) {}

    fn condition(&mut self, condition: &Condition) -> f32 {
        match condition {
            Condition::Native(native) if !native.script => {
                self.condition_value(&native.function, &native.parameters)
            }
            // `script:` conditions need the VM: they never hold here.
            _ => -f32::MAX,
        }
    }

    fn random(&mut self) -> f32 {
        self.rand() as f32
    }
}
