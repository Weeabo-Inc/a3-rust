//! Weapons and ammunition: the `CfgWeapons`, `CfgMagazines` and `CfgAmmo` parameters the
//! simulation reads.
//!
//! Sources: `docs/re/sim-ballistics.md` §1 (`CfgAmmo` → `AmmoType`) and §2 (the `simulation`
//! kinds); the offline Arma wiki for the config semantics of `muzzles[]`, `modes[]`, `initSpeed`,
//! `dispersion` and `recoil`.
//!
//! # Vocabulary
//!
//! - A **Muzzle** is one barrel or launcher of a weapon. `muzzles[] = {"this"}` (the default) means
//!   the weapon class itself is its only muzzle, so a weapon-body parameter is a muzzle parameter;
//!   a weapon with two barrels (`muzzles[] = {"this", "EGLM"}`) declares the extra ones as
//!   sub-classes of the weapon class. Muzzle-only parameters are stated as such.
//! - A **Mode** is one trigger setting of a muzzle (`Single`, `FullAuto`, ...), named by the
//!   muzzle's `modes[]` and declared as sub-classes of it. `dispersion` and the recoil class names
//!   live on the mode; when a muzzle declares no `modes[]`, the muzzle itself acts as its single
//!   `"this"` mode, which is what single-mode weapons (and most hand-written configs) do.
//! - `initSpeed` is the shot's muzzle velocity in m/s. The magazine's is the base; a muzzle's or
//!   mode's own value overrides it when positive and scales it when negative (`-1.1` multiplies by
//!   1.1), and leaves it alone when zero. The muzzle's declared value wins over the mode's.
//! - `dispersion` is the shot cone half-angle in **radians**.
//!
//! # Not read yet
//!
//! Firing-mode state (burst counts, reload times, `aiRateOfFire*`), `CfgRecoils` resolution (the
//! [`ModeType`] keeps the class names only), muzzle memory points (`usti hlavne` /
//! `konec hlavne`), and the missile parameters of §1 (`thrust`, `maneuvrability`, ...).

use std::collections::HashMap;
use std::sync::Arc;

use a3_config::{ConfigRef, ConfigTree, Value};

use crate::{Error, SimulationClass};

/// The name of the muzzle a weapon is its own muzzle under (`muzzles[] = {"this"}`).
pub const DEFAULT_MUZZLE: &str = "this";

/// `CfgAmmo` parameters the simulation reads (`docs/re/sim-ballistics.md` §1).
///
/// Defaults are the engine's where known (documented per field) and otherwise neutral.
#[derive(Debug, Clone, PartialEq)]
pub struct AmmoType {
    /// The `CfgAmmo` class name, e.g. `B_65x39_Ball`.
    pub name: String,
    /// `hit`: damage on a direct hit, at `typicalSpeed` (§5).
    pub hit: f64,
    /// `indirectHit`: damage at the centre of the explosion (§6).
    pub indirect_hit: f64,
    /// `indirectHitRange`: the explosion radius in metres (§6). Damage falls off as `r⁴/x⁴`
    /// outside it.
    pub indirect_hit_range: f64,
    /// `explosive` 0..1: how much of the damage is the explosion rather than the impact (§5).
    /// Above 0 the shot explodes when it stops (§4.3).
    pub explosive: f64,
    /// `caliber`: penetration power and the direct-damage scale (§4.2, §5). The engine reads `0`
    /// as "standard damage"; we treat a value ≤ 0 as "does not penetrate".
    pub caliber: f64,
    /// `deflecting`: the largest grazing angle that ricochets, **radians** (degrees in config).
    pub deflecting: f64,
    /// `deflectionSlowDown`: the cap on the speed kept by a ricochet (default 1).
    pub deflection_slow_down: f64,
    /// `deflectionDirDistribution`: the per-axis randomization of a ricochet's normal.
    pub deflection_dir_distribution: f64,
    /// `penetrationDirDistribution`: the per-axis randomization of a penetrating shot's direction.
    pub penetration_dir_distribution: f64,
    /// `airFriction`: negative for bullets, positive for self-propelled ammo. `a = k·|v|·v` (§3).
    pub air_friction: f64,
    /// `waterFriction`: the `airFriction` used while submerged (§3).
    pub water_friction: f64,
    /// `coefGravity`: the gravity multiplier (default 1, the wiki's default too).
    pub coef_gravity: f64,
    /// `typicalSpeed`: the speed `hit` is valid at (§5).
    pub typical_speed: f64,
    /// `timeToLive`: seconds before the shot is deleted; ≤ 0 means it never expires (§3).
    pub time_to_live: f64,
    /// `minTimeToLive`, kept as data.
    pub min_time_to_live: f64,
    /// `maxSpeed`, kept as data.
    pub max_speed: f64,
    /// `explosionTime`: the fuse, seconds after firing; ≤ 0 means it never fires (§3).
    pub explosion_time: f64,
    /// `fuseDistance`: the distance the shot must travel before it is armed, so a fused shot does
    /// not go off against its own launcher (§3).
    pub fuse_distance: f64,
    /// `simulation`: the C++ class that flies the shot, `None` when the name is unknown (§2).
    pub simulation: Option<SimulationClass>,
    /// `simulationStep`: the Entity's step length, when the class declares one.
    pub simulation_step: Option<f64>,
}

