//! The Move state machine of a Man: the move he plays now, the blend from the one before, and
//! how he walks the move graph between them (`docs/re/sim-man-movement.md`,
//! `docs/re/sim-man-anim-state.md`).
//!
//! The machine plays one move of the moves type at a time. A move his input asks for is routed
//! through the move graph ([`Moves::find_path`]) and the route is played hop by hop: a hop over
//! an `interpolateTo` edge blends into the next move at once (once the current move has played
//! its `minPlayTime`), a hop over a `connectTo` edge waits for the end of the current move's
//! cycle. Movement ([`super::ManState::motion`]) comes out of the moves: each contributes its
//! RTM step scaled by its blend weight.

use std::collections::VecDeque;

use a3_moves::{EdgeKind, MoveId, Moves, Stance};
use glam::{DQuat, DVec3, Vec3};

use super::ManInput;

/// How fast his velocity may change, per axis, in m/s² (`docs/re/sim-man-movement.md` §3).
const MAX_ACCELERATION: f32 = 20.0;

/// The state machine of one Man: the move he plays, where in its cycle he is, the route he is
/// walking through the graph, and the blend from the move before.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MoveState {
    /// The move he plays now. `None` until his first step, or while no moves type is loaded.
    current: Option<MoveId>,
    /// How far through the current move's cycle he is, `0.0..=1.0` (of the cycle it plays, so a
    /// move played backwards counts the same way).
    phase: f64,
    /// Whether the current move reached the end of its cycle on his last step; a `connectTo`
    /// hop waits for it (`docs/re/sim-man-anim-state.md` §5).
    cycle_ended: bool,
    /// The move being blended out of; dropped once the blend into [`Self::current`] is done.
    previous: Option<MoveId>,
    /// How much of him the current move is: `1.0` with no blend running, and the previous move
    /// holds `1.0 - weight`.
    weight: f64,
    /// The blend ramp behind [`Self::weight`]; `1.0` when no blend is running.
    accumulator: f64,
    /// The moves still to play to reach the one his input asks for, from the move graph.
    plan: VecDeque<MoveId>,
    /// His velocity in model space (forward is −Z), built from the moves he plays
    /// (`docs/re/sim-man-movement.md` §3).
    velocity: Vec3,
}

impl MoveState {
    /// The move he plays now.
    pub fn current(&self) -> Option<MoveId> {
        self.current
    }

