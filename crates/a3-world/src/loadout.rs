//! A unit's weapons and magazines (its Loadout) and the trigger: which weapon and muzzle is
//! selected, which magazine each muzzle has loaded, how many rounds are left, and when the next
//! round can go.
//!
//! Source: `docs/re/sim-weapons.md` §2–§3 (`WeaponsState`, the fire path, the round and magazine
//! reload, bursts and requests) and §6 (the commands).
//!
//! The World steps every Loadout once per frame ([`World::step_loadouts`], the engine's weapons
//! simulate `0x140f90e40`): the loaded magazines' reload timers run down, a burst in progress
//! fires its next round, a held trigger on an `autoFire` mode keeps firing, and a pending `fire`
//! request goes when the weapon is ready. Rounds leave through [`World::fire`] from the unit's
//! aim ([`World::set_aim`]).
//!
//! _Deviations_: the soldier's reload action (the `reloadAction` gesture) is not played, so a
//! magazine goes in at once and only `magazineReloadTime` delays the next round; the AI skill
//! factors (`reloadSpeed`, `aimingAccuracy`) are 1.

use std::sync::Arc;

use glam::DVec3;

use crate::fire::FireRequest;
use crate::weapons::{MagazineType, ModeType, WeaponType};
use crate::{EntityId, Error, World};

/// A magazine: its class, the rounds left in it, and its reload state (`Magazine+0x70..+0x7c`).
#[derive(Debug, Clone, PartialEq)]
pub struct Magazine {
    pub kind: Arc<MagazineType>,
    pub ammo: u32,
    /// The round reload phase: 1 just after a shot, falling to 0 over `reloadTime · factor`.
    pub round_phase: f64,
    /// The round reload duration factor.
    pub round_factor: f64,
    /// Seconds left of the magazine reload.
    pub reload_left: f64,
    /// The magazine reload's total seconds.
    pub reload_total: f64,
}

impl Magazine {
    fn new(kind: Arc<MagazineType>, ammo: u32) -> Self {
        Self {
            kind,
            ammo,
            round_phase: 0.0,
            round_factor: 1.0,
            reload_left: 0.0,
            reload_total: 0.0,
        }
    }

    /// The `CfgMagazines` class name.
    pub fn name(&self) -> &str {
        &self.kind.name
    }

    /// Ready for the next round (`0x140fb5be0`): rounds left, no round or magazine reload running.
    pub fn ready(&self) -> bool {
        self.ammo > 0 && self.round_phase <= 0.0 && self.reload_left <= 0.0
    }
}

/// One muzzle of a carried weapon: its loaded magazine and trigger state.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MuzzleSlot {
    /// The loaded magazine, `None` when there is none.
    pub magazine: Option<Magazine>,
    /// The selected fire mode, an index into the muzzle's modes.
    pub mode: usize,
    /// Rounds of the current burst still to fire (`MuzzleState+0x44`).
    pub burst_left: u32,
}

/// A carried weapon and its muzzles.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponSlot {
    pub kind: Arc<WeaponType>,
    /// One per muzzle of the weapon, in `muzzles[]` order.
    pub muzzles: Vec<MuzzleSlot>,
}

/// Where the unit's selected muzzle is and where it points, World space. The engine takes them
/// from the weapon proxy's memory points (`sim-weapons.md` §2.2); until weapon models are posed
/// the host supplies them ([`World::set_aim`]), and without one a unit fires from its eyes along
/// its facing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aim {
    pub from: DVec3,
    pub direction: DVec3,
}

/// A unit's weapons and magazines (`WeaponsState`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Loadout {
    pub weapons: Vec<WeaponSlot>,
    /// Carried magazines that are not loaded, in the order they were added.
    pub magazines: Vec<Magazine>,
    /// The selected weapon and muzzle (indices; `WeaponsState+0x2c`).
    pub current: Option<(usize, usize)>,
    /// A `fire` request: weapon, muzzle and mode indices (`WeaponsState+0x30`).
    pub request: Option<(usize, usize, usize)>,
    /// Whether the fire action is held.
    pub trigger: bool,
    /// Whether it was held last frame: a trigger pull starts on the press.
    pub(crate) trigger_was: bool,
    pub aim: Option<Aim>,
}

