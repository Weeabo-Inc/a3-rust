//! Firing, hits and explosions: [`World::fire`] makes a shot, the projectile step
//! (`sim::projectile`) flies it, and [`World::apply_hit`] / [`World::explode`] turn what it hits
//! into damage and events.
//!
//! Sources: `docs/re/sim-ballistics.md` §3–§6, `docs/re/sim-weapons.md` (the fire path).

use std::collections::HashMap;
use std::sync::Arc;

use a3_config::ConfigTree;
use a3_physics::{Interest, LayerMask, ObjectKey, QueryShape, TerrainStatics};
use glam::{DQuat, DVec3};

use crate::sim::{ExplosionRecord, HitRecord, ProjectileState, body_key};
use crate::weapons::{AmmoType, WeaponBank};
use crate::{
    ClassState, DamageHit, EntityId, Error, ListKind, Locality, Near, ObjectRef, World, WorldEvent,
};

/// One shot to fire: who fires, from which weapon and magazine, from where and where to.
#[derive(Debug, Clone, PartialEq)]
pub struct FireRequest {
    pub shooter: EntityId,
    /// The `CfgWeapons` class.
    pub weapon: String,
    /// The `CfgMagazines` class the round comes from.
    pub magazine: String,
    /// The muzzle; `None` is the weapon body (`"this"`).
    pub muzzle: Option<String>,
    /// The fire mode; `None` is the muzzle's first.
    pub mode: Option<String>,
    /// The muzzle position, World space.
    pub from: DVec3,
    /// The aim; any length but zero.
    pub direction: DVec3,
    /// Whether the shot draws a tracer.
    pub tracer: bool,
    /// The rounds in the magazine before this shot: with the direction it seeds the shot's
    /// dispersion and picks the tracer (`sim-weapons.md` §2.2, §2.5). The engine never fires
    /// from an empty magazine, so it is at least 1.
    pub rounds: u32,
}

impl FireRequest {
    pub fn new(
        shooter: EntityId,
        weapon: impl Into<String>,
        magazine: impl Into<String>,
        from: DVec3,
        direction: DVec3,
    ) -> Self {
        Self {
            shooter,
            weapon: weapon.into(),
            magazine: magazine.into(),
            muzzle: None,
            mode: None,
            from,
            direction,
            tracer: false,
            rounds: 1,
        }
    }

    /// The rounds in the magazine before this shot (`sim-weapons.md` §2.2, §2.5).
    pub fn rounds(mut self, rounds: u32) -> Self {
        self.rounds = rounds;
        self
    }

    /// Fires from the named muzzle instead of the weapon body.
    pub fn muzzle(mut self, muzzle: impl Into<String>) -> Self {
        self.muzzle = Some(muzzle.into());
        self
    }

    /// Fires in the named mode instead of the muzzle's first.
    pub fn mode(mut self, mode: impl Into<String>) -> Self {
        self.mode = Some(mode.into());
        self
    }

    /// Marks the shot as a tracer round.
    pub fn tracer(mut self, tracer: bool) -> Self {
        self.tracer = tracer;
        self
    }
}

/// What a shot was fired with: the `Fired` handlers' arguments.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FiredShot {
    pub weapon: String,
    pub muzzle: String,
    pub mode: String,
    pub ammo: String,
    pub magazine: String,
    /// The unit whose weapon fired (the shooter for a man).
    pub gunner: Option<EntityId>,
}

/// The World's weapons config: the parameter caches and the `CfgAmmo` Entity types of shots.
#[derive(Debug)]
pub(crate) struct Armory {
    pub(crate) bank: WeaponBank,
    shot_types: HashMap<String, Arc<crate::EntityType>>,
}

impl Armory {
    fn new(config: Arc<ConfigTree>) -> Self {
        Self {
            bank: WeaponBank::new(config),
            shot_types: HashMap::new(),
        }
    }

