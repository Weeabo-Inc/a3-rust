//! Projectiles (`Shot`: CfgAmmo `shot*` simulations). Issue #127 adds ballistics (air friction,
//! gravity, `coefGravity`), hit detection, explosions and fuses.
//!
//! Called once per step on every machine. For now a projectile moves in a straight line at its
//! velocity.

use crate::Entity;

use super::StepContext;

/// Class-specific state of projectiles (`ClassState::Projectile`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProjectileState {}

pub(crate) fn simulate(entity: &mut Entity, _ctx: &mut StepContext<'_>, dt: f64) {
    entity.position += entity.velocity * dt;
}
