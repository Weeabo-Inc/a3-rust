//! A unit's weapons and magazines (its Loadout) and the trigger: which weapon and muzzle is
//! selected, which magazine each muzzle has loaded, how many rounds are left, and when the next
//! round can go.
//!
//! Sources: `docs/re/sim-weapons.md` (rate of fire, bursts, magazine reload, tracers) and the
//! SQF command semantics in `docs/re/sim-damage.md`'s neighbour `sqf-commands.tsv`.
//!
//! The World steps every Loadout once per frame ([`World::step_loadouts`]): timers run down, an
//! empty muzzle reloads from the carried magazines, and a pulled trigger fires through
//! [`World::fire`] from the unit's aim ([`World::set_aim`]).

use std::sync::Arc;

use glam::DVec3;

use crate::fire::FireRequest;
use crate::weapons::{MagazineType, WeaponType};
use crate::{EntityId, Error, World};

/// A magazine: its class and the rounds left in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Magazine {
    pub kind: Arc<MagazineType>,
    pub ammo: u32,
}

impl Magazine {
    /// The `CfgMagazines` class name.
    pub fn name(&self) -> &str {
        &self.kind.name
    }
}

/// One muzzle of a carried weapon: its loaded magazine and its trigger state.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MuzzleSlot {
    /// The loaded magazine, `None` when empty.
    pub magazine: Option<Magazine>,
    /// The selected fire mode, an index into the muzzle's modes.
    pub mode: usize,
    /// Seconds until the next round can go (the mode's `reloadTime`).
    pub ready_in: f64,
    /// Seconds left of a magazine reload in progress; the magazine goes in when it reaches 0.
    pub reloading: Option<(f64, Magazine)>,
    /// Rounds of the current burst still to fire.
    pub burst_left: u32,
    /// Rounds fired from the loaded magazine (for `tracersEvery`).
    pub fired: u32,
}

/// A carried weapon and its muzzles.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponSlot {
    pub kind: Arc<WeaponType>,
    /// One per muzzle of the weapon, in `muzzles[]` order.
    pub muzzles: Vec<MuzzleSlot>,
}

/// Where the unit's selected muzzle is and where it points, World space. Until weapon models are
/// posed the host supplies it ([`World::set_aim`]); without one a unit fires from its eyes along
/// its facing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aim {
    pub from: DVec3,
    pub direction: DVec3,
}

/// A unit's weapons and magazines.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Loadout {
    pub weapons: Vec<WeaponSlot>,
    /// Carried magazines that are not loaded, in the order they were added.
    pub magazines: Vec<Magazine>,
    /// The selected weapon and muzzle (indices).
    pub current: Option<(usize, usize)>,
    /// Whether the fire action is held.
    pub trigger: bool,
    /// Whether the trigger was held last frame (a semi-automatic mode fires on the press).
    pub(crate) trigger_was: bool,
    pub aim: Option<Aim>,
}

/// How high above its position a unit without an aim fires from.
const EYE_HEIGHT: f64 = 1.5;

