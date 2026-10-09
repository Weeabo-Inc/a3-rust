//! Projectiles (`Shot`: CfgAmmo `shot*` simulations): the flight, the timers and the impact of a
//! shot, per `docs/re/sim-ballistics.md` §3–§5.
//!
//! One step of `dt` (the ammo's `simulationStep`, `ShotShell::Simulate`):
//!
//! 1. the timers count down: `timeToLive` deletes the shot, the explosion timer (`explosionTime`)
//!    explodes it where it is;
//! 2. the move (`0x140e66ea0`): a segment test from the position to `position + velocity·dt`
//!    against the terrain and the Fire Geometry of every Object except the shot, its shooter and
//!    the shooter's vehicle. Without a hit the shot moves to the segment's end and its velocity
//!    takes one explicit Euler step (`0x140e6c3d0`). With a hit it moves to the hit point (the
//!    velocity steps for the time that took) and tries a ricochet, then a penetration, then
//!    stops; a ricochet or a penetration continues the move with what is left of `dt`.
//!
//! A shot without ammo parameters (an Entity created from a bare type) flies in a straight line.
//! Hits and explosions are queued as commands; the World applies the damage and records the
//! events after the projectile phase ([`crate::World::apply_hit`], [`crate::World::explode`]).

use std::sync::Arc;

use a3_physics::{CollisionWorld, Layer, LayerMask, ObjectKey, RayHit, RayQuery};
use glam::DVec3;

use crate::weapons::AmmoType;
use crate::{Entity, EntityId, ObjectRef, StaticKey};

use super::{ClassState, Command, StepContext};

/// The engine's gravity, m/s² (`9.8066` in `0x140e6c3d0`).
const GRAVITY: f64 = 9.8066;

/// A continued move (after a ricochet or a penetration) starts its object test this far along
/// the flight, so it does not meet the surface it just left.
const CONTINUE_OFFSET: f64 = 0.1;

/// A ricochet needs more than 5 m/s (`|v|² > 25`).
const RICOCHET_MIN_SPEED_SQ: f64 = 25.0;

/// Ricochets and penetrations continue the move recursively; the original has no bound, a shot
/// stuck between two surfaces must not hang the step.
const MAX_CONTINUATIONS: u32 = 32;

/// Class-specific state of projectiles (`ClassState::Projectile`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectileState {
    /// The `CfgAmmo` parameters; `None` for a shot made from a bare type, which flies straight.
    pub ammo: Option<Arc<AmmoType>>,
    /// Who fired it (`getShotParents`); ignored by its hit test with the vehicle they are in.
    pub shooter: Option<EntityId>,
    /// The `CfgWeapons` class it was fired from ("" when not fired from a weapon).
    pub weapon: String,
    /// The muzzle it was fired from.
    pub muzzle: String,
    /// The fire mode it was fired in.
    pub mode: String,
    /// The `CfgMagazines` class it came from.
    pub magazine: String,
    /// Seconds left before the shot is deleted (`+0x5e8`); infinite when `timeToLive ≤ 0`.
    pub time_to_live: f64,
    /// Seconds left before the shot explodes (`+0x648`); infinite when `explosionTime ≤ 0`.
    pub explosion_timer: f64,
    /// Metres left to fly before the shot is armed (`fuseDistance`, `+0x64c`).
    pub fuse_distance: f64,
    /// Whether it has exploded (`+0x5ec`): a shot explodes once.
    pub exploded: bool,
    /// Whether it draws a tracer (the magazine's `tracersEvery` / `lastRoundsTracer`).
    pub tracer: bool,
}

impl Default for ProjectileState {
    fn default() -> Self {
        Self {
            ammo: None,
            shooter: None,
            weapon: String::new(),
            muzzle: String::new(),
            mode: String::new(),
            magazine: String::new(),
            time_to_live: f64::INFINITY,
            explosion_timer: f64::INFINITY,
            fuse_distance: 0.0,
            exploded: false,
            tracer: false,
        }
    }
}

impl ProjectileState {
    /// The state of a new shot of `ammo`: its timers start from the config values.
    pub fn new(ammo: Arc<AmmoType>) -> Self {
        Self {
            time_to_live: if ammo.time_to_live > 0.0 {
                ammo.time_to_live
            } else {
                f64::INFINITY
            },
            explosion_timer: if ammo.explosion_time > 0.0 {
                ammo.explosion_time
            } else {
                f64::INFINITY
            },
            fuse_distance: ammo.fuse_distance.max(0.0),
            ammo: Some(ammo),
            ..Self::default()
        }
    }