impl Default for AmmoType {
    fn default() -> Self {
        Self {
            name: String::new(),
            hit: 0.0,
            indirect_hit: 0.0,
            indirect_hit_range: 0.0,
            explosive: 0.0,
            caliber: 1.0,
            deflecting: 0.0,
            deflection_slow_down: 1.0,
            deflection_dir_distribution: 0.0,
            penetration_dir_distribution: 0.0,
            air_friction: 0.0,
            water_friction: 0.0,
            coef_gravity: 1.0,
            typical_speed: 0.0,
            time_to_live: 0.0,
            min_time_to_live: 0.0,
            max_speed: 0.0,
            explosion_time: 0.0,
            fuse_distance: 0.0,
            simulation: None,
            simulation_step: None,
        }
    }
}

/// `CfgMagazines` parameters: what a loaded magazine fires and how fast.
#[derive(Debug, Clone, PartialEq)]
pub struct MagazineType {
    pub name: String,
    /// `ammo`: the `CfgAmmo` class of the shots.
    pub ammo: String,
    /// `count`: the rounds in a full magazine.
    pub count: u32,
    /// `initSpeed`: the base muzzle velocity in m/s.
    pub init_speed: f64,
}

/// One trigger setting of a [`MuzzleType`] (`Single`, `FullAuto`, ... or `"this"`).
#[derive(Debug, Clone, PartialEq)]
pub struct ModeType {
    /// The mode class name, or `"this"` when the muzzle is its own mode.
    pub name: String,
    /// `dispersion`: the shot cone half-angle, radians.
    pub dispersion: f64,
    /// `recoil`: the `CfgRecoils` class this mode kicks with, kept as data.
    pub recoil: Option<String>,
    /// `recoilProne`: the `CfgRecoils` class used while prone.
    pub recoil_prone: Option<String>,
    /// `initSpeed`, when the class declares one; see [`ShotParams::init_speed`].
    pub init_speed: Option<f64>,
}

/// One muzzle of a weapon: its own magazines, `initSpeed` and modes.
#[derive(Debug, Clone, PartialEq)]
pub struct MuzzleType {
    /// The muzzle's name in `muzzles[]`; [`DEFAULT_MUZZLE`] for the weapon body.
    pub name: String,
    /// The config class the parameters come from: the weapon itself for `"this"`, otherwise the
    /// muzzle sub-class.
    pub class: String,
    /// `magazines[]`: the magazine classes this muzzle accepts.
    pub magazines: Vec<String>,
    /// The muzzle's own `initSpeed`, when its class declares one.
    pub init_speed: Option<f64>,
    /// `modes[]`, in declaration order; the first is the default. A muzzle with none is its own
    /// single `"this"` mode.
    pub modes: Vec<ModeType>,
}

