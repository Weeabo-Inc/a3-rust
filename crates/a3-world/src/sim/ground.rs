//! Ground vehicles and ships (`TankOrCar`: car, carx, motorcycle, tank, tankx, ship, shipx, ...). Issue #125: engine, gearbox, wheels or tracks, buoyancy.
//!
//! Called once per simulation step of each Entity of this family, on every machine. Do the
//! authoritative work (forces, damage, decisions) only when `entity.is_local()`; a remote
//! Entity only advances from its last received state.

use crate::Entity;

use super::StepContext;

/// Class-specific state of this family (`ClassState`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GroundState {}

pub(crate) fn simulate(_entity: &mut Entity, _ctx: &mut StepContext<'_>, _dt: f64) {}
