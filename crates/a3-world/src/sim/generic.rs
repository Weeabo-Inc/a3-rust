//! Entities without family behaviour yet: buildings, things, flags, targets, triggers, cameras,
//! lamps, proxies. They step (their accumulator and visual state advance) but do nothing.

use crate::Entity;

use super::StepContext;

pub(crate) fn simulate(_entity: &mut Entity, _ctx: &mut StepContext<'_>, _dt: f64) {}