    /// The Entity type of a `CfgAmmo` class.
    fn shot_type(&mut self, ammo: &str) -> Result<Arc<crate::EntityType>, Error> {
        let key = ammo.to_ascii_lowercase();
        if let Some(t) = self.shot_types.get(&key) {
            return Ok(t.clone());
        }
        let cfg = self.bank.config().root().get("CfgAmmo").get(ammo);
        if !cfg.is_class() {
            return Err(Error::UnknownAmmo(ammo.to_owned()));
        }
        let ty = Arc::new(crate::EntityType::from_config(
            crate::TypeSource::Ammo,
            &cfg,
        )?);
        self.shot_types.insert(key, ty.clone());
        Ok(ty)
    }
}

impl World {
    /// Installs the merged config the World reads weapons, magazines and ammunition from
    /// (`CfgWeapons`, `CfgMagazines`, `CfgAmmo`), dropping the parameters cached from the previous
    /// one.
    pub fn set_config(&mut self, config: Arc<ConfigTree>) {
        self.armory = Some(Armory::new(config));
    }

    /// The config [`set_config`](Self::set_config) installed.
    pub fn config(&self) -> Option<&ConfigTree> {
        self.armory.as_ref().map(|a| a.bank.config())
    }

    /// The weapon parameter caches, once a config is installed.
    pub fn weapon_bank(&mut self) -> Option<&mut WeaponBank> {
        self.armory.as_mut().map(|a| &mut a.bank)
    }

    /// Seeds the World's random source (dispersion, ricochet and penetration spread).
    pub fn set_random_seed(&mut self, seed: u64) {
        self.random = crate::random::EngineRandom::new((seed ^ (seed >> 32)) as u32);
    }

    /// The collision world, mutably: for a host that adds or moves Entity bodies.
    pub fn collision_mut(&mut self) -> Option<&mut a3_physics::CollisionWorld> {
        self.collision_world_mut()
    }

    /// Streams the collision world's Static objects and terrain chunks around `interests`, from
    /// the loaded terrain's objects. Does nothing without a terrain or a collision world.
    pub fn stream_collision(&mut self, interests: &[Interest]) {
        let Some(terrain) = self.terrain().cloned() else {
            return;
        };
        if let Some(collision) = self.collision_world_mut() {
            collision.stream(&TerrainStatics::new(terrain), interests);
        }
    }