    /// The name of the move he plays now, in its original case (`Stand`, not `stand`), if any.
    pub fn move_name<'a>(&self, moves: &'a Moves) -> Option<&'a str> {
        Some(moves.get(self.current?).name.as_str())
    }

    /// One step of the machine: plan the move his input asks for, play the route, advance the
    /// cycle and the blend, and update the velocity his moves give him. `moves` is `None` while
    /// no moves type is loaded, and then he has no move.
    pub fn advance(&mut self, moves: Option<&Moves>, input: &ManInput, dt: f64) {
        let Some(moves) = moves else {
            return;
        };
        // His first step: he starts in the idle move of his stance, with nothing to blend from.
        if self.current.is_none() {
            self.current = idle_move(moves, self.wanted_stance(moves, input));
            self.weight = 1.0;
            self.accumulator = 1.0;
        }
        let Some(current) = self.current else {
            return;
        };
        self.plan_route(moves, current, input);
        self.hop(moves);
        self.advance_phase(moves, dt);
        self.blend(moves, dt);
        self.advance_velocity(moves, dt);
    }

    /// The world velocity his animation gives him, from the last [`Self::advance`]: the moves he
    /// plays, each with its phase rate times the RTM step of its cycle, weighted by the blend
    /// (`docs/re/sim-man-movement.md` §3). [`DVec3::ZERO`] while he has no move, or while his
    /// move does not move him (an idle).
    pub fn velocity(&self, orientation: DQuat) -> DVec3 {
        step_in_world(self.velocity, orientation)
    }

    /// Routes to the move his input asks for, when he is not already walking a route. A move the
    /// graph cannot reach is planned again on the next step, as the engine's planner keeps the
    /// request standing.
    fn plan_route(&mut self, moves: &Moves, current: MoveId, input: &ManInput) {
        if !self.plan.is_empty() {
            return;
        }
        let Some(target) = requested_move(moves, current, input) else {
            return;
        };
        if target != current {
            self.plan = moves.find_path(current, target).unwrap_or_default().into();
        }
    }

    /// Plays the next move of the route, when it may start now: the blend into the current move
    /// has finished, and the edge allows it (`docs/re/sim-man-anim-state.md` §5).
    fn hop(&mut self, moves: &Moves) {
        let (Some(current), Some(&next)) = (self.current, self.plan.front()) else {
            return;
        };
        if self.weight < 1.0 {
            return;
        }
        let Some(edge) = moves.edge(current, next) else {
            // A route whose edge the graph no longer has: drop the hop and play the next.
            self.plan.pop_front();
            return;
        };
        match edge.kind {
            // The next move starts where this one ends: wait for the end of its cycle.
            EdgeKind::Connect => {
                if !self.cycle_ended {
                    return;
                }
            }
            // It blends in at once, but not before the current move has played its
            // `minPlayTime` — unless the current move loops (it never ends on its own) or the
            // edge ignores the time. (The engine also lets a hop through when the current move
            // object's variant id equals the next move's; not modelled, we load no variants.)
            EdgeKind::Interpolate => {
                let mv = moves.get(current);
                if !mv.looped
                    && !edge.ignore_min_play_time
                    && self.phase < f64::from(mv.min_play_time)
                {
                    return;
                }
            }
        }
        self.plan.pop_front();
        match edge.kind {
            EdgeKind::Connect => self.snap(next),
            EdgeKind::Interpolate => self.interpolate(moves, next),
        }
    }

    /// Puts him in `next` outright, its cycle from the start: a `connectTo` pair is authored so
    /// the pose at the end of the first move is the pose at the start of the second, so there is
    /// nothing to blend (`docs/re/sim-man-anim-state.md` §5).
    fn snap(&mut self, next: MoveId) {
        self.current = Some(next);
        self.previous = None;
        self.phase = 0.0;
        self.weight = 1.0;
        self.accumulator = 1.0;
        self.cycle_ended = false;
    }

    /// Blends into `next` at once: the move he was in plays on while `next` takes over, starting
    /// at the phase its `interpolationRestart` asks for (`docs/re/sim-man-anim-state.md` §5).
    fn interpolate(&mut self, moves: &Moves, next: MoveId) {
        let phase = self.phase;
        self.previous = self.current;
        self.current = Some(next);
        self.phase = match moves.get(next).interpolation_restart {
            1 => 0.0,
            2 => 1.0 - phase,
            // 0: the blend carries over, both moves at the phase the old one was at.
            _ => phase,
        };
        self.weight = 0.0;
        self.accumulator = 0.0;
        self.cycle_ended = false;
    }

    /// Advances the current move's cycle at its phase rate (`docs/re/sim-man-movement.md` §3). A
    /// looped move wraps at the end of its cycle and a one-shot move stops there; either way he
    /// may leave the move from his next step on ([`Self::cycle_ended`]).
    fn advance_phase(&mut self, moves: &Moves, dt: f64) {
        self.cycle_ended = false;
        let Some(current) = self.current else {
            return;
        };
        let mv = moves.get(current);
        self.phase += f64::from(mv.speed) * dt;
        if self.phase >= 1.0 {
            self.cycle_ended = true;
            self.phase = if mv.looped { self.phase - 1.0 } else { 1.0 };
        }
    }

    /// Ramps the blend at the current move's `interpolationSpeed`; once it holds all of him the
    /// move before is dropped. _Simplification_: the original eases the ramp
    /// (`docs/re/sim-man-anim-state.md` §3).
    fn blend(&mut self, moves: &Moves, dt: f64) {
        let Some(current) = self.current.filter(|_| self.previous.is_some()) else {
            self.weight = 1.0;
            self.accumulator = 1.0;
            return;
        };
        let speed = f64::from(moves.get(current).interpolation_speed);
        self.accumulator = (self.accumulator + speed * dt).min(1.0);
        self.weight = self.accumulator;
        if self.accumulator >= 1.0 {
            self.previous = None;
            self.weight = 1.0;
        }
    }

    /// Moves his velocity towards the one his moves ask for, by at most [`MAX_ACCELERATION`] per
    /// second (`docs/re/sim-man-movement.md` §3).
    fn advance_velocity(&mut self, moves: &Moves, dt: f64) {
        let target = self.target_velocity(moves);
        let limit = MAX_ACCELERATION * dt as f32;
        let delta = target - self.velocity;
        self.velocity += delta.clamp(Vec3::splat(-limit), Vec3::splat(limit));
    }

    /// The velocity the moves he plays ask for (model space): each contributes its RTM step
    /// times its phase rate, weighted by the blend (`docs/re/sim-man-movement.md` §3).
    fn target_velocity(&self, moves: &Moves) -> Vec3 {
        let Some(current) = self.current else {
            return Vec3::ZERO;
        };
        let weight = self.weight as f32;
        let mut velocity = contribution(moves, current) * weight;
        let mut total = weight;
        if let Some(previous) = self.previous {
            velocity += contribution(moves, previous) * (1.0 - weight);
            total += 1.0 - weight;
        }
        if total > 0.0 {
            velocity / total
        } else {
            Vec3::ZERO
        }
    }

    /// The stance he asks to be in: the input's stance, or the stance of the move he plays (he
    /// keeps it), or standing when he has no move yet.
    fn wanted_stance(&self, moves: &Moves, input: &ManInput) -> Stance {
        if input.stance != Stance::Undefined {
            return input.stance;
        }
        self.current
            .and_then(|current| moves.get(current).actions)
            .map(|map| moves.action_map(map).stance)
            .filter(|stance| *stance != Stance::Undefined)
            .unwrap_or(Stance::Stand)
    }
}

