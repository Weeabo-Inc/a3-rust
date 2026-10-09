//! `CfgWeapons` / `CfgMagazines` / `CfgAmmo` parameters, from a synthetic config.
//!
//! Vocabulary and sources: `docs/re/sim-ballistics.md` §1 (ammo), `docs/re/sim-weapons.md` §1
//! (the loaders) and §2.3 (`initSpeed`), the offline Arma wiki (`muzzles[]`, `modes[]`,
//! `recoil`).

use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_world::Error;
use a3_world::SimulationClass;
use a3_world::weapons::{AmmoType, WeaponBank, WeaponType};

const CONFIG: &str = r#"
class Mode_SemiAuto {
    dispersion = 0.001;
    recoil = "recoil_semi";
    recoilProne = "recoil_semi_prone";
    initSpeed = 0;
};
class CfgAmmo {
    class Default { simulation = ""; };
    class BulletCore: Default { simulation = "shotBullet"; simulationStep = 0.001; };
    class B_65x39_Ball: BulletCore {
        hit = 8;
        indirectHit = 0;
        indirectHitRange = 0;
        caliber = 0.9;
        deflecting = 15;
        deflectionSlowDown = 1;
        airFriction = -0.0012;
        coefGravity = 1;
        typicalSpeed = 800;
        timeToLive = 6;
    };
    class G_20mm_HE: BulletCore {
        hit = 30;
        indirectHit = 12;
        indirectHitRange = 2.5;
        explosive = 0.6;
        caliber = 2.5;
        deflecting = 0;
        deflectionSlowDown = 1;
        airFriction = -0.0005;
        typicalSpeed = 900;
        timeToLive = 20;
        simulation = "shotShell";
        explosionTime = 0.5;
        fuseDistance = 10;
        penetrationDirDistribution = 0.25;
        deflectionDirDistribution = 0.5;
    };
};
class CfgMagazines {
    class 30Rnd_65x39_Mag {
        ammo = "B_65x39_Ball";
        count = 30;
        initSpeed = 800;
    };
    class 1Rnd_20mm_HE_Mag {
        ammo = "G_20mm_HE";
        count = 1;
        initSpeed = 1000;
    };
};
class CfgWeapons {
    class arifle_MX_F {
        muzzles[] = { "this" };
        magazines[] = { "30Rnd_65x39_Mag" };
        modes[] = { "Single" };
        class Single: Mode_SemiAuto {
            dispersion = 0.0007;
            recoil = "recoil_arifle_MX";
        };
    };
    class launcher_20mm_F {
        muzzles[] = { "this", "EGLM" };
        magazines[] = { "1Rnd_20mm_HE_Mag" };
        initSpeed = -1.1;
        class EGLM {
            magazines[] = { "1Rnd_20mm_HE_Mag" };
            initSpeed = 78;
            dispersion = 0.005;
            recoil = "recoil_eglm";
            modes[] = { "EGLM_Single" };
            class EGLM_Single: Mode_SemiAuto {
                initSpeed = 250;
            };
        };
    };
    class mode_speed_F {
        magazines[] = { "30Rnd_65x39_Mag" };
        modes[] = { "Single" };
        class Single: Mode_SemiAuto {
            initSpeed = 700;
        };
    };
    class simple_F {
        magazines[] = { "30Rnd_65x39_Mag" };
        dispersion = 0.002;
        initSpeed = 950;
    };
    class rifle_neg_F {
        magazines[] = { "30Rnd_65x39_Mag" };
        initSpeed = -1.1;
    };
};
"#;

/// The `f64` a config number literal reads as: config numbers are stored as `f32`, and the types
/// report them as `f64`, so an expected value written as a plain `f64` literal would not be the
/// value the config holds.
fn f32v(v: f32) -> f64 {
    f64::from(v)
}

fn bank() -> WeaponBank {
    let config = parse_text(CONFIG).unwrap();
    WeaponBank::new(Arc::new(ConfigTree::from_config(&config)))
}

fn weapon(bank: &mut WeaponBank, name: &str) -> Arc<WeaponType> {
    bank.weapon(name).unwrap()
}

#[test]
fn the_weapon_body_is_the_default_muzzle() {
    let mut bank = bank();

    let w = weapon(&mut bank, "arifle_MX_F");

    assert_eq!(w.name, "arifle_MX_F");
    let muzzle = w.muzzle(None).expect("the default muzzle");
    assert_eq!(muzzle.name, "this");
    assert_eq!(muzzle.class, "arifle_MX_F");
    assert_eq!(muzzle.magazines, ["30Rnd_65x39_Mag"]);
    // The body's modes[] are the muzzle's modes.
    let mode = muzzle.mode(None).expect("the default mode");
    assert_eq!(mode.name, "Single");
    assert_eq!(mode.dispersion, f32v(0.0007));
    assert_eq!(mode.recoil.as_deref(), Some("recoil_arifle_MX"));
    // Inherited from the mode base class.
    assert_eq!(mode.recoil_prone.as_deref(), Some("recoil_semi_prone"));
}