    /// Whether the shot is armed: no fuse is running (`+0x648`), no arming distance is left
    /// (`+0x64c`). Only an armed shot explodes or deals its hit when it stops.
    fn armed(&self) -> bool {
        !(self.explosion_timer > 0.0 && self.explosion_timer < f64::MAX)
            && self.fuse_distance <= 0.0
    }
}

/// A shot met an Object: what the World applies as damage and reports as
/// [`crate::WorldEvent::Hit`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HitRecord {
    pub shot: EntityId,
    pub shooter: Option<EntityId>,
    pub target: ObjectRef,
    pub ammo: Arc<AmmoType>,
    pub position: DVec3,
    pub normal: DVec3,
    pub velocity_in: DVec3,
    pub velocity_out: DVec3,
    /// `hit · e` (§5).
    pub value: f64,
    /// The Fire Geometry component that was hit (`componentNN`), when known.
    pub component: Option<String>,
    /// The surface material that was hit (`.bisurf` path or `#Class`), when known.
    pub surface: Option<String>,
    /// The bounding radius of the hit layer (§5's `R`), 0 when unknown.
    pub radius: f64,
}

/// A shot exploded: the World applies the indirect damage (§6).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExplosionRecord {
    pub shot: EntityId,
    pub shooter: Option<EntityId>,
    pub ammo: Arc<AmmoType>,
    pub position: DVec3,
}

pub(crate) fn simulate(entity: &mut Entity, ctx: &mut StepContext<'_>, dt: f64) {
    let ClassState::Projectile(state) = &entity.class_state else {
        return;
    };
    let Some(ammo) = state.ammo.clone() else {
        entity.position += entity.velocity * dt;
        return;
    };
    let mut state = state.clone();

    // 1. Timers.
    state.time_to_live -= dt;
    if state.time_to_live <= 0.0 {
        ctx.delete(entity.id);
        entity.class_state = ClassState::Projectile(state);
        return;
    }
    if state.explosion_timer < f64::MAX {
        state.explosion_timer -= dt;
        if state.explosion_timer <= 0.0 {
            explode(entity, &mut state, &ammo, ctx);
            ctx.delete(entity.id);
            entity.class_state = ClassState::Projectile(state);
            return;
        }
    }

    // 2. The move.
    let mut flight = Flight {
        entity,
        state: &mut state,
        ammo: &ammo,
        ignore: Vec::new(),
        stopped: false,
    };
    flight.ignore = ignore_list(flight.entity, flight.state, ctx);
    flight.advance(ctx, dt, false, 0);
    let stopped = flight.stopped;
    if stopped {
        ctx.delete(entity.id);
    }
    entity.class_state = ClassState::Projectile(state);
}

/// The Objects a shot's segment test passes through: the shot itself, its shooter, and the
/// vehicle the shooter is in (`Landscape::FilterIgnoreTwo` / `FilterIgnoreOne`).
fn ignore_list(entity: &Entity, state: &ProjectileState, ctx: &StepContext<'_>) -> Vec<ObjectKey> {
    let mut ignore = vec![body_key(entity.id)];
    if let Some(shooter) = state.shooter {
        ignore.push(body_key(shooter));
        if let Some(vehicle) = ctx
            .world()
            .entity(shooter)
            .and_then(|e| e.attachment())
            .map(|a| a.to)
        {
            ignore.push(body_key(vehicle));
        }
    }
    ignore
}

/// The collision world's key of an Entity's body.
pub(crate) fn body_key(id: EntityId) -> ObjectKey {
    ObjectKey::Entity(ObjectRef::Entity(id).to_body_key())
}

/// The Object a collision key names, as scripts see it (a promoted Static object is its Entity).
pub(crate) fn object_of_key(world: &crate::World, key: ObjectKey) -> Option<ObjectRef> {
    match key {
        ObjectKey::Terrain => None,
        ObjectKey::Entity(handle) => ObjectRef::from_handle_id(handle),
        ObjectKey::Static(raw) => {
            StaticKey::from_raw(raw).map(|key| world.object_ref_of_static(key))
        }
    }
}

/// One shot's move, with what it needs from its step.
struct Flight<'a> {
    entity: &'a mut Entity,
    state: &'a mut ProjectileState,
    ammo: &'a AmmoType,
    ignore: Vec<ObjectKey>,
    /// Set when the shot stops (hits and is consumed); the step deletes it.
    stopped: bool,
}

