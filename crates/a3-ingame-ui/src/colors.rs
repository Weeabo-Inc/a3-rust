//! The weapon-state colours of the HUD: `RscInGameUI >> colorReady*`, `colorPrepare` and
//! `colorUnload`, read once by `InGameUI::Init` (`FUN_140aa9ec0`), and the rule choosing one
//! for a weapon (`FUN_140aa9b60`, `FUN_140aa9db0`).

use a3_config::ConfigTree;
use a3_ui::{Eval, Rgba, read_color};

/// A unit's side, as the engine numbers it (`TargetSide`: east 0, west 1, resistance 2,
/// civilian 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Side {
    East,
    #[default]
    West,
    Resistance,
    Civilian,
    /// Any other side (logic, enemy, friendly, ...).
    Other,
}

impl Side {
    /// The side of a `CfgVehicles` `side` number.
    pub fn from_config(side: i32) -> Side {
        match side {
            0 => Side::East,
            1 => Side::West,
            2 => Side::Resistance,
            3 => Side::Civilian,
            _ => Side::Other,
        }
    }
}

/// The colours of `RscInGameUI`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IguiColors {
    pub ready: Rgba,
    pub ready_west: Rgba,
    pub ready_east: Rgba,
    pub ready_independent: Rgba,
    pub ready_civilian: Rgba,
    pub prepare: Rgba,
    pub unload: Rgba,
}

impl Default for IguiColors {
    /// The values `RscInGameUI` gives with the default profile colours (`CfgUIColors >> IGUI`
    /// preset `PresetA3`).
    fn default() -> Self {
        let text = [0.95, 0.95, 0.95, 1.0];
        IguiColors {
            ready: text,
            ready_west: text,
            ready_east: text,
            ready_independent: text,
            ready_civilian: text,
            prepare: [0.8, 0.5, 0.0, 1.0],
            unload: [0.8, 0.0, 0.0, 1.0],
        }
    }
}

impl IguiColors {
    /// Reads `RscInGameUI`'s colours, evaluating their profile-variable expressions with
    /// `eval`. Entries that are missing keep [`IguiColors::default`].
    pub fn from_config(config: &ConfigTree, eval: &mut dyn Eval) -> Self {
        let class = config.root().get("RscInGameUI");
        let mut colors = IguiColors::default();
        if !class.is_class() {
            return colors;
        }
        for (name, slot) in [
            ("colorReady", &mut colors.ready),
            ("colorReadyWest", &mut colors.ready_west),
            ("colorReadyEast", &mut colors.ready_east),
            ("colorReadyIndependent", &mut colors.ready_independent),
            ("colorReadyCivilian", &mut colors.ready_civilian),
            ("colorPrepare", &mut colors.prepare),
            ("colorUnload", &mut colors.unload),
        ] {
            if let Some(color) = read_color(&class, name, eval) {
                *slot = color;
            }
        }
        colors
    }

    /// The "ready" colour of a unit of `side` (`FUN_140aa9db0`).
    pub fn ready_for(&self, side: Side) -> Rgba {
        match side {
            Side::West => self.ready_west,
            Side::East => self.ready_east,
            Side::Resistance => self.ready_independent,
            Side::Civilian => self.ready_civilian,
            Side::Other => self.ready,
        }
    }

    /// The colour of a weapon's texts (`FUN_140aa9b60`): ready while its magazine is loaded and
    /// the next shot is due, prepare in the last fifth of the shot cycle, unload while the
    /// magazine reloads, the next shot is further away, or nothing is loaded.
    pub fn weapon(&self, side: Side, magazine: Option<&MagazineTimers>) -> Rgba {
        match magazine {
            Some(m) if m.reload_remaining <= 0.0 => {
                if m.cycle <= 0.0 {
                    self.ready_for(side)
                } else if m.cycle <= 0.2 {
                    self.prepare
                } else {
                    self.unload
                }
            }
            _ => self.unload,
        }
    }
}

/// The timers of the magazine a muzzle has loaded (engine `Magazine` +0x70, +0x78, +0x7c).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MagazineTimers {
    /// The part of the fire mode's `reloadTime` still to run before the next shot, 0..1.
    pub cycle: f32,
    /// Seconds of magazine reload still to run (0 when not reloading).
    pub reload_remaining: f32,
    /// Seconds the running magazine reload takes.
    pub reload_duration: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weapon_colour_follows_the_magazine_timers() {
        let c = IguiColors {
            ready_west: [0.0, 0.0, 1.0, 1.0],
            ..IguiColors::default()
        };
        let ready = MagazineTimers::default();
        assert_eq!(c.weapon(Side::West, Some(&ready)), c.ready_west);
        assert_eq!(c.weapon(Side::East, Some(&ready)), c.ready_east);
        let soon = MagazineTimers {
            cycle: 0.2,
            ..ready
        };
        assert_eq!(c.weapon(Side::West, Some(&soon)), c.prepare);
        let later = MagazineTimers {
            cycle: 0.5,
            ..ready
        };
        assert_eq!(c.weapon(Side::West, Some(&later)), c.unload);
        let reloading = MagazineTimers {
            reload_remaining: 1.0,
            reload_duration: 2.0,
            ..ready
        };
        assert_eq!(c.weapon(Side::West, Some(&reloading)), c.unload);
        assert_eq!(c.weapon(Side::West, None), c.unload);
    }
}