/// Timer leftovers below this are rounding noise: config times are `f32`, frame times `f64`.
const TIME_EPSILON: f64 = 1e-6;

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

    /// Whether `magazine` fits muzzle `(w, m)`.
    fn accepts(&self, (w, m): (usize, usize), magazine: &str) -> bool {
        self.weapons
            .get(w)
            .and_then(|slot| slot.kind.muzzles.get(m))
            .is_some_and(|muzzle| {
                muzzle
                    .magazines
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(magazine))
            })
    }

    /// The carried magazine to load into `(w, m)`: the fullest one of the class it last had, else
    /// the fullest one it accepts.
    fn take_magazine_for(&mut self, at: (usize, usize), prefer: Option<&str>) -> Option<Magazine> {
        let pick = |only: Option<&str>| {
            self.magazines
                .iter()
                .enumerate()
                .filter(|(_, mag)| mag.ammo > 0 && self.accepts(at, mag.name()))
                .filter(|(_, mag)| only.is_none_or(|n| mag.name().eq_ignore_ascii_case(n)))
                .max_by_key(|(i, mag)| (mag.ammo, usize::MAX - i))
                .map(|(i, _)| i)
        };
        let index = prefer.and_then(|p| pick(Some(p))).or_else(|| pick(None))?;
        Some(self.magazines.remove(index))
    }

    fn slot(&self, (w, m): (usize, usize)) -> Option<&MuzzleSlot> {
        self.weapons.get(w)?.muzzles.get(m)
    }

    fn slot_mut(&mut self, (w, m): (usize, usize)) -> Option<&mut MuzzleSlot> {
        self.weapons.get_mut(w)?.muzzles.get_mut(m)
    }
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

    /// `addWeapon`: gives the unit a weapon and loads each muzzle from its carried magazines at
    /// once. The first weapon a unit gets is selected.
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
            if let Some(mag) = loadout.take_magazine_for((w, m), None) {
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
        loadout.magazines.push(Magazine { kind, ammo });
        Ok(())
    }

    /// `weapons`: the carried weapons' classes.
    pub fn weapons_of(&self, unit: EntityId) -> Vec<String> {
        self.loadout(unit)
            .map(|l| l.weapons.iter().map(|w| w.kind.name.clone()).collect())
            .unwrap_or_default()
    }

    /// `magazines`: the carried magazines that are not loaded, by class.
    pub fn magazines_of(&self, unit: EntityId) -> Vec<String> {
        self.loadout(unit)
            .map(|l| l.magazines.iter().map(|m| m.name().to_owned()).collect())
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
                let (w, m) = l.current?;
                let muzzle = l.weapons.get(w)?.kind.muzzles.get(m)?;
                let mode = muzzle.modes.get(l.slot((w, m))?.mode)?;
                Some(mode.name.clone())
            })
            .unwrap_or_default()
    }

    /// `ammo`: the rounds in the magazine loaded in `muzzle` (a weapon class or a muzzle name).
    pub fn ammo_in(&self, unit: EntityId, muzzle: &str) -> u32 {
        self.loadout(unit)
            .and_then(|l| l.slot(l.find_muzzle(muzzle)?)?.magazine.as_ref())
            .map_or(0, |m| m.ammo)
    }

    /// `setAmmo`: sets the rounds in the magazine loaded in `muzzle`, clamped to its `count`.
    /// Does nothing when no magazine is loaded.
    pub fn set_ammo(&mut self, unit: EntityId, muzzle: &str, ammo: u32) {
        if let Some(loadout) = self.loadout_mut(unit) {
            if let Some(at) = loadout.find_muzzle(muzzle) {
                if let Some(mag) = loadout.slot_mut(at).and_then(|s| s.magazine.as_mut()) {
                    mag.ammo = ammo.min(mag.kind.count);
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
        loadout.weapons[w].muzzles[m].mode = index;
        true
    }

    /// `reload`: starts a magazine change on every muzzle that has a fuller magazine to take.
    pub fn reload(&mut self, unit: EntityId) {
        let Some(loadout) = self.loadout_mut(unit) else {
            return;
        };
        for w in 0..loadout.weapons.len() {
            for m in 0..loadout.weapons[w].muzzles.len() {
                start_reload(loadout, (w, m), true);
            }
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

    /// `fire` / `forceWeaponFire`: fires one round from `muzzle` (the selected one when `None`)
    /// in `mode` (its selected one when `None`) if it is loaded and ready. Returns the shot, or
    /// `None` when the muzzle cannot fire now.
    pub fn fire_weapon(
        &mut self,
        unit: EntityId,
        muzzle: Option<&str>,
        mode: Option<&str>,
    ) -> Result<Option<EntityId>, Error> {
        let loadout = self.loadout(unit).ok_or(Error::NoSuchEntity(unit))?;
        let at = match muzzle {
            Some(name) => loadout.find_muzzle(name),
            None => loadout.current,
        };
        let Some(at) = at else {
            return Ok(None);
        };
        let mode_index = match mode {
            Some(name) => loadout.weapons[at.0].kind.muzzles[at.1]
                .modes
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(name)),
            None => Some(loadout.slot(at).map_or(0, |s| s.mode)),
        };
        let Some(mode_index) = mode_index else {
            return Ok(None);
        };
        self.fire_round(unit, at, mode_index)
    }

    /// Fires one round from muzzle `at` in mode `mode` when loaded and ready; spends the round
    /// and starts the mode's `reloadTime`.
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
        let Some(magazine) = slot.magazine.as_ref().filter(|m| m.ammo > 0) else {
            return Ok(None);
        };
        if slot.ready_in > TIME_EPSILON || slot.reloading.is_some() {
            return Ok(None);
        }
        let weapon = loadout.weapons[at.0].kind.clone();
        let muzzle = &weapon.muzzles[at.1];
        let Some(mode_type) = muzzle.modes.get(mode) else {
            return Ok(None);
        };
        let aim = loadout.aim.unwrap_or_else(|| Aim {
            from: entity.position() + DVec3::new(0.0, EYE_HEIGHT, 0.0),
            direction: entity.orientation() * DVec3::Z,
        });
        let kind = &magazine.kind;
        let count = kind.count;
        // Tracers: every `tracersEvery`-th round, and the last `lastRoundsTracer` rounds.
        let round = slot.fired + 1;
        let tracer = (kind.tracers_every > 0 && round % kind.tracers_every == 0)
            || magazine.ammo <= kind.last_rounds_tracer;
        let _ = count;
        let request = FireRequest::new(
            unit,
            weapon.name.clone(),
            kind.name.clone(),
            aim.from,
            aim.direction,
        )
        .muzzle(muzzle.name.clone())
        .mode(mode_type.name.clone())
        .tracer(tracer);
        let reload_time = mode_type.reload_time;
        let shot = self.fire(request)?;
        if let Some(slot) = self.loadout_mut(unit).and_then(|l| l.slot_mut(at)) {
            if let Some(mag) = slot.magazine.as_mut() {
                mag.ammo -= 1;
            }
            slot.fired += 1;
            slot.ready_in = reload_time;
        }
        Ok(Some(shot))
    }

    /// One frame of every unit's weapons: timers, automatic reloads, and the held trigger.
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
        // Timers and reloads.
        {
            let Some(loadout) = self.loadout_mut(unit) else {
                return;
            };
            for w in 0..loadout.weapons.len() {
                for m in 0..loadout.weapons[w].muzzles.len() {
                    let slot = &mut loadout.weapons[w].muzzles[m];
                    slot.ready_in = (slot.ready_in - dt).max(0.0);
                    if let Some((left, _)) = slot.reloading.as_mut() {
                        *left -= dt;
                        if *left <= TIME_EPSILON {
                            let (_, mag) = slot.reloading.take().expect("checked");
                            slot.magazine = Some(mag);
                            slot.fired = 0;
                            slot.ready_in = 0.0;
                        }
                    }
                    // An empty muzzle reloads by itself.
                    let empty = slot.magazine.as_ref().is_none_or(|m| m.ammo == 0);
                    if empty && slot.reloading.is_none() {
                        start_reload(loadout, (w, m), false);
                    }
                }
            }
        }

        // The trigger.
        let Some(loadout) = self.loadout(unit) else {
            return;
        };
        let (pulled, was) = (loadout.trigger, loadout.trigger_was);
        let Some(at) = loadout.current else {
            if let Some(l) = self.loadout_mut(unit) {
                l.trigger_was = pulled;
            }
            return;
        };
        let Some(slot) = loadout.slot(at) else {
            return;
        };
        let mode_index = slot.mode;
        let Some(mode) = loadout.weapons[at.0].kind.muzzles[at.1]
            .modes
            .get(mode_index)
            .cloned()
        else {
            return;
        };
        let pressed = pulled && !was;
        let mut burst_left = slot.burst_left;
        if pressed {
            burst_left = mode.burst.max(1);
        }
        let wants = burst_left > 0 || (pulled && mode.auto_fire);
        if wants {
            if let Ok(Some(_)) = self.fire_round(unit, at, mode_index) {
                burst_left = burst_left.saturating_sub(1);
            }
        }
        if let Some(l) = self.loadout_mut(unit) {
            l.trigger_was = pulled;
            if let Some(slot) = l.slot_mut(at) {
                // A burst ends with an empty magazine.
                slot.burst_left = if slot.magazine.as_ref().is_some_and(|m| m.ammo > 0) {
                    burst_left
                } else {
                    0
                };
            }
        }
    }
}

/// Starts a magazine reload of `at` when the unit carries a magazine for it: the magazine comes
/// out of the inventory now and goes in after the muzzle's `magazineReloadTime`; the one it
/// replaces goes back into the inventory unless empty. `forced` (`reload`) changes a magazine
/// that still has rounds.
fn start_reload(loadout: &mut Loadout, at: (usize, usize), forced: bool) {
    let Some(slot) = loadout.slot(at) else {
        return;
    };
    if slot.reloading.is_some() {
        return;
    }
    let current = slot
        .magazine
        .as_ref()
        .map(|m| (m.name().to_owned(), m.ammo));
    if !forced && current.as_ref().is_some_and(|(_, ammo)| *ammo > 0) {
        return;
    }
    let prefer = current.as_ref().map(|(n, _)| n.clone());
    let Some(next) = loadout.take_magazine_for(at, prefer.as_deref()) else {
        return;
    };
    if forced && current.as_ref().is_some_and(|(_, ammo)| next.ammo <= *ammo) {
        loadout.magazines.push(next);
        return;
    }
    let time = loadout.weapons[at.0].kind.muzzles[at.1].magazine_reload_time;
    let slot = loadout.slot_mut(at).expect("checked");
    if let Some(old) = slot.magazine.take().filter(|m| m.ammo > 0) {
        loadout.magazines.push(old);
    }
    let slot = loadout.slot_mut(at).expect("checked");
    slot.burst_left = 0;
    if time <= 0.0 {
        slot.magazine = Some(next);
        slot.fired = 0;
    } else {
        slot.reloading = Some((time, next));
    }
}