impl MuzzleType {
    /// The muzzle's `mode`th mode. `None` picks the first declared one.
    pub fn mode(&self, name: Option<&str>) -> Option<&ModeType> {
        match name {
            Some(name) => self
                .modes
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(name)),
            None => self.modes.first(),
        }
    }
}

/// A `CfgWeapons` class: its muzzles, modes and magazines.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponType {
    pub name: String,
    /// `muzzles[]`, always at least the weapon body as `"this"`.
    pub muzzles: Vec<MuzzleType>,
    /// The weapon body's `magazines[]`; each muzzle holds its own list too.
    pub magazines: Vec<String>,
}

impl WeaponType {
    /// The muzzle called `name`; `None` picks the weapon body ([`DEFAULT_MUZZLE`]).
    pub fn muzzle(&self, name: Option<&str>) -> Option<&MuzzleType> {
        let name = name.unwrap_or(DEFAULT_MUZZLE);
        self.muzzles
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name))
    }

    /// The parameters of one shot: muzzle `muzzle` (the body when `None`) in mode `mode` (the
    /// muzzle's default when `None`).
    pub fn shot_params(&self, muzzle: Option<&str>, mode: Option<&str>) -> Option<ShotParams<'_>> {
        let muzzle = self.muzzle(muzzle)?;
        Some(ShotParams {
            muzzle,
            mode: muzzle.mode(mode)?,
        })
    }
}

/// One muzzle of one weapon in one of its modes: everything a single shot reads from `CfgWeapons`.
#[derive(Debug, Clone, Copy)]
pub struct ShotParams<'a> {
    pub muzzle: &'a MuzzleType,
    pub mode: &'a ModeType,
}

impl ShotParams<'_> {
    /// The shot's muzzle velocity: the magazine's `initSpeed`, overridden by a positive muzzle or
    /// mode value, scaled by a negative one, and taken as is when both are zero or absent.
    pub fn init_speed(&self, magazine: &MagazineType) -> f64 {
        let declared = self
            .muzzle
            .init_speed
            .or(self.mode.init_speed)
            .unwrap_or(0.0);
        if declared > 0.0 {
            declared
        } else if declared < 0.0 {
            -declared * magazine.init_speed
        } else {
            magazine.init_speed
        }
    }

    /// The shot cone half-angle in radians.
    pub fn dispersion(&self) -> f64 {
        self.mode.dispersion
    }

    /// The `CfgRecoils` class of a standing shot, as data.
    pub fn recoil(&self) -> Option<&str> {
        self.mode.recoil.as_deref()
    }

    /// The `CfgRecoils` class of a prone shot, as data.
    pub fn recoil_prone(&self) -> Option<&str> {
        self.mode.recoil_prone.as_deref()
    }
}

/// Builds and caches [`WeaponType`]s, [`MagazineType`]s and [`AmmoType`]s from the merged config.
/// Class names are matched case-insensitively, as the engine does.
#[derive(Debug)]
pub struct WeaponBank {
    config: Arc<ConfigTree>,
    weapons: HashMap<String, Arc<WeaponType>>,
    magazines: HashMap<String, Arc<MagazineType>>,
    ammos: HashMap<String, Arc<AmmoType>>,
}

impl WeaponBank {
    pub fn new(config: Arc<ConfigTree>) -> Self {
        Self {
            config,
            weapons: HashMap::new(),
            magazines: HashMap::new(),
            ammos: HashMap::new(),
        }
    }

    /// The merged config the parameters are read from.
    pub fn config(&self) -> &ConfigTree {
        &self.config
    }

    /// The weapon of `CfgWeapons` class `name`.
    pub fn weapon(&mut self, name: &str) -> Result<Arc<WeaponType>, Error> {
        let key = name.to_ascii_lowercase();
        if let Some(w) = self.weapons.get(&key) {
            return Ok(w.clone());
        }
        let cfg = self
            .class("CfgWeapons", name)
            .ok_or_else(|| Error::UnknownWeapon(name.to_owned()))?;
        let w = Arc::new(WeaponType {
            name: cfg.name().to_owned(),
            muzzles: self.muzzles(&cfg),
            magazines: text_list(&cfg, "magazines"),
        });
        self.weapons.insert(key, w.clone());
        Ok(w)
    }

