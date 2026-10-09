//! `World::simulate`: the frame's simulation order, the per-Entity step accumulator and the
//! dispatch to the family modules (`man`, `ground`, `air`, `projectile`, `generic`).
//!
//! Mirrors the original's `World::SimulateAllVehicles` (`docs/re/world-object-model.md`,
//! "Simulation order"; ADR 0006):
//!
//! 1. Remote state received from the network is applied (not yet: network work is deferred).
//! 2. Projectiles: each simulates the whole frame in chunks of its own step, plus the remainder
//!    (the original's `EntityMTSimJob`).
//! 3. Vehicles, slow Entities and promoted Static objects: the frame is cut into sub-steps of
//!    [`SUB_STEP`] while more than [`SUB_STEP`] remains, then the remainder. In each sub-step
//!    every Entity adds the time to its accumulator and, once the accumulator reaches its
//!    simulation step, runs one step of exactly that length (`Entity_SimulateFixedStep`). An
//!    Entity whose step is 0 runs every sub-step with the accumulated time.
//!
//!    _Deviation, to settle in issue #117_: the original's catch-up loop consumes 0.025 s of a
//!    long frame per iteration but feeds 0.05 s to Entities of one of four interleaved sub-lists
//!    at a time; the remainder goes through a separate per-frame path. We keep the invariant
//!    that every simulated Entity covers exactly the frame time, with 0.025 s sub-steps.
//! 4. Attached positions: every `attachTo`-ed Entity moves to its parent.
//! 5. AI: every local group works its waypoints, updates its targets and fills the [`ManInput`]
//!    of its units (#129).
//! 6. Commands queued by steps (creations, deletions) are applied, then deletions take effect.
//!
//! A step runs for local and remote Entities alike; family modules do the authoritative parts
//! (physics forces, damage, AI decisions, firing) only when [`Entity::is_local`].

mod air;
mod generic;
mod ground;
mod man;
mod projectile;

use crate::{Create, DamageHit, EntityClass, EntityId, ListKind, SimulationClass, World};

pub use air::AirState;
pub use ground::GroundState;
pub use man::{GRAVITY, MAX_STEP_DOWN, MAX_STEP_UP, ManInput, ManState, Motion, MoveState};
pub use projectile::ProjectileState;

use crate::Entity;

/// Frame time per sub-step; frames longer than this are cut into sub-steps (the original's
/// catch-up threshold, 0.025 s).
pub const SUB_STEP: f64 = 0.025;

/// Time differences below a microsecond are rounding noise (steps come from `f32` config values).
const EPSILON: f64 = 1e-6;

/// Class-specific state of an Entity, one variant per simulation family.
// One ClassState is stored inline in every Entity; the Man variant is by far the largest and
// boxing it would put a pointer chase on every man's step for a few hundred kilobytes saved.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ClassState {
    /// Entities without family behaviour yet (buildings, things, triggers, ...).
    #[default]
    Generic,
    Man(ManState),
    Ground(GroundState),
    Air(AirState),
    Projectile(ProjectileState),
}

impl ClassState {
    pub(crate) fn for_class(class: SimulationClass) -> ClassState {
        match Family::of(class) {
            Family::Generic => ClassState::Generic,
            Family::Man => ClassState::Man(ManState::default()),
            Family::Ground => ClassState::Ground(GroundState::default()),
            Family::Air => ClassState::Air(AirState::default()),
            Family::Projectile => ClassState::Projectile(ProjectileState::default()),
        }
    }
}

/// Which module simulates a class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    Generic,
    Man,
    Ground,
    Air,
    Projectile,
}

impl Family {
    pub(crate) fn of(class: SimulationClass) -> Family {
        let c = class.engine_class();
        if c.is_kind_of(EntityClass::Shot) {
            Family::Projectile
        } else if c.is_kind_of(EntityClass::Person) {
            Family::Man
        } else if c.is_kind_of(EntityClass::TankOrCar) {
            Family::Ground
        } else if c.is_kind_of(EntityClass::PlaneOrHeli) || c.is_kind_of(EntityClass::Parachute) {
            Family::Air
        } else {
            Family::Generic
        }
    }
}

/// Something a step asks the World to do after the phase, because a step may only change its
/// own Entity.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Command {
    Create(Create),
    Delete(EntityId),
    /// A hit on another Entity (a projectile's impact, an explosion). Applied after the phase,
    /// when every Entity is back in its slot, so the damage path sees the whole World.
    Damage(EntityId, Box<DamageHit>),
}

/// What a family step sees besides its own Entity: the rest of the World (read-only; the
/// stepping Entity itself is absent from it during its step) and a queue for changes to other
/// Entities.
pub(crate) struct StepContext<'a> {
    world: &'a World,
    commands: &'a mut Vec<Command>,
}