    /// Fires one round: resolves the weapon's muzzle and mode, the magazine and its ammunition,
    /// and creates the shot — a local [`ListKind::Projectiles`] Entity at `from`, flying at the
    /// muzzle velocity along `direction` spread by the mode's `dispersion`. Records
    /// [`WorldEvent::Fired`].
    ///
    /// This is the low-level fire path: it does not check or spend the shooter's ammunition
    /// (see [`World::fire_weapon`]).
    pub fn fire(&mut self, request: FireRequest) -> Result<EntityId, Error> {
        if self.armory.is_none() {
            return Err(Error::NoConfig);
        }
        let shooter_velocity = self
            .entity(request.shooter)
            .filter(|e| !e.is_deleted())
            .map(|e| e.velocity())
            .ok_or(Error::NoSuchEntity(request.shooter))?;
        let armory = self.armory.as_mut().expect("checked above");
        let weapon = armory.bank.weapon(&request.weapon)?;
        let params = {
            let muzzle =
                weapon
                    .muzzle(request.muzzle.as_deref())
                    .ok_or_else(|| Error::NoSuchMuzzle {
                        weapon: weapon.name.clone(),
                        muzzle: request.muzzle.clone().unwrap_or_default(),
                    })?;
            let mode = muzzle
                .mode(request.mode.as_deref())
                .ok_or_else(|| Error::NoSuchMode {
                    weapon: weapon.name.clone(),
                    muzzle: muzzle.name.clone(),
                    mode: request.mode.clone().unwrap_or_default(),
                })?;
            (muzzle.clone(), mode.clone())
        };
        let (muzzle, mode) = params;
        // Scripts name the weapon body's muzzle by the weapon's class.
        let muzzle_name = if muzzle
            .name
            .eq_ignore_ascii_case(crate::weapons::DEFAULT_MUZZLE)
        {
            weapon.name.clone()
        } else {
            muzzle.name.clone()
        };
        let magazine = armory.bank.magazine(&request.magazine)?;
        let ammo = armory.bank.ammo(&magazine.ammo)?;
        let shot_type = armory.shot_type(&magazine.ammo)?;
        let direction = request
            .direction
            .try_normalize()
            .ok_or(Error::ZeroDirection)?;

        let speed = crate::weapons::ShotParams {
            weapon: &weapon,
            muzzle: &muzzle,
            mode: &mode,
        }
        .init_speed(&magazine, &ammo);
        // §2.2: a bullet keeps the dispersed direction's length, every other shot is normalised;
        // the shooter's velocity adds on.
        let aim = disperse(direction, mode.dispersion, request.rounds);
        let bullet = matches!(
            ammo.simulation,
            Some(crate::SimulationClass::ShotBullet | crate::SimulationClass::ShotSpread)
        );
        let velocity = if bullet { aim } else { aim.normalize() } * speed + shooter_velocity;

        let shot = self.insert(
            shot_type,
            request.from,
            None,
            Locality::Local,
            ListKind::Projectiles,
        );
        if let Some(e) = self.entity_mut(shot) {
            e.velocity = velocity;
            e.orientation = orientation_along(aim.normalize());
            e.anchor_visual_state();
            let mut state = ProjectileState::new(ammo);
            state.shooter = Some(request.shooter);
            state.weapon = weapon.name.clone();
            state.muzzle = muzzle_name.clone();
            state.mode = mode.name.clone();
            state.magazine = magazine.name.clone();
            state.tracer = request.tracer;
            e.class_state = ClassState::Projectile(state);
        }
        self.fired_shots.insert(
            shot,
            FiredShot {
                weapon: weapon.name.clone(),
                muzzle: muzzle_name,
                mode: mode.name.clone(),
                ammo: magazine.ammo.clone(),
                magazine: magazine.name.clone(),
                gunner: Some(request.shooter),
            },
        );
        self.record(WorldEvent::Fired {
            shot,
            shooter: request.shooter,
            weapon: weapon.name.clone(),
        });
        Ok(shot)
    }

    /// Gives a shot created from a `CfgAmmo` type (`createVehicle "B_65x39_Caseless"`) its ammo
    /// parameters, so it flies and hits like a fired one.
    pub(crate) fn init_projectile(&mut self, id: EntityId) {
        let Some(name) = self
            .entity(id)
            .filter(|e| e.list() == ListKind::Projectiles)
            .map(|e| e.type_name().to_owned())
        else {
            return;
        };
        let Some(ammo) = self.armory.as_mut().and_then(|a| a.bank.ammo(&name).ok()) else {
            return;
        };
        if let Some(e) = self.entity_mut(id) {
            if let ClassState::Projectile(state) = &mut e.class_state {
                if state.ammo.is_none() {
                    *state = ProjectileState::new(ammo);
                }
            }
        }
    }

    /// Applies a shot's direct hit (`docs/re/sim-ballistics.md` §5) and records
    /// [`WorldEvent::Hit`].
    ///
    /// The total damage is `(rd/R)²·D` with `rd = 0.27·√value`, `D = value / armor` and `R` the
    /// hit layer's bounding radius. _Deviation_: the per-hit-point distribution over the
    /// HitPoints LOD (§7.2) needs that LOD's geometry, which the World does not load yet, so a
    /// direct hit changes only the total.
    pub(crate) fn apply_hit(&mut self, hit: HitRecord) {
        let target = self.current_object(hit.target);
        let mut damage = 0.0;
        if hit.value > 0.0 {
            if let Some(armor) = self.armor_of(target) {
                let radius = if hit.radius > 0.0 { hit.radius } else { 1.0 };
                let rd = 0.27 * hit.value.sqrt();
                let total = ((rd / radius).powi(2) * hit.value / armor).min(2000.0);
                damage = self.damage_from_shot(target, total, hit.shooter);
            }
        }
        let target = self.current_object(target);
        self.record(WorldEvent::Hit {
            shot: hit.shot,
            shooter: hit.shooter,
            target,
            ammo: hit.ammo.name.clone(),
            position: hit.position,
            normal: hit.normal,
            velocity: hit.velocity_in,
            speed_in: hit.velocity_in.length(),
            speed_out: hit.velocity_out.length(),
            value: hit.value,
            direct: true,
            component: hit.component,
            surface: hit.surface,
            radius: hit.radius,
            damage,
        });
    }

