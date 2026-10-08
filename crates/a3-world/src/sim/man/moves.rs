//! The Move state machine of a Man: the move he plays now, the blend from the one before, and
//! the play queue `playMove` and friends build (`docs/re/sim-man-movement.md`,
//! `docs/re/sim-man-anim-state.md`).
//!
//! The machine plays one move of the moves type at a time. A move is left along the move graph
//! (a `connectTo` or `interpolateTo` edge) and the blend between the two is driven by their
//! `interpolationSpeed`. Movement ([`super::ManState::motion`]) comes out of the moves: each
//! contributes its RTM step scaled by its blend weight.

use a3_moves::{MoveId, Moves, Stance};
use glam::{DQuat, DVec3, Vec3};

use super::ManInput;

/// The state machine of one Man: his current move, where in its animation he is, and the blend
/// from the previous move.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MoveState {
    /// The move he plays now. `None` until his first step, or while no moves type is loaded.
    current: Option<MoveId>,
    /// The move being blended out of; dropped once the blend into [`Self::current`] is done.
    previous: Option<MoveId>,
    /// How much of him the current move is: `1.0` with no blend running, and the previous move
    /// holds `1.0 - weight`.
    weight: f64,
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

    /// One step of the machine: pick the move his input asks for, blend into it, and advance the
    /// play queue. `moves` is `None` while no moves type is loaded, and then he has no move.
    pub fn advance(&mut self, moves: Option<&Moves>, input: &ManInput, dt: f64) {
        let Some(moves) = moves else {
            return;
        };
        // His first step: he starts in the idle move of his stance, with nothing to blend from.
        if self.current.is_none() {
            self.current = idle_move(moves, self.wanted_stance(moves, input));
            self.weight = 1.0;
        }
        let Some(current) = self.current else {
            return;
        };
        if let Some(target) = requested_move(moves, current, input) {
            self.start(moves, current, target);
        }
        self.blend(moves, dt);
    }

    /// The world velocity his animation gives him: the moves he plays, each with its phase rate
    /// times the RTM step of its cycle, weighted by the blend
    /// (`docs/re/sim-man-movement.md` §3). [`DVec3::ZERO`] while he has no move, or while his
    /// move does not move him (an idle).
    pub fn velocity(&self, moves: Option<&Moves>, orientation: DQuat) -> DVec3 {
        let (Some(moves), Some(current)) = (moves, self.current) else {
            return DVec3::ZERO;
        };
        let mut step = contribution(moves, current) * self.weight as f32;
        if let Some(previous) = self.previous {
            step += contribution(moves, previous) * (1.0 - self.weight) as f32;
        }
        step_in_world(step, orientation)
    }

    /// Starts the move `target` in place of `current`, blending out of it, when the graph links
    /// the two directly. A move further away needs the states between them, which #191 does not
    /// walk yet.
    fn start(&mut self, moves: &Moves, current: MoveId, target: MoveId) {
        if target == current || moves.edge(current, target).is_none() {
            return;
        }
        self.previous = Some(current);
        self.weight = 0.0;
        self.current = Some(target);
    }

    /// Ramps the blend at the current move's `interpolationSpeed`; once it holds all of him the
    /// move before is dropped. _Simplification_: the original eases the ramp
    /// (`docs/re/sim-man-anim-state.md` §5).
    fn blend(&mut self, moves: &Moves, dt: f64) {
        let Some(current) = self.current.filter(|_| self.previous.is_some()) else {
            return;
        };
        let speed = f64::from(moves.get(current).interpolation_speed);
        self.weight = (self.weight + speed * dt).min(1.0);
        if self.weight >= 1.0 {
            self.previous = None;
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
