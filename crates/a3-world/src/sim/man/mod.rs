//! Men and animals (`Person`: soldier, uavpilot, animal, logic). Issue #124: moves state machine (CfgMovesBasic) on RTM animation, walking on terrain and roadways, getting in and out of Transports.
//!
//! Called once per simulation step of each Entity of this family, on every machine. Do the
//! authoritative work (forces, damage, decisions) only when `entity.is_local()`; a remote
//! Entity only advances from its last received state.

use glam::DQuat;

use crate::{ClassState, Entity};

use super::StepContext;

mod ground;
mod input;
mod moves;
mod turn;

pub use ground::{GRAVITY, MAX_STEP_DOWN, MAX_STEP_UP, Motion};
pub use input::ManInput;
pub use moves::MoveState;
pub use turn::Turning;

/// Class-specific state of this family (`ClassState`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ManState {
    /// Where his feet are relative to the ground: standing on it, or falling to it.
    pub motion: Motion,
    /// What he is asked to do, filled by the controller that drives him.
    pub input: ManInput,
    /// Which move he plays and how it blends from the previous one.
    pub moves: MoveState,
    /// Which way he is turning, following [`Self::input`].
    pub turn: Turning,
}

pub(crate) fn simulate(entity: &mut Entity, ctx: &mut StepContext<'_>, dt: f64) {
    if !entity.is_local() {
        return;
    }
    let feet = entity.position;
    let orientation = entity.orientation();
    // Without a collision world there is no surface to stand on or land on; leave him where he
    // is (in the original a World always has one).
    let Some(ground) = ctx.world().collision_world() else {
        return;
    };
    let moves = ctx.world().moves().cloned();
    let (next, velocity, orientation) = match entity.class_state_mut() {
        ClassState::Man(man) => {
            man.moves.advance(moves.as_deref(), &man.input, dt);
            // He turns as fast as the move he plays lets him (`docs/re/sim-man-locomotion.md`
            // §2), and his front is what the animation moves.
            let yaw = man
                .turn
                .step(man.input.turn, man.moves.turn_speed(moves.as_deref()), dt);
            let orientation = DQuat::from_rotation_y(yaw) * orientation;
            // The animation is the source of truth for his movement: the moves he plays carry
            // him, and the ground decides his height (`docs/re/sim-man-movement.md` §3).
            let velocity = man.moves.velocity(orientation);
            (
                man.motion.step(feet, velocity, ground, dt),
                velocity,
                orientation,
            )
        }
        _ => return,
    };
    entity.position = next;
    entity.velocity = velocity;
    entity.orientation = orientation;
}