/// What a move moves him by per second (model space): its RTM step per cycle times its phase
/// rate.
fn contribution(moves: &Moves, id: MoveId) -> Vec3 {
    let mv = moves.get(id);
    mv.step * mv.speed
}

/// An RTM step in world space: a step is model space, whose forward is −Z (`docs/re/rtm.md`),
/// while the World's front is `orientation * Z`.
fn step_in_world(step: Vec3, orientation: DQuat) -> DVec3 {
    orientation * DVec3::new(-f64::from(step.x), f64::from(step.y), -f64::from(step.z))
}

/// The move his input asks for while he is in `current`: the action of the current move's map
/// whose key matches his axes, sprinting picking `RunF` over `WalkF`.
fn requested_move(moves: &Moves, current: MoveId, input: &ManInput) -> Option<MoveId> {
    let action = if input.forward > 0.0 {
        if input.sprint { "RunF" } else { "WalkF" }
    } else if input.forward < 0.0 {
        "WalkB"
    } else if input.strafe > 0.0 {
        "WalkR"
    } else if input.strafe < 0.0 {
        "WalkL"
    } else {
        "Stop"
    };
    moves.action_move(current, action)
}

/// The idle move of a stance: the `Stop` action of the stance's primary action map. Without a map
/// of that stance the first primary map answers (`CfgMovesBasic` `primaryActionMaps`).
fn idle_move(moves: &Moves, stance: Stance) -> Option<MoveId> {
    let maps = moves.primary_action_maps();
    let map = maps
        .iter()
        .map(|id| moves.action_map(*id))
        .find(|map| map.stance == stance)
        .or_else(|| maps.first().map(|id| moves.action_map(*id)))?;
    map.get_move("Stop")
}