impl Flight<'_> {
    /// `0x140e66ea0`: moves the shot by `dt`, resolving what it hits on the way. `continuing` is
    /// set for the rest of a step after a ricochet or a penetration.
    fn advance(&mut self, ctx: &mut StepContext<'_>, dt: f64, continuing: bool, depth: u32) {
        if dt <= 0.0 || self.stopped {
            return;
        }
        let mut dt = dt;
        let v = self.entity.velocity;
        let speed = v.length();
        // A very draggy shot moves at most 4 m per test.
        if self.friction() < -0.99 && speed > 0.0 {
            dt = dt.min(4.0 / speed);
        }
        let p = self.entity.position;
        let end = p + v * dt;
        let length = speed * dt;
        if length <= 0.0 {
            self.move_to(dt, end, 1.0);
            return;
        }
        let Some(collision) = ctx.world().collision_world() else {
            self.move_to(dt, end, 1.0);
            return;
        };
        let dir = v / speed;
        let start = if continuing {
            p + dir * CONTINUE_OFFSET
        } else {
            p
        };

        let terrain = collision.ray_cast(&RayQuery::new(p, end, LayerMask::NONE));
        let terrain_distance = terrain.map_or(f64::MAX, |h| h.distance);
        // Right after leaving a surface, a terrain hit closer than 0.1 m does not count.
        let terrain_counts = !continuing || terrain_distance >= CONTINUE_OFFSET;
        let object = collision
            .ray_cast(
                &RayQuery::new(start, end, Layer::FireGeometry)
                    .without_terrain()
                    .ignoring(&self.ignore),
            )
            .filter(|h| !terrain_counts || h.distance < terrain_distance);

        if let Some(hit) = object {
            let along = hit.distance + if continuing { CONTINUE_OFFSET } else { 0.0 };
            let t = (along / speed).min(dt);
            self.move_to(t, hit.position, 1.0);
            let remaining = dt - t;
            let target = object_of_key(ctx.world(), hit.object);
            let surface = hit.surface.map(|s| collision.surface(s).clone());
            let deflect = surface.as_ref().map_or(1.0, |s| f64::from(s.deflection));
            // `objLimit` (the hit object's `vfunc +0x350`) is not decoded; every Object lets a
            // grazing shot ricochet (deviation, see `docs/re/sim-ballistics.md` §4.1).
            let object_limit = 1.0;
            if self.ricochet(
                ctx,
                Some(&hit),
                target,
                along,
                remaining,
                object_limit,
                deflect,
                depth,
            ) {
                return;
            }
            if self.penetrate(ctx, collision, &hit, target, along, remaining, depth) {
                return;
            }
            self.stop(ctx, Some(&hit), target);
            return;
        }

        match terrain.filter(|_| terrain_counts && length > terrain_distance) {
            None => self.move_to(dt, end, 1.0),
            Some(hit) => {
                let t = (terrain_distance / speed).min(dt);
                self.move_to(t, hit.position, 1.0);
                let deflect = collision
                    .surface_below(hit.position + DVec3::Y * 0.5, 1.0)
                    .and_then(|s| s.surface)
                    .map_or(1.0, |s| f64::from(collision.surface(s).deflection));
                if self.ricochet(
                    ctx,
                    None,
                    None,
                    terrain_distance,
                    dt - t,
                    1.0,
                    deflect,
                    depth,
                ) {
                    return;
                }
                self.stop(ctx, Some(&hit), None);
            }
        }
    }

    /// `airFriction` (the original switches to `waterFriction` under water; there is no water
    /// test yet).
    fn friction(&self) -> f64 {
        self.ammo.air_friction
    }

    /// `0x140e6c3d0`: puts the shot at `to` after `dt` of flight, counts the distance off the
    /// arming distance, and steps the velocity — only when `0 < factor ≤ caliber·1000` (the
    /// penetration passes the surface's resistance as the factor).
    fn move_to(&mut self, dt: f64, to: DVec3, factor: f64) {
        if self.state.fuse_distance > 0.0 {
            self.state.fuse_distance -= to.distance(self.entity.position);
        } else {
            self.state.fuse_distance = 0.0;
        }
        self.entity.position = to;
        if factor > 0.0 && factor <= self.ammo.caliber * 1000.0 {
            let v = self.entity.velocity;
            let k = self.friction();
            let a = k * v.length() * v - DVec3::new(0.0, GRAVITY * self.ammo.coef_gravity, 0.0);
            let mut dv = a * dt;
            let (v2, dv2) = (v.length_squared(), dv.length_squared());
            // Never reverses the velocity. The original divides by |dv| even when it is 0 (a
            // shot at rest without gravity), which makes NaN; a zero step changes nothing here.
            if v2 <= dv2 {
                dv = if dv2 > 0.0 {
                    dv * (v2.sqrt() / dv2.sqrt())
                } else {
                    DVec3::ZERO
                };
            }
            self.entity.velocity = v + dv;
        }
    }

    /// §4.1 (`0x140e65820`). `along` is the distance flown to the hit; `object_limit` the hit
    /// object's ricochet limit; `deflect` the surface's `deflection`. A terrain hit has no
    /// `hit` record.
    #[allow(clippy::too_many_arguments)]
    fn ricochet(
        &mut self,
        ctx: &mut StepContext<'_>,
        hit: Option<&RayHit>,
        target: Option<ObjectRef>,
        along: f64,
        remaining: f64,
        object_limit: f64,
        deflect: f64,
        depth: u32,
    ) -> bool {
        let max_sin = (deflect * self.ammo.deflecting).sin().max(0.0);
        if !(object_limit > 0.0 || max_sin > 0.0) {
            return false;
        }
        let normal = match hit {
            Some(h) => h.normal,
            None => self.terrain_normal(ctx),
        };
        let d = self.ammo.deflection_dir_distribution;
        let rng = ctx.random();
        let jitter = DVec3::new(rng.spread(d), rng.spread(d), rng.spread(d));
        let n = (normal + jitter).normalize_or_zero();
        let v = self.entity.velocity;
        let sin_g = -n.dot(v.normalize_or_zero());
        let _ = along; // the original's slow-shell rolling branch reads it; rolling is not done
        if !(sin_g >= 0.0 && sin_g < max_sin && sin_g < object_limit)
            || v.length_squared() <= RICOCHET_MIN_SPEED_SQ
        {
            return false;
        }
        let reflected = v - 2.0 * v.dot(n) * n;
        if reflected.dot(normal) <= 0.0 {
            return false;
        }
        let keep = (1.0 - (sin_g / max_sin).powi(2))
            .max(0.0)
            .min(self.ammo.deflection_slow_down);
        let v_out = reflected * (keep * ctx.random().min_mid_max(0.6, 0.9, 1.0));
        if let (Some(hit), Some(target)) = (hit, target) {
            self.record_hit(ctx, hit, target, v_out);
        }
        self.entity.velocity = v_out;
        if depth < MAX_CONTINUATIONS {
            self.advance(ctx, remaining, true, depth + 1);
        }
        true
    }

    /// §4.2 (`0x140e69b00`): only Objects are penetrated, not the terrain.
    #[allow(clippy::too_many_arguments)]
    fn penetrate(
        &mut self,
        ctx: &mut StepContext<'_>,
        collision: &CollisionWorld,
        hit: &RayHit,
        target: Option<ObjectRef>,
        along: f64,
        remaining: f64,
        depth: u32,
    ) -> bool {
        let Some(target) = target else {
            return false;
        };
        let Some(surface) = hit.surface.map(|s| collision.surface(s)) else {
            return false;
        };
        let resistance = f64::from(surface.penetration_resistance);
        if !(resistance > 0.0 && (self.ammo.explosive < 0.7 || resistance <= 100.0)) {
            return false;
        }
        let v = self.entity.velocity;
        let speed = v.length();
        if speed <= 0.0 {
            return false;
        }
        let dir = v / speed;
        // The length of the flight inside the hit component.
        let mut length = component_depth(collision, hit, dir, speed * remaining.max(0.0));
        if let Some(thickness) = surface.thickness.map(f64::from).filter(|t| *t > 0.0) {
            // A plate material: the declared thickness along the flight, not the geometry.
            let cos = hit.normal.dot(dir);
            if cos != 0.0 {
                length = (thickness / cos).abs();
            }
        }
        let caliber = self.ammo.caliber;
        let loss = resistance / caliber * length;
        if loss.is_nan() || loss >= speed {
            return false;
        }
        let exit = hit.position + dir * length;
        let f = loss / speed;
        let d = self.ammo.penetration_dir_distribution;
        let rng = ctx.random();
        // The original draws z, y, x in that order.
        let (z, y, x) = (rng.spread(d), rng.spread(d), rng.spread(d));
        let dir_out = (dir + DVec3::new(x, y, z) * f).normalize_or_zero();
        let v_out = dir_out * (speed - loss);
        let t = (length / speed).min(remaining);
        self.move_to(t, exit, resistance);
        self.record_hit(ctx, hit, target, v_out);
        self.entity.velocity = v_out;
        let _ = along;
        if depth < MAX_CONTINUATIONS {
            self.advance(ctx, remaining - t, true, depth + 1);
        }
        true
    }

    /// §4.3: an armed shot explodes (when explosive) and deals its hit; every stopped shot is
    /// consumed.
    fn stop(&mut self, ctx: &mut StepContext<'_>, hit: Option<&RayHit>, target: Option<ObjectRef>) {
        self.stopped = true;
        if !self.state.armed() {
            return;
        }
        if let (Some(hit), Some(target)) = (hit, target) {
            self.record_hit(ctx, hit, target, DVec3::ZERO);
        }
        if !self.state.exploded && self.ammo.explosive > 0.0 {
            let ammo = Arc::new(self.ammo.clone());
            explode(self.entity, self.state, &ammo, ctx);
        }
    }

    /// The terrain normal under the shot (for a ricochet off the ground).
    fn terrain_normal(&self, ctx: &StepContext<'_>) -> DVec3 {
        ctx.world()
            .collision_world()
            .and_then(|c| c.surface_below(self.entity.position + DVec3::Y * 0.5, 1.0))
            .map_or(DVec3::Y, |s| s.normal)
    }

    /// Queues the hit (§5) with the shot's velocity at the hit as `v_in`.
    fn record_hit(
        &mut self,
        ctx: &mut StepContext<'_>,
        hit: &RayHit,
        target: ObjectRef,
        velocity_out: DVec3,
    ) {
        let velocity_in = self.entity.velocity;
        let value = hit_value(
            self.ammo,
            velocity_in.length(),
            velocity_out.length(),
            self.ammo.explosion_time > 0.0,
        );
        let collision = ctx.world().collision_world();
        let component = collision.and_then(|c| {
            let shape = hit.shape?;
            Some(c.component(shape, hit.component?)?.name.clone())
        });
        let surface = collision.and_then(|c| hit.surface.map(|s| c.surface(s).name.clone()));
        // `R`: the model's bounding sphere; an MLOD model stores none, so its layer's extent
        // stands in rather than dividing by zero.
        let radius = collision
            .and_then(|c| c.shape(hit.shape?))
            .map(|s| match s.model_bounding_sphere() {
                r if r > 0.0 => r,
                _ => s.bounding_radius(),
            })
            .unwrap_or(0.0);
        ctx.push(Command::Hit(Box::new(HitRecord {
            shot: self.entity.id,
            shooter: self.state.shooter,
            target,
            ammo: Arc::new(self.ammo.clone()),
            position: hit.position,
            normal: hit.normal,
            velocity_in,
            velocity_out,
            value,
            component,
            surface,
            radius,
        })));
    }
}