/// How high above its position a unit without an aim fires from.
const EYE_HEIGHT: f64 = 1.5;

impl Loadout {
    /// The weapon slot holding `weapon` (case-insensitive).
    fn weapon_index(&self, weapon: &str) -> Option<usize> {
        self.weapons
            .iter()
            .position(|w| w.kind.name.eq_ignore_ascii_case(weapon))
    }

    /// The (weapon, muzzle) a muzzle name selects: a weapon's class name selects its body
    /// (`"this"`), any other name a named muzzle of a carried weapon.
    pub fn find_muzzle(&self, name: &str) -> Option<(usize, usize)> {
        if let Some(w) = self.weapon_index(name) {
            let m = self.weapons[w]
                .kind
                .muzzles
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(crate::weapons::DEFAULT_MUZZLE))
                .unwrap_or(0);
            return Some((w, m));
        }
        self.weapons.iter().enumerate().find_map(|(w, slot)| {
            slot.kind
                .muzzles
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(name))
                .map(|m| (w, m))
        })
    }

    /// The script name of a muzzle: the weapon class for the body, else the muzzle's name.
    pub fn muzzle_name(&self, (w, m): (usize, usize)) -> Option<String> {
        let weapon = &self.weapons.get(w)?.kind;
        let muzzle = weapon.muzzles.get(m)?;
        Some(
            if muzzle
                .name
                .eq_ignore_ascii_case(crate::weapons::DEFAULT_MUZZLE)
            {
                weapon.name.clone()
            } else {
                muzzle.name.clone()
            },
        )
    }

    fn slot(&self, (w, m): (usize, usize)) -> Option<&MuzzleSlot> {
        self.weapons.get(w)?.muzzles.get(m)
    }

    fn slot_mut(&mut self, (w, m): (usize, usize)) -> Option<&mut MuzzleSlot> {
        self.weapons.get_mut(w)?.muzzles.get_mut(m)
    }

    fn mode_type(&self, (w, m): (usize, usize), mode: usize) -> Option<&ModeType> {
        self.weapons.get(w)?.kind.muzzles.get(m)?.modes.get(mode)
    }

    /// The carried magazine to load into `at` (`0x140fa51b0`): of the same class as `old`, the
    /// one with the most rounds (the first on a tie); otherwise, walking the muzzle's magazine
    /// list in order, the fullest of the first class that has any. Empty magazines do not count.
    fn pick_magazine(&self, at: (usize, usize), old: Option<&str>) -> Option<usize> {
        let fullest = |class: &str| {
            let mut best: Option<(usize, u32)> = None;
            for (i, mag) in self.magazines.iter().enumerate() {
                if mag.ammo > 0
                    && mag.name().eq_ignore_ascii_case(class)
                    && best.is_none_or(|(_, ammo)| mag.ammo > ammo)
                {
                    best = Some((i, mag.ammo));
                }
            }
            best.map(|(i, _)| i)
        };
        if let Some(i) = old.and_then(fullest) {
            return Some(i);
        }
        let muzzle = self.weapons.get(at.0)?.kind.muzzles.get(at.1)?;
        muzzle
            .magazines
            .iter()
            .filter(|class| old.is_none_or(|o| !o.eq_ignore_ascii_case(class)))
            .find_map(|class| fullest(class))
    }
}

/// The random part of a reload duration: `U(1 ± spread)` (`0x14030e240`).
fn spread_factor(world: &mut World, spread: f64) -> f64 {
    1.0 - spread + 2.0 * spread * world.random.uniform()
}

impl World {
    fn loadout(&self, unit: EntityId) -> Option<&Loadout> {
        self.entity(unit).map(|e| &e.loadout)
    }

    fn loadout_mut(&mut self, unit: EntityId) -> Option<&mut Loadout> {
        self.entity_mut(unit).map(|e| &mut e.loadout)
    }

