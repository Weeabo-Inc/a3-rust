//! The HUD state of a unit as its config class spawns it: `weapons[]` and `magazines[]` of
//! `CfgVehicles >> class`, every muzzle loaded with the first compatible magazine carried (the
//! engine's reload of all weapons after creation), the first weapon selected in its first fire
//! mode. For views without a simulated World (`arma3 --play`); a running World builds its
//! [`UnitInfo`] from the unit's actual weapons instead.

use a3_config::{ConfigRef, ConfigTree, Value};

use crate::colors::{MagazineTimers, Side};
use crate::unit_info::{LoadedMagazine, ThrowableState, UnitInfo, WeaponState};

/// The text entries of an array entry (others skipped).
fn strings(class: &ConfigRef<'_>, name: &str) -> Vec<String> {
    let e = class.get(name);
    if !e.is_array() {
        return Vec::new();
    }
    e.array()
        .into_iter()
        .filter_map(|v| match v {
            Value::String(s) => Some(s),
            _ => None,
        })
        .collect()
}

/// A text entry, `$STR_` keys localized.
fn text(class: &ConfigRef<'_>, name: &str, localize: &dyn Fn(&str) -> Option<String>) -> String {
    let e = class.get(name);
    if !e.is_text() {
        return String::new();
    }
    let t = e.text();
    match t.strip_prefix('$') {
        Some(key) if key.len() > 4 && key[..4].eq_ignore_ascii_case("STR_") => {
            localize(key).unwrap_or(t)
        }
        _ => t,
    }
}

/// A magazine carried, with the rounds left.
#[derive(Debug, Clone)]
struct Carried {
    class: String,
    ammo: u32,
}

/// The magazines a muzzle takes: `magazines[]` and every list of its `magazineWell[]` wells.
fn compatible(config: &ConfigTree, muzzle: &ConfigRef<'_>) -> Vec<String> {
    let mut out = strings(muzzle, "magazines");
    let wells = config.root().get("CfgMagazineWells");
    for well in strings(muzzle, "magazineWell") {
        let class = wells.get(&well);
        if !class.is_class() {
            continue;
        }
        for entry in class.entries_with_inherited() {
            if entry.is_array() {
                out.extend(strings(&class, entry.name()));
            }
        }
    }
    out
}

fn magazine_count(config: &ConfigTree, magazine: &str) -> u32 {
    let e = config.root().get("CfgMagazines").get(magazine).get("count");
    if e.is_number() {
        e.number().max(0.0) as u32
    } else {
        0
    }
}

/// The muzzle class `name` of `weapon` (`"this"` is the weapon itself).
fn muzzle<'a>(weapon: &ConfigRef<'a>, name: &str) -> ConfigRef<'a> {
    if name.eq_ignore_ascii_case("this") {
        weapon.clone()
    } else {
        weapon.get(name)
    }
}

/// Takes the first carried magazine `muzzle` accepts out of `inventory`.
fn load(
    config: &ConfigTree,
    muzzle: &ConfigRef<'_>,
    inventory: &mut Vec<Carried>,
) -> Option<Carried> {
    let accepted = compatible(config, muzzle);
    let i = inventory
        .iter()
        .position(|m| accepted.iter().any(|a| a.eq_ignore_ascii_case(&m.class)))?;
    Some(inventory.remove(i))
}