    /// A shot's explosion (`docs/re/sim-ballistics.md` §6): every Object within `4·r` of
    /// `position` takes `0.33·(fn + fc + ff)·indirectHit`, where `f(x)` is 1 inside `r` and
    /// `r⁴/x⁴` outside, at the distances from the explosion to the Object's near side, centre and
    /// far side. Records [`WorldEvent::Exploded`] and one [`WorldEvent::Hit`] per damaged Object.
    pub(crate) fn explode(&mut self, explosion: ExplosionRecord) {
        let ammo = explosion.ammo.clone();
        let r = ammo.indirect_hit_range;
        let center = explosion.position;
        self.record(WorldEvent::Exploded {
            shot: explosion.shot,
            shooter: explosion.shooter,
            ammo: ammo.name.clone(),
            position: center,
            radius: r,
        });
        for (object, value) in self.blast(center, &ammo) {
            let damage = match self.armor_of(object) {
                Some(armor) => self.damage_from_shot(object, value / armor, explosion.shooter),
                None => 0.0,
            };
            let target = self.current_object(object);
            self.record(WorldEvent::Hit {
                shot: explosion.shot,
                shooter: explosion.shooter,
                target,
                ammo: ammo.name.clone(),
                position: center,
                normal: DVec3::ZERO,
                velocity: DVec3::ZERO,
                speed_in: 0.0,
                speed_out: 0.0,
                value,
                direct: false,
                component: None,
                surface: None,
                radius: r,
                damage,
            });
        }
    }

    /// The indirect hit value of every Object a blast of `ammo` at `center` reaches.
    fn blast(&self, center: DVec3, ammo: &AmmoType) -> Vec<(ObjectRef, f64)> {
        let r = ammo.indirect_hit_range;
        let reach = 4.0 * r;
        if ammo.indirect_hit <= 0.0 || reach <= 0.0 {
            return Vec::new();
        }
        // The bounding sphere of each Object around the blast (`shape+0x7c`: the model's
        // `ModelInfo::bounding_sphere`), found through its Fire Geometry.
        let mut radii: HashMap<ObjectKey, f64> = HashMap::new();
        if let Some(collision) = self.collision_world() {
            for o in collision.overlaps(
                QueryShape::Sphere { radius: reach },
                center,
                DQuat::IDENTITY,
                LayerMask::FIRE,
                false,
                &[],
            ) {
                let radius = o
                    .shape
                    .and_then(|s| collision.shape(s))
                    .map_or(0.0, |s| s.model_bounding_sphere());
                let entry = radii.entry(o.object).or_insert(0.0);
                *entry = entry.max(radius);
            }
        }
        let f = |x: f64| {
            if x * x <= r * r {
                1.0
            } else {
                r.powi(4) / x.powi(4)
            }
        };
        let mut out = Vec::new();
        for (object, d) in self.objects_near(center, reach, Near::All) {
            if let ObjectRef::Entity(id) = object {
                if self
                    .entity(id)
                    .is_none_or(|e| e.list() == ListKind::Projectiles)
                {
                    continue;
                }
            }
            let radius = radii.get(&object_key(object)).copied().unwrap_or(0.0);
            let (f_near, f_centre, f_far) = (f((d - radius).max(0.0)), f(d), f(d + radius));
            if ammo.indirect_hit * f_near <= 0.0 {
                continue;
            }
            // `typeExplosionCoef` (`+0x2a8` of the type component) is 1 for every type we read.
            out.push((
                object,
                0.33 * (f_near + f_centre + f_far) * ammo.indirect_hit,
            ));
        }
        out
    }