    /// A unit's Loadout.
    pub fn unit_loadout(&self, unit: EntityId) -> Option<&Loadout> {
        self.loadout(unit)
    }

    /// Gives a new unit the weapons and magazines its class lists (`CfgVehicles >> class >>
    /// weapons[]`, `magazines[]`), as creation does: the magazines first, then each weapon, which
    /// loads its muzzles from them. Needs the World's config ([`World::set_config`]); names the
    /// config does not have are skipped.
    pub(crate) fn arm_from_config(&mut self, unit: EntityId) {
        let Some(armory) = self.armory.as_ref() else {
            return;
        };
        let Some(ty) = self.entity(unit).map(|e| e.entity_type().clone()) else {
            return;
        };
        if ty.source() != crate::TypeSource::Vehicles
            || !ty.class().is_kind_of(crate::EntityClass::EntityAi)
        {
            return;
        }
        // By name: the type may come from another tree than the World's config.
        let cfg = armory
            .bank
            .config()
            .root()
            .get("CfgVehicles")
            .get(ty.name());
        if !cfg.is_class() {
            return;
        }
        let list = |name: &str| -> Vec<String> {
            cfg.get(name)
                .array()
                .into_iter()
                .filter_map(|v| match v {
                    a3_config::Value::String(s) => Some(s),
                    _ => None,
                })
                .collect()
        };
        let (weapons, magazines) = (list("weapons"), list("magazines"));
        for magazine in magazines {
            let _ = self.add_magazine(unit, &magazine, None);
        }
        for weapon in weapons {
            let _ = self.add_weapon(unit, &weapon);
        }
    }

    /// `addWeapon`: gives the unit a weapon and loads each muzzle from its carried magazines at
    /// once (the engine's reload of all weapons after creation). The first weapon a unit gets is
    /// selected.
    pub fn add_weapon(&mut self, unit: EntityId, weapon: &str) -> Result<(), Error> {
        let kind = self
            .armory
            .as_mut()
            .ok_or(Error::NoConfig)?
            .bank
            .weapon(weapon)?;
        let loadout = self.loadout_mut(unit).ok_or(Error::NoSuchEntity(unit))?;
        if loadout.weapon_index(&kind.name).is_some() {
            return Ok(());
        }
        let w = loadout.weapons.len();
        loadout.weapons.push(WeaponSlot {
            muzzles: vec![MuzzleSlot::default(); kind.muzzles.len()],
            kind,
        });
        for m in 0..loadout.weapons[w].muzzles.len() {
            if let Some(i) = loadout.pick_magazine((w, m), None) {
                let mag = loadout.magazines.remove(i);
                if let Some(slot) = loadout.slot_mut((w, m)) {
                    slot.magazine = Some(mag);
                }
            }
        }
        if loadout.current.is_none() {
            loadout.current = Some((w, 0));
        }
        Ok(())
    }

    /// `removeWeapon`: drops the weapon and the magazines loaded in it. Returns whether the unit
    /// had it.
    pub fn remove_weapon(&mut self, unit: EntityId, weapon: &str) -> bool {
        let Some(loadout) = self.loadout_mut(unit) else {
            return false;
        };
        let Some(w) = loadout.weapon_index(weapon) else {
            return false;
        };
        loadout.weapons.remove(w);
        loadout.request = None;
        loadout.current = match loadout.current {
            Some((cw, _)) if cw == w => (!loadout.weapons.is_empty()).then_some((0, 0)),
            Some((cw, cm)) if cw > w => Some((cw - 1, cm)),
            other => other,
        };
        true
    }

    /// `addMagazine`: adds a magazine, full or with `ammo` rounds (clamped to its `count`).
    pub fn add_magazine(
        &mut self,
        unit: EntityId,
        magazine: &str,
        ammo: Option<u32>,
    ) -> Result<(), Error> {
        let kind = self
            .armory
            .as_mut()
            .ok_or(Error::NoConfig)?
            .bank
            .magazine(magazine)?;
        let loadout = self.loadout_mut(unit).ok_or(Error::NoSuchEntity(unit))?;
        let ammo = ammo.unwrap_or(kind.count).min(kind.count);
        loadout.magazines.push(Magazine::new(kind, ammo));
        Ok(())
    }