    /// The magazine of `CfgMagazines` class `name`.
    pub fn magazine(&mut self, name: &str) -> Result<Arc<MagazineType>, Error> {
        let key = name.to_ascii_lowercase();
        if let Some(m) = self.magazines.get(&key) {
            return Ok(m.clone());
        }
        let cfg = self
            .class("CfgMagazines", name)
            .ok_or_else(|| Error::UnknownMagazine(name.to_owned()))?;
        let count = number(&cfg, "count").unwrap_or(0.0);
        let m = Arc::new(MagazineType {
            name: cfg.name().to_owned(),
            ammo: text(&cfg, "ammo").unwrap_or_default(),
            count: if count > 0.0 { count as u32 } else { 0 },
            init_speed: number(&cfg, "initSpeed").unwrap_or(0.0),
        });
        self.magazines.insert(key, m.clone());
        Ok(m)
    }

    /// The ammunition of `CfgAmmo` class `name`.
    pub fn ammo(&mut self, name: &str) -> Result<Arc<AmmoType>, Error> {
        let key = name.to_ascii_lowercase();
        if let Some(a) = self.ammos.get(&key) {
            return Ok(a.clone());
        }
        let cfg = self
            .class("CfgAmmo", name)
            .ok_or_else(|| Error::UnknownAmmo(name.to_owned()))?;
        let a = Arc::new(self.ammo_from(&cfg));
        self.ammos.insert(key, a.clone());
        Ok(a)
    }

    /// The `CfgAmmo` class `name` without caching; unknown names give the defaults, which is what
    /// an Entity whose type has no `CfgAmmo` class gets.
    pub fn ammo_or_default(&self, name: &str) -> AmmoType {
        match self.class("CfgAmmo", name) {
            Some(cfg) => self.ammo_from(&cfg),
            None => AmmoType::default(),
        }
    }

    fn ammo_from(&self, cfg: &ConfigRef<'_>) -> AmmoType {
        AmmoType {
            name: cfg.name().to_owned(),
            hit: number_or(cfg, "hit", 0.0),
            indirect_hit: number_or(cfg, "indirectHit", 0.0),
            indirect_hit_range: number_or(cfg, "indirectHitRange", 0.0),
            explosive: number_or(cfg, "explosive", 0.0),
            caliber: number_or(cfg, "caliber", 1.0),
            deflecting: number_or(cfg, "deflecting", 0.0).to_radians(),
            deflection_slow_down: number_or(cfg, "deflectionSlowDown", 1.0),
            deflection_dir_distribution: number_or(cfg, "deflectionDirDistribution", 0.0),
            penetration_dir_distribution: number_or(cfg, "penetrationDirDistribution", 0.0),
            air_friction: number_or(cfg, "airFriction", 0.0),
            water_friction: number_or(cfg, "waterFriction", 0.0),
            coef_gravity: number_or(cfg, "coefGravity", 1.0),
            typical_speed: number_or(cfg, "typicalSpeed", 0.0),
            time_to_live: number_or(cfg, "timeToLive", 0.0),
            min_time_to_live: number_or(cfg, "minTimeToLive", 0.0),
            max_speed: number_or(cfg, "maxSpeed", 0.0),
            explosion_time: number_or(cfg, "explosionTime", 0.0),
            fuse_distance: number_or(cfg, "fuseDistance", 0.0),
            simulation: text(cfg, "simulation").and_then(|s| SimulationClass::from_simulation(&s)),
            simulation_step: number(cfg, "simulationStep"),
        }
    }