    /// A Static object that has been promoted is its Entity.
    fn current_object(&self, object: ObjectRef) -> ObjectRef {
        match object {
            ObjectRef::Static(key) => self.object_ref_of_static(key),
            other => other,
        }
    }

    /// The armor a hit's value is divided by (the type's `armor`); a Static object that was
    /// never promoted has the default type's.
    fn armor_of(&self, object: ObjectRef) -> Option<f64> {
        let armor = match object {
            ObjectRef::Entity(id) => self.entity(id)?.entity_type().damage().armor(),
            ObjectRef::Static(_) => crate::DamageModel::new().armor(),
        };
        (armor > 0.0).then_some(f64::from(armor))
    }

    /// Adds `damage` to an Object's total as engine damage caused by `shooter`. Totals below the
    /// type's `minTotalDamageThreshold` are dropped (§6). Returns the change of the total.
    fn damage_from_shot(
        &mut self,
        object: ObjectRef,
        damage: f64,
        shooter: Option<EntityId>,
    ) -> f64 {
        let threshold = match object {
            ObjectRef::Entity(id) => self.entity(id).map_or(0.0, |e| {
                e.entity_type().damage().min_total_damage_threshold()
            }),
            ObjectRef::Static(_) => crate::DamageModel::new().min_total_damage_threshold(),
        };
        if damage.is_nan() || damage <= 0.0 || (damage as f32) < threshold {
            return 0.0;
        }
        let before = self.damage_of(object).unwrap_or(0.0);
        let mut hit = DamageHit::added(damage as f32);
        if let Some(shooter) = shooter {
            hit = hit.caused_by(shooter).instigated_by(shooter);
        }
        match self.apply_damage(object, hit) {
            Some(outcome) => f64::from(outcome.damage - before),
            None => 0.0,
        }
    }
}

/// The collision key of an Object.
fn object_key(object: ObjectRef) -> ObjectKey {
    match object {
        ObjectRef::Entity(id) => body_key(id),
        ObjectRef::Static(key) => ObjectKey::Static(key.raw()),
    }
}

/// The shot's direction after the mode's `dispersion` (`0x140fb05a0`, `sim-weapons.md` §2.2):
/// `aside·dx + up·dy + aim` in the frame of the aim with `(0, 1, 0)` as the up reference, each
/// offset `(ΣU·½ − 1)·dispersion` over four draws of a generator seeded per shot by FNV-1a-64
/// over the rounds before the shot and the aim (`f32` little-endian). Not normalised.
fn disperse(aim: DVec3, dispersion: f64, rounds: u32) -> DVec3 {
    if dispersion <= 0.0 {
        return aim;
    }
    let mut seed_bytes = Vec::with_capacity(16);
    seed_bytes.extend_from_slice(&(rounds as i32).to_le_bytes());
    for c in [aim.x, aim.y, aim.z] {
        seed_bytes.extend_from_slice(&(c as f32).to_le_bytes());
    }
    let mut rng = crate::random::CRandom::new(fnv1a64(&seed_bytes) as u32);
    let (mut sx, mut sy) = (0.0, 0.0);
    for _ in 0..4 {
        sx += rng.uniform();
        sy += rng.uniform();
    }
    let dx = (sx * 0.5 - 1.0) * dispersion;
    let dy = (sy * 0.5 - 1.0) * dispersion;
    // The engine's frame of a direction and an up vector: aside = up × dir, up' = dir × aside.
    let aside = DVec3::Y.cross(aim).try_normalize().unwrap_or(DVec3::X);
    let up = aim.cross(aside);
    aside * dx + up * dy + aim
}

/// FNV-1a, 64 bits.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// An orientation whose forward (+Z) is `dir`.
fn orientation_along(dir: DVec3) -> DQuat {
    DQuat::from_rotation_arc(DVec3::Z, dir)
}
