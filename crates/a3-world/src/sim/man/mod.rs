//! Men and animals (`Person`: soldier, uavpilot, animal, logic). Issue #124: moves state machine (CfgMovesBasic) on RTM animation, walking on terrain and roadways, getting in and out of Transports.
//!
//! Called once per simulation step of each Entity of this family, on every machine. Do the
//! authoritative work (forces, damage, decisions) only when `entity.is_local()`; a remote
//! Entity only advances from its last received state.

use crate::{ClassState, Entity};

use super::StepContext;

mod ground;

pub use ground::{GRAVITY, GroundContact, GroundQuery, MAX_STEP_DOWN, MAX_STEP_UP, Motion};

/// Class-specific state of this family (`ClassState`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ManState {
    /// Where his feet are relative to the ground: standing on it, or falling to it.
    pub motion: Motion,
}

pub(crate) fn simulate(entity: &mut Entity, ctx: &mut StepContext<'_>, dt: f64) {
    if !entity.is_local() {
        return;
    }
    let feet = entity.position;
    let velocity = entity.velocity;
    // Without a terrain there is no surface to stand on or land on; leave him where he is
    // (in the original a World always has one).
    let Some(terrain) = ctx.world().terrain().cloned() else {
        return;
    };
    let next = match entity.class_state_mut() {
        ClassState::Man(man) => man.motion.step(feet, velocity, terrain.as_ref(), dt),
        _ => return,
    };
    entity.position = next;
}
