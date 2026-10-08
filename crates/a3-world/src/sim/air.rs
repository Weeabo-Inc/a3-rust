//! Aircraft and parachutes (`PlaneOrHeli`, `Parachute`). Issue #126: helicopter (basic and RTD), airplane and parachute flight models.
//!
//! Called once per simulation step of each Entity of this family, on every machine. Do the
//! authoritative work (forces, damage, decisions) only when `entity.is_local()`; a remote
//! Entity only advances from its last received state.

use crate::Entity;

use super::StepContext;

/// Class-specific state of this family (`ClassState`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AirState {}

pub(crate) fn simulate(_entity: &mut Entity, _ctx: &mut StepContext<'_>, _dt: f64) {}