    /// The weapon's `muzzles[]` as [`MuzzleType`]s. An absent or empty array means the weapon
    /// body is the only muzzle; a name with no class of its own is left out.
    fn muzzles(&self, weapon: &ConfigRef<'_>) -> Vec<MuzzleType> {
        let mut names = text_list(weapon, "muzzles");
        if names.is_empty() {
            names.push(DEFAULT_MUZZLE.to_owned());
        }
        names
            .iter()
            .filter_map(|name| {
                let cfg = self.muzzle_class(weapon, name)?;
                Some(MuzzleType {
                    name: name.clone(),
                    class: cfg.name().to_owned(),
                    magazines: text_list(&cfg, "magazines"),
                    init_speed: number(&cfg, "initSpeed"),
                    modes: self.modes(&cfg),
                })
            })
            .collect()
    }

    /// The class a muzzle's parameters come from: the weapon itself for `"this"`, otherwise its
    /// sub-class.
    fn muzzle_class<'a>(&self, weapon: &ConfigRef<'a>, name: &str) -> Option<ConfigRef<'a>> {
        if name.eq_ignore_ascii_case(DEFAULT_MUZZLE) {
            return Some(weapon.clone());
        }
        let cfg = weapon.get(name);
        cfg.is_class().then_some(cfg)
    }

    /// A muzzle's `modes[]`. A muzzle with none (or with `{"this"}`) is its own `"this"` mode.
    fn modes(&self, muzzle: &ConfigRef<'_>) -> Vec<ModeType> {
        let mut names: Vec<String> = text_list(muzzle, "modes")
            .into_iter()
            .filter(|n| !n.eq_ignore_ascii_case(DEFAULT_MUZZLE))
            .collect();
        if names.is_empty() {
            return vec![mode_from(muzzle, DEFAULT_MUZZLE)];
        }
        let mut modes = Vec::with_capacity(names.len());
        for name in names.drain(..) {
            let cfg = muzzle.get(&name);
            if cfg.is_class() {
                modes.push(mode_from(&cfg, &name));
            }
        }
        modes
    }

    /// The class `name` of the config root's `root` class, e.g. `CfgWeapons >> arifle_MX_F`.
    fn class<'a>(&'a self, root: &str, name: &str) -> Option<ConfigRef<'a>> {
        let cfg = self.config.root().get(root).get(name);
        cfg.is_class().then_some(cfg)
    }
}

fn mode_from(cfg: &ConfigRef<'_>, name: &str) -> ModeType {
    ModeType {
        name: if name.eq_ignore_ascii_case(DEFAULT_MUZZLE) {
            DEFAULT_MUZZLE.to_owned()
        } else {
            cfg.name().to_owned()
        },
        dispersion: number_or(cfg, "dispersion", 0.0),
        recoil: text(cfg, "recoil").filter(|s| !s.is_empty()),
        recoil_prone: text(cfg, "recoilProne").filter(|s| !s.is_empty()),
        init_speed: number(cfg, "initSpeed"),
    }
}

/// `cfg >> name` as a number, `None` when the entry is absent or null.
fn number(cfg: &ConfigRef<'_>, name: &str) -> Option<f64> {
    let v = cfg.get(name);
    (!v.is_null()).then(|| f64::from(v.number()))
}

fn number_or(cfg: &ConfigRef<'_>, name: &str, default: f64) -> f64 {
    number(cfg, name).unwrap_or(default)
}

/// `cfg >> name` as text, `None` when the entry is absent or null.
fn text(cfg: &ConfigRef<'_>, name: &str) -> Option<String> {
    let v = cfg.get(name);
    (!v.is_null()).then(|| v.text())
}

/// `cfg >> name` as a list of strings; numbers are kept as their text form.
fn text_list(cfg: &ConfigRef<'_>, name: &str) -> Vec<String> {
    cfg.get(name)
        .array()
        .iter()
        .filter_map(|v| match v {
            Value::String(s) | Value::Expression(s) => Some(s.clone()),
            Value::Float(f) => Some(f.to_string()),
            Value::Int(i) => Some(i.to_string()),
            Value::Int64(i) => Some(i.to_string()),
            Value::Array(_) => None,
        })
        .collect()
}