#[test]
fn a_weapon_with_no_muzzles_or_modes_is_its_own_muzzle_and_mode() {
    let mut bank = bank();

    let w = weapon(&mut bank, "simple_F");

    let muzzle = w.muzzle(None).expect("the default muzzle");
    assert_eq!(muzzle.name, "this");
    assert_eq!(muzzle.class, "simple_F");
    let mode = muzzle.mode(None).expect("the default mode");
    assert_eq!(mode.name, "this");
    assert_eq!(mode.dispersion, f32v(0.002));
    assert!((muzzle.init_speed.expect("a declared initSpeed") - 950.0).abs() < 1e-9);
}

#[test]
fn a_named_muzzle_is_a_sub_class_with_its_own_parameters() {
    let mut bank = bank();

    let w = weapon(&mut bank, "launcher_20mm_F");

    let names: Vec<&str> = w.muzzles.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["this", "EGLM"]);
    let eglm = w.muzzle(Some("EGLM")).expect("the EGLM muzzle");
    assert_eq!(eglm.class, "EGLM");
    assert_eq!(eglm.magazines, ["1Rnd_20mm_HE_Mag"]);
    assert!((eglm.init_speed.expect("the EGLM initSpeed") - 78.0).abs() < 1e-9);
    // The EGLM declares its own mode, which inherits `dispersion` from Mode_SemiAuto here.
    let mode = eglm.mode(None).expect("the EGLM's first mode");
    assert_eq!(mode.name, "EGLM_Single");
    assert_eq!(mode.dispersion, f32v(0.001));
    // Unknown muzzle names are None, not the default muzzle.
    assert!(w.muzzle(Some("nope")).is_none());
}

#[test]
fn a_muzzle_with_no_modes_uses_itself_as_the_mode() {
    let mut bank = bank();

    // The EGLM of the launcher declares modes, so use a weapon whose only muzzle has none.
    let w = weapon(&mut bank, "simple_F");

    let mode = w.muzzle(None).and_then(|m| m.mode(None)).unwrap();

    assert_eq!(mode.name, "this");
    assert_eq!(mode.recoil.as_deref(), None);
}

#[test]
fn a_positive_init_speed_overrides_the_magazine() {
    let mut bank = bank();
    let w = weapon(&mut bank, "simple_F");
    let mag = bank.magazine("30Rnd_65x39_Mag").unwrap();

    let shot = w.shot_params(None, None).unwrap();
    let ammo = bank.ammo(&mag.ammo).unwrap();

    assert!((shot.init_speed(&mag, &ammo) - 950.0).abs() < 1e-9);
}

#[test]
fn a_negative_init_speed_multiplies_the_magazine() {
    let mut bank = bank();
    let w = weapon(&mut bank, "rifle_neg_F");
    let mag = bank.magazine("30Rnd_65x39_Mag").unwrap();
    let ammo = bank.ammo(&mag.ammo).unwrap();

    // initSpeed = -1.1 → 1.1 × the magazine's 800.
    let shot = w.shot_params(None, None).unwrap();

    assert_eq!(shot.init_speed(&mag, &ammo), f32v(1.1) * 800.0);
}

#[test]
fn shells_fly_at_the_magazine_speed_whatever_the_weapon_says() {
    // `0x140faf810` reads the weapon's initSpeed only for shotBullet / shotSpread ammo
    // (`sim-weapons.md` §2.3): the HE shell keeps its magazine's 1000 under the weapon's -1.1.
    let mut bank = bank();
    let w = weapon(&mut bank, "launcher_20mm_F");
    let mag = bank.magazine("1Rnd_20mm_HE_Mag").unwrap();
    let ammo = bank.ammo(&mag.ammo).unwrap();

    let shot = w.shot_params(None, None).unwrap();

    assert_eq!(shot.init_speed(&mag, &ammo), 1000.0);
}

#[test]
fn init_speed_zero_takes_the_magazine_value() {
    let mut bank = bank();
    let w = weapon(&mut bank, "arifle_MX_F");
    let mag = bank.magazine("30Rnd_65x39_Mag").unwrap();

    // The weapon declares none: the engine's default -1 multiplies the magazine's by 1.
    let shot = w.shot_params(None, None).unwrap();
    let ammo = bank.ammo(&mag.ammo).unwrap();

    assert_eq!(mag.init_speed, 800.0);
    assert!((shot.init_speed(&mag, &ammo) - 800.0).abs() < 1e-9);
}