/// The depth of the component `hit` entered, measured along the flight for at most `reach`
/// metres past the entry (at least 10 m, so a thick wall reads as thick).
fn component_depth(collision: &CollisionWorld, hit: &RayHit, dir: DVec3, reach: f64) -> f64 {
    let from = hit.position - dir * 0.01;
    let to = hit.position + dir * reach.max(10.0);
    collision
        .penetrations(
            &RayQuery::new(from, to, hit.layer.map_or(LayerMask::FIRE, Layer::mask))
                .without_terrain(),
        )
        .into_iter()
        .find(|p| p.entry.object == hit.object && p.entry.component == hit.component)
        .map_or(0.0, |p| p.depth())
}

/// §5: the direct hit value `hit · e` of a shot that went from `speed_in` to `speed_out`.
pub(crate) fn hit_value(ammo: &AmmoType, speed_in: f64, speed_out: f64, has_fuse: bool) -> f64 {
    // `shotCoef` (`Shot+0x638`) is 1 for every shot we make.
    let shot_coef = 1.0;
    let mut e = if ammo.explosive < 1.0 {
        ((speed_in - speed_out) / ammo.typical_speed * shot_coef).min(2.0) * (1.0 - ammo.explosive)
    } else {
        0.0
    };
    if speed_out <= 1e-6 && !has_fuse {
        e += ammo.explosive;
    }
    ammo.hit * e
}

/// The shot explodes where it is (`0x140e5bf10`): queued for the World's explosion.
fn explode(
    entity: &Entity,
    state: &mut ProjectileState,
    ammo: &Arc<AmmoType>,
    ctx: &mut StepContext<'_>,
) {
    if state.exploded {
        return;
    }
    state.exploded = true;
    ctx.push(Command::Explosion(Box::new(ExplosionRecord {
        shot: entity.id,
        shooter: state.shooter,
        ammo: ammo.clone(),
        position: entity.position,
    })));
}