#[allow(dead_code)] // Used by the family modules as they are filled in.
impl StepContext<'_> {
    /// The World without the stepping Entity.
    pub(crate) fn world(&self) -> &World {
        self.world
    }

    /// World time at the start of the frame, in seconds.
    pub(crate) fn time(&self) -> f64 {
        self.world.time()
    }

    /// Creates an Entity after this phase (a fired projectile, a dropped object).
    pub(crate) fn create(&mut self, request: Create) {
        self.commands.push(Command::Create(request));
    }

    /// Deletes an Entity after this phase.
    pub(crate) fn delete(&mut self, id: EntityId) {
        self.commands.push(Command::Delete(id));
    }

    /// Damages an Entity after this phase (`apply_damage_to`; the target may be any Entity,
    /// including the stepping one, once the phase is over).
    pub(crate) fn damage(&mut self, id: EntityId, hit: DamageHit) {
        self.commands.push(Command::Damage(id, Box::new(hit)));
    }
}

impl World {
    /// Advances the World by one frame of `dt` seconds (already clamped and scaled by the frame
    /// clock, ADR 0002). See the module docs for the order.
    pub fn simulate(&mut self, dt: f64) {
        for e in self.slots_mut().flatten() {
            e.visual.since_step += dt;
        }
        let mut commands = Vec::new();

        // 1. Remote state: applied here once the network layer exists (#131).

        // 2. Projectiles.
        for id in self.list(ListKind::Projectiles).to_vec() {
            self.simulate_projectile(id, dt, &mut commands);
        }
        self.apply(&mut commands);

        // 3. Fixed sub-steps over the other lists.
        let mut remaining = dt;
        while remaining > SUB_STEP {
            self.sub_step(SUB_STEP, &mut commands);
            remaining -= SUB_STEP;
        }
        if remaining > EPSILON {
            self.sub_step(remaining, &mut commands);
        }
        self.apply(&mut commands);

        // 4. Attached positions.
        self.update_attached_positions();

        // 5. AI: a local group works its waypoints and fills its units' inputs (#129). A group
        // owned elsewhere is driven there; the network layer brings its state in.
        self.perform_ai(dt);

        // 6.
        self.flush_deletions();
        self.advance_time(dt);
    }

    fn simulate_projectile(&mut self, id: EntityId, dt: f64, commands: &mut Vec<Command>) {
        let Some(step) = self
            .entity(id)
            .filter(|e| e.is_simulated())
            .map(|e| e.simulation_step)
        else {
            return;
        };
        let mut remaining = dt;
        if step > 0.0 {
            while step <= remaining + EPSILON {
                self.run_step(id, step, commands);
                remaining -= step;
            }
        }
        if remaining > EPSILON {
            self.run_step(id, remaining, commands);
        }
    }

    fn sub_step(&mut self, dt: f64, commands: &mut Vec<Command>) {
        for list in [ListKind::Vehicles, ListKind::Slow, ListKind::Static] {
            for id in self.list(list).to_vec() {
                let Some(e) = self.entity_mut(id).filter(|e| e.is_simulated()) else {
                    continue;
                };
                e.accumulated += dt;
                let step = e.simulation_step;
                let run = if step <= 0.0 {
                    e.accumulated
                } else if e.accumulated + EPSILON >= step {
                    step
                } else {
                    continue;
                };
                e.accumulated = (e.accumulated - run).max(0.0);
                self.run_step(id, run, commands);
            }
        }
    }

    /// One simulation step of `dt` for one Entity: record the visual state, then the family.
    fn run_step(&mut self, id: EntityId, dt: f64, commands: &mut Vec<Command>) {
        let Some(mut entity) = self.take_entity(id) else {
            return;
        };
        entity.visual.previous_position = entity.position;
        entity.visual.previous_orientation = entity.orientation;
        entity.visual.last_step = dt;
        entity.visual.since_step = 0.0;
        {
            let mut ctx = StepContext {
                world: self,
                commands,
            };
            step_family(&mut entity, &mut ctx, dt);
        }
        entity.steps += 1;
        entity.simulated_time += dt;
        self.put_entity(entity);
    }

    fn apply(&mut self, commands: &mut Vec<Command>) {
        for command in commands.drain(..) {
            match command {
                Command::Create(request) => {
                    // A family step only asks for creatable types; a refusal leaves the World as is.
                    let _ = self.create(request);
                }
                Command::Delete(id) => {
                    self.delete(id);
                }
                Command::Damage(id, hit) => {
                    self.apply_damage_to(id, *hit);
                }
            }
        }
    }
}

fn step_family(entity: &mut Entity, ctx: &mut StepContext<'_>, dt: f64) {
    match Family::of(entity.class()) {
        Family::Generic => generic::simulate(entity, ctx, dt),
        Family::Man => man::simulate(entity, ctx, dt),
        Family::Ground => ground::simulate(entity, ctx, dt),
        Family::Air => air::simulate(entity, ctx, dt),
        Family::Projectile => projectile::simulate(entity, ctx, dt),
    }
}