#[test]
fn a_named_muzzles_init_speed_is_not_read() {
    // Muzzles have no initSpeed of their own in the engine (`sim-weapons.md` §1.2, §2.3): the
    // EGLM's 78 and its mode's 250 are ignored, its shell flies at the magazine's 1000.
    let mut bank = bank();
    let w = weapon(&mut bank, "launcher_20mm_F");
    let mag = bank.magazine("1Rnd_20mm_HE_Mag").unwrap();
    let ammo = bank.ammo(&mag.ammo).unwrap();

    let shot = w.shot_params(Some("EGLM"), None).unwrap();

    assert_eq!(shot.init_speed(&mag, &ammo), 1000.0);
}

#[test]
fn a_modes_init_speed_is_not_read() {
    // `WeaponModeType` has no initSpeed (`sim-weapons.md` §1.1): mode_speed_F's mode value 700 is
    // ignored, the weapon's default -1 keeps the magazine's 800.
    let mut bank = bank();
    let w = weapon(&mut bank, "mode_speed_F");
    let mag = bank.magazine("30Rnd_65x39_Mag").unwrap();
    let ammo = bank.ammo(&mag.ammo).unwrap();

    let shot = w.shot_params(None, None).unwrap();

    assert_eq!(shot.init_speed(&mag, &ammo), 800.0);
}

#[test]
fn ammo_parameters_come_from_cfg_ammo() {
    let mut bank = bank();

    let ammo: Arc<AmmoType> = bank.ammo("B_65x39_Ball").unwrap();

    assert_eq!(ammo.name, "B_65x39_Ball");
    assert!((ammo.hit - 8.0).abs() < 1e-9);
    assert!((ammo.indirect_hit - 0.0).abs() < 1e-9);
    assert!((ammo.indirect_hit_range - 0.0).abs() < 1e-9);
    assert!((ammo.explosive - 0.0).abs() < 1e-9);
    assert_eq!(ammo.caliber, f32v(0.9));
    assert!((ammo.deflection_slow_down - 1.0).abs() < 1e-9);
    assert_eq!(ammo.air_friction, f32v(-0.0012));
    assert!((ammo.coef_gravity - 1.0).abs() < 1e-9);
    assert!((ammo.typical_speed - 800.0).abs() < 1e-9);
    assert!((ammo.time_to_live - 6.0).abs() < 1e-9);
    assert!((ammo.explosion_time - 0.0).abs() < 1e-9);
    assert!((ammo.fuse_distance - 0.0).abs() < 1e-9);
    assert_eq!(ammo.simulation, Some(SimulationClass::ShotBullet));
    assert_eq!(ammo.simulation_step, Some(f32v(0.001)));
}

#[test]
fn deflecting_is_degrees_in_config_and_radians_in_the_type() {
    let mut bank = bank();

    let ball = bank.ammo("B_65x39_Ball").unwrap();
    let he = bank.ammo("G_20mm_HE").unwrap();

    assert!((ball.deflecting - 15_f64.to_radians()).abs() < 1e-12);
    assert!((he.deflecting - 0.0).abs() < 1e-12);
}

#[test]
fn an_explosive_ammo_keeps_its_explosion_parameters() {
    let mut bank = bank();

    let he = bank.ammo("G_20mm_HE").unwrap();

    assert!((he.hit - 30.0).abs() < 1e-9);
    assert!((he.indirect_hit - 12.0).abs() < 1e-9);
    assert!((he.indirect_hit_range - 2.5).abs() < 1e-9);
    assert_eq!(he.explosive, f32v(0.6));
    assert!((he.caliber - 2.5).abs() < 1e-9);
    assert!((he.explosion_time - 0.5).abs() < 1e-9);
    assert!((he.fuse_distance - 10.0).abs() < 1e-9);
    assert_eq!(he.penetration_dir_distribution, f32v(0.25));
    assert_eq!(he.deflection_dir_distribution, f32v(0.5));
    assert_eq!(he.simulation, Some(SimulationClass::ShotShell));
}

#[test]
fn a_magazine_names_its_ammo_and_carries_the_base_muzzle_velocity() {
    let mut bank = bank();

    let mag = bank.magazine("1Rnd_20mm_HE_Mag").unwrap();

    assert_eq!(mag.name, "1Rnd_20mm_HE_Mag");
    assert_eq!(mag.ammo, "G_20mm_HE");
    assert_eq!(mag.count, 1);
    assert!((mag.init_speed - 1000.0).abs() < 1e-9);
}

#[test]
fn lookups_are_case_insensitive_and_cached() {
    let mut bank = bank();

    let a = bank.weapon("ARIFLE_mx_f").unwrap();
    let b = bank.weapon("arifle_MX_F").unwrap();

    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(a.name, "arifle_MX_F");
}