/// The [`UnitInfo`] of a freshly spawned unit of `CfgVehicles >> class`, alive, on foot and
/// standing. `localize` turns `STR_` keys into text.
pub fn spawned_unit_info(
    config: &ConfigTree,
    class: &str,
    localize: &dyn Fn(&str) -> Option<String>,
) -> UnitInfo {
    let unit = config.root().get("CfgVehicles").get(class);
    let side = unit.get("side");
    let side = if side.is_number() {
        Side::from_config(side.number() as i32)
    } else {
        Side::Other
    };
    let unit_info_types = {
        let e = unit.get("unitInfoType");
        if e.is_array() {
            strings(&unit, "unitInfoType")
        } else if e.is_text() {
            vec![e.text()]
        } else {
            Vec::new()
        }
    };
    let mut inventory: Vec<Carried> = strings(&unit, "magazines")
        .into_iter()
        .map(|class| Carried {
            ammo: magazine_count(config, &class),
            class,
        })
        .collect();
    let weapons_cfg = config.root().get("CfgWeapons");
    let magazines_cfg = config.root().get("CfgMagazines");

    // Every muzzle of every weapon takes its magazine first, in weapon and muzzle order.
    let mut loaded: Vec<(String, String, Option<Carried>)> = Vec::new();
    for weapon in strings(&unit, "weapons") {
        let w = weapons_cfg.get(&weapon);
        if !w.is_class() {
            continue;
        }
        let mut muzzles = strings(&w, "muzzles");
        if muzzles.is_empty() {
            muzzles.push("this".to_owned());
        }
        for m in muzzles {
            let mc = muzzle(&w, &m);
            let magazine = load(config, &mc, &mut inventory);
            loaded.push((weapon.clone(), m, magazine));
        }
    }

    let short_name = |magazine: &str| {
        let class = magazines_cfg.get(magazine);
        text(&class, "displayNameShort", localize)
    };

    // The selected weapon: the first muzzle of the first weapon that is not thrown or put.
    let weapon = loaded
        .iter()
        .find(|(w, _, _)| !w.eq_ignore_ascii_case("Throw") && !w.eq_ignore_ascii_case("Put"))
        .map(|(weapon, m, magazine)| {
            let w = weapons_cfg.get(weapon);
            let mc = muzzle(&w, m);
            let modes = strings(&mc, "modes");
            let mode = match modes.first() {
                Some(name) if !name.eq_ignore_ascii_case("this") => mc.get(name),
                _ => mc.clone(),
            };
            let accepted = compatible(config, &mc);
            let spare: Vec<&Carried> = inventory
                .iter()
                .filter(|c| c.ammo > 0)
                .filter(|c| accepted.iter().any(|a| a.eq_ignore_ascii_case(&c.class)))
                .collect();
            let shown = magazine
                .as_ref()
                .map(|m| m.class.clone())
                .or_else(|| spare.first().map(|c| c.class.clone()))
                .unwrap_or_default();
            let capacity = magazine_count(config, &shown);
            let reload = mode.get("reloadTime");
            WeaponState {
                display_name: text(&w, "displayName", localize),
                mode_name: text(&mode, "displayName", localize),
                mode_texture_type: mode.get("textureType").text(),
                reload_time: if reload.is_number() {
                    reload.number()
                } else {
                    0.0
                },
                magazine_name: if shown.is_empty() {
                    String::new()
                } else {
                    short_name(&shown)
                },
                loaded: magazine.as_ref().map(|m| LoadedMagazine {
                    ammo: m.ammo,
                    timers: MagazineTimers::default(),
                }),
                magazines: spare.len() as u32,
                capacity,
                total_capacity: magazine
                    .iter()
                    .chain(spare.iter().copied())
                    .map(|m| magazine_count(config, &m.class))
                    .sum(),
            }
        });

    // The selected throwable: the first `Throw` muzzle holding one (_uncertain: the engine's
    // initial choice_), counted with the ones still carried.
    let throwable = loaded
        .iter()
        .filter(|(w, _, _)| w.eq_ignore_ascii_case("Throw"))
        .find_map(|(_, _, magazine)| magazine.as_ref())
        .map(|m| ThrowableState {
            magazine_name: short_name(&m.class),
            count: 1 + inventory
                .iter()
                .filter(|c| c.class.eq_ignore_ascii_case(&m.class))
                .count() as u32,
            loaded: Some(MagazineTimers::default()),
        });

    UnitInfo {
        alive: true,
        side,
        unit_info_types,
        on_foot: true,
        weapon,
        throwable,
        ..UnitInfo::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_config::parse_text;

    const CONFIG: &str = r#"
        class CfgMagazines {
            class Mag30 { count = 30; displayNameShort = "$STR_Short"; };
            class Mag20 { count = 20; displayNameShort = "20rnd"; };
            class Frag { count = 1; displayNameShort = "Frag"; };
            class Smoke { count = 1; displayNameShort = "Smoke"; };
        };
        class CfgMagazineWells { class Well { BI[] = {"Mag20"}; }; };
        class CfgWeapons {
            class Mode_SemiAuto { displayName = "Semi"; textureType = "semi"; reloadTime = 0.1; };
            class Rifle {
                displayName = "Rifle 6.5";
                magazines[] = {"Mag30"};
                magazineWell[] = {"Well"};
                modes[] = {"Single"};
                class Single: Mode_SemiAuto {};
            };
            class Throw {
                muzzles[] = {"Stone", "FragMuzzle", "SmokeMuzzle"};
                class Stone { magazines[] = {"Stone"}; };
                class FragMuzzle { magazines[] = {"Frag"}; };
                class SmokeMuzzle { magazines[] = {"Smoke"}; };
            };
        };
        class CfgVehicles {
            class Soldier {
                side = 1;
                unitInfoType = "RscUnitInfoSoldier";
                weapons[] = {"Rifle", "Throw"};
                magazines[] = {"Mag30", "Mag30", "Mag20", "Frag", "Smoke", "Frag", "Frag"};
            };
        };
    "#;

    #[test]
    fn a_spawned_unit_has_loaded_weapons() {
        let config = ConfigTree::from_config(&parse_text(CONFIG).unwrap());
        let localize = |key: &str| (key == "STR_Short").then(|| "6.5mm".to_owned());
        let info = spawned_unit_info(&config, "Soldier", &localize);
        assert!(info.alive && info.on_foot);
        assert_eq!(info.side, Side::West);
        assert_eq!(info.unit_info_types, ["RscUnitInfoSoldier"]);
        let w = info.weapon.expect("rifle selected");
        assert_eq!(w.display_name, "Rifle 6.5");
        assert_eq!(
            (w.mode_name.as_str(), w.mode_texture_type.as_str()),
            ("Semi", "semi")
        );
        assert_eq!(w.reload_time, 0.1);
        assert_eq!(w.magazine_name, "6.5mm");
        assert_eq!(w.loaded.map(|m| m.ammo), Some(30));
        assert_eq!(w.magazines, 2, "the second Mag30 and the well's Mag20");
        assert_eq!(w.capacity, 30);
        assert_eq!(w.total_capacity, 80);
        let t = info.throwable.expect("a grenade in hand");
        assert_eq!((t.magazine_name.as_str(), t.count), ("Frag", 3));
    }
}
