//! The Move state machine of a Man: the move he plays now, the blend from the one before, and
//! how he walks the move graph between them (`docs/re/sim-man-movement.md`,
//! `docs/re/sim-man-anim-state.md`).
//!
//! The machine plays one move of the moves type at a time. A move his input asks for — or the
//! one a script asked for with `playMove`, which wins until it is done — is routed through the
//! move graph ([`Moves::find_path`]) and the route is played hop by hop: a hop over an
//! `interpolateTo` edge blends into the next move at once (once the current move has played its
//! `minPlayTime`), a hop over a `connectTo` edge waits for the end of the current move's cycle.
//! Movement ([`super::ManState::motion`]) comes out of the moves: each contributes its RTM step
//! scaled by its blend weight.

use std::collections::VecDeque;

use a3_moves::{EdgeKind, Move, MoveId, Moves, Stance};
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
    /// How far through the cycle of [`Self::previous`]'s own move that move has played, on its
    /// own phase rate while it fades: a pose samples each move of a blend at its own phase
    /// (`a3_pose::MoveBlend`). Meaningless while [`Self::previous`] is `None`.
    previous_phase: f64,
    /// How much of him the current move is: `1.0` with no blend running, and the previous move
    /// holds `1.0 - weight`.
    weight: f64,
    /// The blend ramp behind [`Self::weight`]; `1.0` when no blend is running.
    accumulator: f64,
    /// The moves still to play to reach the one his input asks for, from the move graph.
    plan: VecDeque<MoveId>,
    /// The move he was explicitly asked for (`playMove`, `playMoveNow`, a script): it wins over
    /// his input until it is done. The engine's request slot (`Man+0x440`), empty as `None`.
    request: Option<MoveId>,
    /// The moves a script queued behind the request (`playMove`); the next one starts once the
    /// request is done. The engine's queue at `Man+0x1DE0`.
    queue: VecDeque<MoveId>,
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

    /// How far through the current move's cycle he is, `0.0..=1.0`.
    pub fn phase(&self) -> f64 {
        self.phase
    }

    /// The move he is blending out of, `None` once the blend into [`Self::current`] is done.
    ///
    /// With [`Self::phase`], [`Self::weight`] and [`Self::previous_phase`] this is everything a
    /// pose needs: the move being left and the one being entered, each at the phase of its own
    /// cycle, and how far between them he is (`a3_pose::MoveBlend`).
    pub fn previous(&self) -> Option<MoveId> {
        self.previous
    }

    /// How far through the cycle of the move being left *that* move has played, `0.0..=1.0` —
    /// each move of a blend advances in its own cycle. The current move's phase while there is
    /// none being left, so a pose can name the move he is in twice without reading the phase
    /// twice.
    pub fn previous_phase(&self) -> f64 {
        match self.previous {
            Some(_) => self.previous_phase,
            None => self.phase,
        }
    }

    /// How much of him the current move is: `1.0` when no blend is running, lower while one
    /// fades the move before it out.
    pub fn weight(&self) -> f64 {
        self.weight
    }

    /// The moves a script queued with [`Self::play`], front first: the order they will play in.
    pub fn queue(&self) -> impl Iterator<Item = MoveId> + '_ {
        self.queue.iter().copied()
    }

    /// `playMove`: queues `move` behind everything already queued. It becomes the request — and
    /// is routed to through the move graph like any other — once the one before it has played out
    /// (`docs/re/sim-man-anim-state.md` §3).
    pub fn play(&mut self, id: MoveId) {
        self.queue.push_back(id);
    }

    /// `playMoveNow`: drops the queue and arms `move` at once. The route through the graph is
    /// still walked; only the queue is skipped (`docs/re/sim-man-anim-state.md` §6.1).
    pub fn play_now(&mut self, id: MoveId) {
        self.queue.clear();
        self.plan.clear();
        self.request = Some(id);
    }

    /// `switchMove`: resets the record to `move` on the spot — no route through the graph, no
    /// blend out of the move before, nothing queued or requested — and writes the cycle `phase`
    /// and the blend `weight` the script asked for (`time` and `blendFactor` of the array form;
    /// `docs/re/sim-man-anim-state.md` §6.3).
    pub fn switch_to(&mut self, id: MoveId, phase: f64, weight: f64) {
        self.queue.clear();
        self.request = None;
        self.plan.clear();
        self.current = Some(id);
        self.previous = None;
        self.phase = phase.clamp(0.0, 1.0);
        self.previous_phase = self.phase;
        self.weight = weight.clamp(0.0, 1.0);
        self.accumulator = self.weight;
        self.cycle_ended = false;
    }

    /// The move an unresolved `switchMove` name falls back to: the default state of the action
    /// map of the move he plays, i.e. its `Stop` move — the idle of the stance he is in
    /// (`docs/re/sim-man-anim-state.md` §6.3).
    pub(crate) fn default_move(&self, moves: &Moves) -> Option<MoveId> {
        moves.action_move(self.current?, "Stop")
    }

    /// One step of the machine: plan the move his input asks for, play the route, advance the
    /// cycle and the blend, and update the velocity his moves give him. `moves` is `None` while
    /// no moves type is loaded, and then he has no move.
    pub fn advance(&mut self, moves: Option<&Moves>, input: &ManInput, dt: f64) {
        let Some(moves) = moves else {
            return;
        };
        // His first step: he starts in the idle move of the moves type's default stance, with
        // nothing to blend from. The stance he asks for is a request like any other, so it walks
        // the graph from there.
        if self.current.is_none() {
            self.current = idle_move(moves, Stance::Stand);
            self.weight = 1.0;
            self.accumulator = 1.0;
        }
        let Some(current) = self.current else {
            return;
        };
        self.consume_queue();
        self.plan_route(moves, current, input);
        self.hop(moves);
        self.advance_phase(moves, dt);
        self.retire_request(moves);
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

    /// How fast a full turn swings his front in the move he plays: the `turnSpeed` of its action
    /// map, the config's per-move turn limit (`docs/re/sim-man-locomotion.md` §2). `0.0` while he
    /// has no move.
    pub fn turn_speed(&self, moves: Option<&Moves>) -> f32 {
        let (Some(moves), Some(current)) = (moves, self.current) else {
            return 0.0;
        };
        moves
            .get(current)
            .actions
            .map(|map| moves.action_map(map).turn_speed)
            .unwrap_or(0.0)
    }

    /// Takes the next queued move into the request slot, once the request before it is done — at
    /// most one per step (`docs/re/sim-man-anim-state.md` §3).
    fn consume_queue(&mut self) {
        if self.request.is_some() {
            return;
        }
        if let Some(next) = self.queue.pop_front() {
            self.request = Some(next);
            self.plan.clear();
        }
    }

    /// Drops the request once the move he was asked for has played out: the request is satisfied
    /// at the end of the requested move's cycle, and from the next step on his input has him —
    /// or the next queued move (`docs/re/sim-man-anim-state.md` §5.4).
    fn retire_request(&mut self, moves: &Moves) {
        let (Some(request), Some(current)) = (self.request, self.current) else {
            return;
        };
        if self.cycle_ended && equivalent(moves, current) == equivalent(moves, request) {
            self.request = None;
        }
    }

    /// Routes to the move he was asked for — the request while one stands, his input otherwise —
    /// when he is not already walking a route. A move the graph cannot reach is planned again on
    /// the next step, as the engine's planner keeps the request standing.
    fn plan_route(&mut self, moves: &Moves, current: MoveId, input: &ManInput) {
        if !self.plan.is_empty() {
            return;
        }
        let Some(target) = self
            .request
            .or_else(|| requested_move(moves, current, input))
        else {
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
        self.previous_phase = 0.0;
        self.weight = 1.0;
        self.accumulator = 1.0;
        self.cycle_ended = false;
    }

    /// Blends into `next` at once: the move he was in plays on while `next` takes over, starting
    /// at the phase its `interpolationRestart` asks for (`docs/re/sim-man-anim-state.md` §5).
    fn interpolate(&mut self, moves: &Moves, next: MoveId) {
        let phase = self.phase;
        self.previous = self.current;
        self.previous_phase = phase;
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
        let (phase, ended) = advance_cycle(self.phase, moves.get(current), dt);
        self.phase = phase;
        self.cycle_ended = ended;
        // The move being left plays on at its own phase rate while it fades, so a pose samples
        // both moves of the blend at the phase each has reached of its own cycle.
        if let Some(previous) = self.previous {
            self.previous_phase = advance_cycle(self.previous_phase, moves.get(previous), dt).0;
        }
    }

    /// Ramps the blend at the current move's `interpolationSpeed`; once it holds all of him the
    /// move before is dropped. A blend the script started with a `blendFactor` below 1 ramps on
    /// too, from nothing (we keep no pose of the move before a `switchMove`). _Simplification_:
    /// the original eases the ramp (`docs/re/sim-man-anim-state.md` §3).
    fn blend(&mut self, moves: &Moves, dt: f64) {
        let Some(current) = self
            .current
            .filter(|_| self.previous.is_some() || self.weight < 1.0)
        else {
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
}

/// The id a request is matched against: a move that is `equivalentTo` another is that other move
/// for the request bookkeeping, as the engine compares the normalised ids
/// (`docs/re/sim-man-anim-state.md` §5.4).
fn equivalent(moves: &Moves, id: MoveId) -> MoveId {
    moves.get(id).equivalent_to.unwrap_or(id)
}

/// What a move moves him by per second (model space): its RTM step per cycle times its phase
/// rate.
fn contribution(moves: &Moves, id: MoveId) -> Vec3 {
    let mv = moves.get(id);
    mv.step * mv.speed
}

/// One step of `phase` through `mv`'s cycle: a looped move wraps at the end, a one-shot move
/// stops there. The flag says the cycle ended on this step.
fn advance_cycle(phase: f64, mv: &Move, dt: f64) -> (f64, bool) {
    let phase = phase + f64::from(mv.speed) * dt;
    if phase >= 1.0 {
        (if mv.looped { phase - 1.0 } else { 1.0 }, true)
    } else {
        (phase, false)
    }
}

/// An RTM step in world space: a step is model space, whose forward is −Z (`docs/re/rtm.md`),
/// while the World's front is `orientation * Z`.
fn step_in_world(step: Vec3, orientation: DQuat) -> DVec3 {
    orientation * DVec3::new(-f64::from(step.x), f64::from(step.y), -f64::from(step.z))
}

/// The move his input asks for while he is in `current`: the idle move of the stance he asks
/// for when that is not the stance he is in, otherwise the action of the current move's map
/// whose key matches his axes, sprinting picking `RunF` over `WalkF`.
fn requested_move(moves: &Moves, current: MoveId, input: &ManInput) -> Option<MoveId> {
    // A stance change is a move like any other: the machine walks the graph from where he is to
    // the idle of the stance he asks for, playing the transition moves between them
    // (`Down`/`StandDown`, and their like, in `docs/re/sim-man-movement.md` §1).
    if input.stance != Stance::Undefined && input.stance != stance_of(moves, current) {
        return idle_move(moves, input.stance);
    }
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

/// The stance the move he plays belongs to, [`Stance::Undefined`] when it belongs to none (a
/// transition move).
fn stance_of(moves: &Moves, id: MoveId) -> Stance {
    moves
        .get(id)
        .actions
        .map(|map| moves.action_map(map).stance)
        .unwrap_or(Stance::Undefined)
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