#[test]
fn unknown_names_are_errors() {
    let mut bank = bank();

    assert_eq!(
        bank.weapon("nope").unwrap_err(),
        Error::UnknownWeapon("nope".to_owned())
    );
    assert_eq!(
        bank.magazine("nope").unwrap_err(),
        Error::UnknownMagazine("nope".to_owned())
    );
    assert_eq!(
        bank.ammo("nope").unwrap_err(),
        Error::UnknownAmmo("nope".to_owned())
    );
}

/// The shipped `CfgWeapons` / `CfgMagazines` / `CfgAmmo` all load and every weapon/magazine pair
/// resolves to a positive muzzle velocity (`sim-weapons.md` §2.3). Skipped without `A3_ROOT`.
#[test]
fn the_shipped_weapons_config_loads() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = a3_gamedata::GameData::load(&a3_gamedata::LoadOptions::new(root)).unwrap();
    let mut bank = WeaponBank::new(data.config.clone());

    let classes = |root_name: &str| -> Vec<String> {
        data.config
            .root()
            .get(root_name)
            .entries()
            .into_iter()
            .filter(|c| c.is_class())
            .map(|c| c.name().to_owned())
            .collect()
    };

    let mut magazines = 0;
    for name in classes("CfgMagazines") {
        let Ok(mag) = bank.magazine(&name) else {
            continue;
        };
        // `Default` and other abstract bases name no ammo; the engine errors only when one is
        // loaded into a muzzle.
        if mag.ammo.is_empty() {
            continue;
        }
        magazines += 1;
        assert!(
            bank.ammo(&mag.ammo).is_ok(),
            "{}: no ammo class {}",
            mag.name,
            mag.ammo
        );
    }

    let mut weapons = 0;
    let mut pairs = 0;
    let mut bullets = 0;
    let mut flying = 0;
    for name in classes("CfgWeapons") {
        let Ok(weapon) = bank.weapon(&name) else {
            continue;
        };
        let Some(params) = weapon.shot_params(None, None) else {
            continue;
        };
        weapons += 1;
        for magazine in &weapon.magazines {
            let Ok(mag) = bank.magazine(magazine) else {
                continue;
            };
            let Ok(ammo) = bank.ammo(&mag.ammo) else {
                continue;
            };
            let speed = params.init_speed(&mag, &ammo);
            assert!(
                speed >= 0.0 && speed.is_finite(),
                "{} with {}: initSpeed {speed}",
                weapon.name,
                magazine
            );
            // §2.3: only a bullet reads the weapon's own `initSpeed`; everything else flies at
            // its magazine's.
            if !matches!(
                ammo.simulation,
                Some(SimulationClass::ShotBullet | SimulationClass::ShotSpread)
            ) {
                assert_eq!(speed, mag.init_speed, "{} with {}", weapon.name, magazine);
            } else {
                bullets += 1;
            }
            if speed > 0.0 {
                flying += 1;
            }
            pairs += 1;
        }
    }
    eprintln!(
        "{magazines} magazines, {weapons} weapons, {pairs} pairs ({bullets} bullet, {flying} flying)"
    );
    assert!(magazines > 500, "{magazines} magazines");
    assert!(weapons > 500, "{weapons} weapons");
    assert!(pairs > 1000, "{pairs} weapon/magazine pairs");
    assert!(bullets > 500, "{bullets} bullet pairs");
    // Missiles fly on `thrust`, not on an initial speed, so a few pairs are 0.
    assert!(flying > pairs * 9 / 10, "{flying} of {pairs} fly");

    // A shipped example of both branches of §2.3: `arifle_MX_F` declares its own 800, and the
    // 570x28 SMG's -1.1 multiplies its magazine's 715 by 1.1.
    let weapon = bank.weapon("arifle_MX_F").expect("the MX");
    let params = weapon.shot_params(None, None).expect("a muzzle and mode");
    assert_eq!(weapon.init_speed, 800.0);
    let mag = bank.magazine("30Rnd_65x39_caseless_mag").expect("its magazine");
    assert_eq!(mag.ammo, "B_65x39_Caseless");
    assert_eq!(params.init_speed(&mag, &bank.ammo(&mag.ammo).unwrap()), 800.0);

    let smg = bank.weapon("SMG_03_TR_BASE").expect("the 570x28 SMG");
    let params = smg.shot_params(None, None).expect("a muzzle and mode");
    assert_eq!(smg.init_speed, f32v(-1.1));
    let mag = bank.magazine("50Rnd_570x28_SMG_03").expect("its magazine");
    assert_eq!(mag.init_speed, 715.0);
    assert_eq!(
        params.init_speed(&mag, &bank.ammo(&mag.ammo).unwrap()),
        f32v(1.1) * 715.0
    );
}