    /// `weapons`: the carried weapons' classes.
    pub fn weapons_of(&self, unit: EntityId) -> Vec<String> {
        self.loadout(unit)
            .map(|l| l.weapons.iter().map(|w| w.kind.name.clone()).collect())
            .unwrap_or_default()
    }

    /// `magazines`: the carried magazines that are not loaded and not empty, by class
    /// (`0x14082cec0` skips empty ones).
    pub fn magazines_of(&self, unit: EntityId) -> Vec<String> {
        self.loadout(unit)
            .map(|l| {
                l.magazines
                    .iter()
                    .filter(|m| m.ammo > 0)
                    .map(|m| m.name().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `primaryWeapon`: the carried weapon whose `CfgWeapons` `type` is 1 (rifles), or "".
    pub fn primary_weapon(&self, unit: EntityId) -> String {
        self.loadout(unit)
            .and_then(|l| l.weapons.iter().find(|w| w.kind.kind & 1 != 0))
            .map(|w| w.kind.name.clone())
            .unwrap_or_default()
    }

    /// `currentWeapon`: the selected weapon's class, or "".
    pub fn current_weapon(&self, unit: EntityId) -> String {
        self.loadout(unit)
            .and_then(|l| l.weapons.get(l.current?.0))
            .map(|w| w.kind.name.clone())
            .unwrap_or_default()
    }

    /// `currentMuzzle`: the selected muzzle's script name, or "".
    pub fn current_muzzle(&self, unit: EntityId) -> String {
        self.loadout(unit)
            .and_then(|l| l.muzzle_name(l.current?))
            .unwrap_or_default()
    }

    /// `currentMagazine`: the class of the magazine in the selected muzzle, or "".
    pub fn current_magazine(&self, unit: EntityId) -> String {
        self.loadout(unit)
            .and_then(|l| l.slot(l.current?)?.magazine.as_ref())
            .map(|m| m.name().to_owned())
            .unwrap_or_default()
    }

    /// `currentWeaponMode`: the selected muzzle's mode name, or "".
    pub fn current_weapon_mode(&self, unit: EntityId) -> String {
        self.loadout(unit)
            .and_then(|l| {
                let at = l.current?;
                Some(l.mode_type(at, l.slot(at)?.mode)?.name.clone())
            })
            .unwrap_or_default()
    }

    /// `ammo`: the rounds in the magazine loaded in `muzzle` (a weapon class or a muzzle name).
    pub fn ammo_in(&self, unit: EntityId, muzzle: &str) -> u32 {
        self.loadout(unit)
            .and_then(|l| l.slot(l.find_muzzle(muzzle)?)?.magazine.as_ref())
            .map_or(0, |m| m.ammo)
    }

    /// `setAmmo`: sets the rounds in the magazine loaded in `muzzle`; `None` (a negative count)
    /// or more than its `count` fills it (`0x1411176b0`). Does nothing when no magazine is
    /// loaded.
    pub fn set_ammo(&mut self, unit: EntityId, muzzle: &str, ammo: Option<u32>) {
        if let Some(loadout) = self.loadout_mut(unit) {
            if let Some(at) = loadout.find_muzzle(muzzle) {
                if let Some(mag) = loadout.slot_mut(at).and_then(|s| s.magazine.as_mut()) {
                    let count = mag.kind.count;
                    mag.ammo = ammo.filter(|n| *n <= count).unwrap_or(count);
                }
            }
        }
    }

    /// `selectWeapon`: selects a muzzle by weapon class or muzzle name. Returns whether the unit
    /// has it.
    pub fn select_weapon(&mut self, unit: EntityId, muzzle: &str) -> bool {
        let Some(loadout) = self.loadout_mut(unit) else {
            return false;
        };
        match loadout.find_muzzle(muzzle) {
            Some(at) => {
                loadout.current = Some(at);
                true
            }
            None => false,
        }
    }

    /// Selects a fire mode of the current muzzle by name. Returns whether it has the mode.
    pub fn select_mode(&mut self, unit: EntityId, mode: &str) -> bool {
        let Some(loadout) = self.loadout_mut(unit) else {
            return false;
        };
        let Some((w, m)) = loadout.current else {
            return false;
        };
        let Some(index) = loadout.weapons[w].kind.muzzles[m]
            .modes
            .iter()
            .position(|x| x.name.eq_ignore_ascii_case(mode))
        else {
            return false;
        };
        let slot = &mut loadout.weapons[w].muzzles[m];
        slot.mode = index;
        slot.burst_left = 0;
        true
    }

    /// `reload`: changes the magazine of every muzzle that has another one to take.
    pub fn reload(&mut self, unit: EntityId) {
        let muzzles: Vec<(usize, usize)> = self
            .loadout(unit)
            .map(|l| {
                l.weapons
                    .iter()
                    .enumerate()
                    .flat_map(|(w, slot)| (0..slot.muzzles.len()).map(move |m| (w, m)))
                    .collect()
            })
            .unwrap_or_default();
        for at in muzzles {
            self.reload_muzzle(unit, at);
        }
    }

    /// Changes the magazine of one muzzle when the unit carries one for it (`0x140fe0010`): the
    /// new magazine goes in, the old one goes back to the inventory unless it is empty, and the
    /// muzzle waits `magazineReloadTime · U(1 ± 0.2)` before the next round.
    fn reload_muzzle(&mut self, unit: EntityId, at: (usize, usize)) {
        let Some(loadout) = self.loadout(unit) else {
            return;
        };
        let old = loadout
            .slot(at)
            .and_then(|s| s.magazine.as_ref())
            .map(|m| m.name().to_owned());
        let Some(index) = loadout.pick_magazine(at, old.as_deref()) else {
            return;
        };
        let reload_time = loadout.weapons[at.0].kind.muzzles[at.1].magazine_reload_time;
        let reload = reload_time * spread_factor(self, 0.2);
        let factor = spread_factor(self, 0.1);
        let Some(loadout) = self.loadout_mut(unit) else {
            return;
        };
        let mut mag = loadout.magazines.remove(index);
        mag.reload_left = reload;
        mag.reload_total = reload;
        mag.round_phase = 0.0;
        mag.round_factor = if mag.kind.quick_reload { 1.0 } else { factor };
        let slot = loadout.slot_mut(at).expect("checked");
        slot.burst_left = 0;
        if let Some(old) = slot.magazine.replace(mag).filter(|m| m.ammo > 0) {
            loadout.magazines.push(old);
        }
    }

    /// Holds or releases the unit's fire action (the player's `defaultAction`).
    pub fn set_trigger(&mut self, unit: EntityId, pulled: bool) {
        if let Some(loadout) = self.loadout_mut(unit) {
            loadout.trigger = pulled;
        }
    }

    /// Sets where the unit's selected muzzle is and points (`None`: from the eyes along the
    /// facing).
    pub fn set_aim(&mut self, unit: EntityId, aim: Option<Aim>) {
        if let Some(loadout) = self.loadout_mut(unit) {
            loadout.aim = aim;
        }
    }

    /// The muzzle and mode a script names: `muzzle` (the selected one when `None`), `mode` (its
    /// selected one when `None`).
    fn resolve_muzzle(
        &self,
        unit: EntityId,
        muzzle: Option<&str>,
        mode: Option<&str>,
    ) -> Option<((usize, usize), usize)> {
        let loadout = self.loadout(unit)?;
        let at = match muzzle {
            Some(name) => loadout.find_muzzle(name)?,
            None => loadout.current?,
        };
        let mode = match mode {
            Some(name) => loadout.weapons[at.0].kind.muzzles[at.1]
                .modes
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(name))?,
            None => loadout.slot(at)?.mode,
        };
        Some((at, mode))
    }

    /// `fire`: requests one round from `muzzle` in `mode` (`WeaponsState+0x30`). The request goes
    /// in a later step, when that muzzle is the selected one and ready; no request is made when
    /// its magazine is empty. Returns whether a request was made.
    pub fn request_fire(
        &mut self,
        unit: EntityId,
        muzzle: Option<&str>,
        mode: Option<&str>,
    ) -> bool {
        let Some((at, mode)) = self.resolve_muzzle(unit, muzzle, mode) else {
            return false;
        };
        let Some(loadout) = self.loadout_mut(unit) else {
            return false;
        };
        let loaded = loadout
            .slot(at)
            .and_then(|s| s.magazine.as_ref())
            .is_some_and(|m| m.ammo > 0);
        if loaded {
            loadout.request = Some((at.0, at.1, mode));
        }
        loaded
    }

    /// Fires one round now from `muzzle` in `mode` if it is loaded and ready (`FireWeapon`;
    /// `forceWeaponFire`). Returns the shot, or `None` when it cannot fire (the engine plays the
    /// dry sound).
    pub fn fire_weapon(
        &mut self,
        unit: EntityId,
        muzzle: Option<&str>,
        mode: Option<&str>,
    ) -> Result<Option<EntityId>, Error> {
        if self.entity(unit).is_none() {
            return Err(Error::NoSuchEntity(unit));
        }
        match self.resolve_muzzle(unit, muzzle, mode) {
            Some((at, mode)) => self.fire_round(unit, at, mode),
            None => Ok(None),
        }
    }

    /// `FireWeapon` + `PostFire` (`0x1407862e0`, `0x140fd5600`): fires one round from muzzle
    /// `at` in `mode` when ready, then starts the round reload, counts the burst down and takes
    /// the rounds.
    fn fire_round(
        &mut self,
        unit: EntityId,
        at: (usize, usize),
        mode: usize,
    ) -> Result<Option<EntityId>, Error> {
        let entity = self.entity(unit).ok_or(Error::NoSuchEntity(unit))?;
        let loadout = &entity.loadout;
        let Some(slot) = loadout.slot(at) else {
            return Ok(None);
        };
        let Some(magazine) = slot.magazine.as_ref().filter(|m| m.ready()) else {
            return Ok(None);
        };
        let burst_left = slot.burst_left;
        let weapon = loadout.weapons[at.0].kind.clone();
        let muzzle = &weapon.muzzles[at.1];
        let Some(mode_type) = muzzle.modes.get(mode).cloned() else {
            return Ok(None);
        };
        let aim = loadout.aim.unwrap_or_else(|| Aim {
            from: entity.position() + DVec3::new(0.0, EYE_HEIGHT, 0.0),
            direction: entity.orientation() * DVec3::Z,
        });
        let kind = magazine.kind.clone();
        let rounds = magazine.ammo;
        // §2.5, with `r` the rounds before this shot.
        let last = kind.last_rounds_tracer;
        let tracer = rounds <= last
            || (kind.tracers_every >= 1 && (rounds - last) % kind.tracers_every == 0);
        let request = FireRequest::new(
            unit,
            weapon.name.clone(),
            kind.name.clone(),
            aim.from,
            aim.direction,
        )
        .muzzle(muzzle.name.clone())
        .mode(mode_type.name.clone())
        .rounds(rounds)
        .tracer(tracer);
        let shot = self.fire(request)?;

        // PostFire (§3.2). A new trigger pull sets the burst length.
        let left = if burst_left == 0 {
            let mut n = mode_type.burst;
            if let Some(max) = mode_type.burst_range_max {
                let extra = (self.random.uniform() * f64::from(max.saturating_sub(n))) as u32;
                n = (n + extra).min(max.saturating_sub(1)).max(mode_type.burst);
            }
            if mode_type.multiplier > 0 {
                n = n.min(rounds / mode_type.multiplier);
            }
            n
        } else {
            burst_left
        };
        // After the last round of a semi-automatic pull the wait is randomised by ±10 %.
        let last_of_pull = !mode_type.auto_fire && (left == 1 || mode_type.burst == 0);
        let factor = if last_of_pull {
            spread_factor(self, 0.1)
        } else {
            1.0
        };
        let player = self.player() == Some(unit);
        let Some(slot) = self.loadout_mut(unit).and_then(|l| l.slot_mut(at)) else {
            return Ok(Some(shot));
        };
        slot.burst_left = left.saturating_sub(1);
        let Some(mag) = slot.magazine.as_mut() else {
            return Ok(Some(shot));
        };
        mag.round_phase = 1.0;
        mag.round_factor = factor;
        mag.ammo -= mode_type.multiplier.min(mag.ammo);
        if mag.ammo == 0 {
            // §3.4: the round reload stops; a soldier drops an empty magazine that says so, or
            // a one-round one that does not say (`0x140748690`).
            mag.round_phase = 0.0;
            slot.burst_left = 0;
            if mag.kind.delete_if_empty.unwrap_or(mag.kind.count == 1) {
                slot.magazine = None;
            }
            // Auto reload: AI always, a player only on an `autoReload` muzzle (`0x140f95940`).
            if !player || weapon.muzzles[at.1].auto_reload {
                self.reload_muzzle(unit, at);
            }
        }
        Ok(Some(shot))
    }

    /// One frame of every unit's weapons: the reload timers, then bursts, held triggers and
    /// requests.
    pub(crate) fn step_loadouts(&mut self, dt: f64) {
        let units: Vec<EntityId> = self
            .entities()
            .filter(|e| !e.loadout.weapons.is_empty() && e.is_local() && e.is_alive())
            .map(|e| e.id())
            .collect();
        for unit in units {
            self.step_loadout(unit, dt);
        }
    }

    fn step_loadout(&mut self, unit: EntityId, dt: f64) {
        let Some(loadout) = self.loadout_mut(unit) else {
            return;
        };
        // §3.3: every loaded magazine with rounds runs its magazine reload, else its round
        // reload, at the reload time of its muzzle's selected mode.
        for weapon in &mut loadout.weapons {
            let kind = weapon.kind.clone();
            for (m, slot) in weapon.muzzles.iter_mut().enumerate() {
                let reload_time = kind.muzzles[m]
                    .modes
                    .get(slot.mode)
                    .map_or(0.0, |mode| mode.reload_time);
                let Some(mag) = slot.magazine.as_mut().filter(|mag| mag.ammo > 0) else {
                    continue;
                };
                if mag.reload_left > 0.0 {
                    mag.reload_left = (mag.reload_left - dt).max(0.0);
                } else if mag.round_phase > 0.0 {
                    mag.round_phase = if reload_time > 0.0 {
                        (mag.round_phase - dt / (reload_time * mag.round_factor)).max(0.0)
                    } else {
                        0.0
                    };
                    // Config times are `f32`, frame times `f64`: what is left of a phase that
                    // has run its time is rounding.
                    if mag.round_phase < 1e-6 {
                        mag.round_phase = 0.0;
                    }
                }
            }
        }

        let pulled = loadout.trigger;
        let pressed = pulled && !loadout.trigger_was;
        loadout.trigger_was = pulled;
        let Some(at) = loadout.current else {
            return;
        };
        let Some(slot) = loadout.slot(at) else {
            return;
        };
        let mode = slot.mode;
        let auto_fire = loadout.mode_type(at, mode).is_some_and(|m| m.auto_fire);
        let ready = slot.magazine.as_ref().is_some_and(Magazine::ready);
        // A burst in progress fires its next round as soon as the weapon is ready (§3.3).
        let burst = slot.burst_left > 0;
        let request = loadout.request.filter(|(w, m, _)| (*w, *m) == at);

        if burst || pressed || (pulled && auto_fire) {
            if ready {
                let _ = self.fire_round(unit, at, mode);
            }
        } else if let Some((_, _, mode)) = request {
            // A request fires only from the selected muzzle; a failed one stays.
            if ready && matches!(self.fire_round(unit, at, mode), Ok(Some(_))) {
                if let Some(l) = self.loadout_mut(unit) {
                    l.request = None;
                }
            }
        }
    }
}
