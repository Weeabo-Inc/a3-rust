//! The unit info displays (`RscInGameUI >> RscUnitInfo*`, IDD 300): what the engine writes into
//! their controls each frame (`FUN_1411859e0`, the weapon part), keyed by IDC.
//!
//! The player's state arrives as a [`UnitInfo`] snapshot; [`fill`] shows, hides, colours and
//! sets the text of every engine-driven control of one display. Controls the engine does not
//! drive keep what their config gave them.

use std::collections::HashMap;

use a3_config::ConfigTree;
use a3_ui::{ControlId, DisplayId, Rgba, Ui};

use crate::colors::{IguiColors, MagazineTimers, Side};
use crate::format::sprintf;
use crate::idc;
use crate::stance::{Stance, StanceAdjust, StanceState};

/// What the HUD shows about the player, gathered by the game each frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UnitInfo {
    /// Whether the player's unit is alive (texts turn `colorUnload` when not).
    pub alive: bool,
    pub side: Side,
    /// `unitInfoType` of the vehicle the player is in, or of his own class on foot: the
    /// `RscInGameUI` classes to show (a config string is a one-element list).
    pub unit_info_types: Vec<String>,
    /// Whether he is on foot: the stance indicator only shows then.
    pub on_foot: bool,
    /// The selected weapon (muzzle and fire mode).
    pub weapon: Option<WeaponState>,
    /// The selected throwable (grenade muzzle of `Throw`).
    pub throwable: Option<ThrowableState>,
    /// Set while he falls freely (the freefall controls show then).
    pub freefall: Option<Freefall>,
    pub stance: Stance,
    pub stance_state: StanceState,
    pub stance_adjust: StanceAdjust,
}

/// The selected weapon, muzzle and fire mode.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WeaponState {
    /// `CfgWeapons` `displayName` of the weapon, localized.
    pub display_name: String,
    /// The fire mode's `displayName`, localized.
    pub mode_name: String,
    /// The fire mode's `textureType` (`semi`, `fullAuto`, ...).
    pub mode_texture_type: String,
    /// The fire mode's `reloadTime`: seconds between shots.
    pub reload_time: f32,
    /// `displayNameShort` of the loaded magazine, or of the first compatible one carried.
    pub magazine_name: String,
    /// The loaded magazine, if any.
    pub loaded: Option<LoadedMagazine>,
    /// Further compatible magazines with ammo carried.
    pub magazines: u32,
    /// `count` of the loaded (or first compatible) magazine type; 0 for a weapon without
    /// magazines.
    pub capacity: u32,
    /// The summed `count` of the loaded and the further magazines.
    pub total_capacity: u32,
}

/// A loaded magazine.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LoadedMagazine {
    /// Rounds left (summed over the magazines sharing one ammo pool, as the engine does).
    pub ammo: u32,
    pub timers: MagazineTimers,
}

/// The selected throwable.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ThrowableState {
    /// `displayNameShort` of the throwable's magazine.
    pub magazine_name: String,
    /// Throwables of that type carried, the one in hand included.
    pub count: u32,
    /// The magazine in hand, for the colour.
    pub loaded: Option<MagazineTimers>,
}

/// Freefall readouts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Freefall {
    /// Speed in km/h.
    pub speed: f32,
    /// Vertical speed.
    pub vertical_speed: f32,
    /// Altitude in metres.
    pub altitude: f32,
}

/// The localized formats the unit info uses.
#[derive(Debug, Clone, PartialEq)]
pub struct Formats {
    /// `STR_UI_AMMO`: rounds and magazines.
    pub ammo: String,
    /// `STR_UI_AMMO_EMPTY`: rounds without further magazines.
    pub ammo_empty: String,
    pub speed_freefall: String,
    pub speed_vertical_freefall: String,
    pub alt_freefall: String,
}

impl Default for Formats {
    /// The English texts.
    fn default() -> Self {
        Formats {
            ammo: "%d | %d".to_owned(),
            ammo_empty: "%d".to_owned(),
            speed_freefall: "SPD %.0fkmph".to_owned(),
            speed_vertical_freefall: "SPD VERT %.0f".to_owned(),
            alt_freefall: "ALT %.0fm".to_owned(),
        }
    }
}

impl Formats {
    /// The formats from the stringtables through `localize` (`STR_...` key to text); unknown
    /// keys keep the English text.
    pub fn localized(localize: impl Fn(&str) -> Option<String>) -> Self {
        let mut f = Formats::default();
        for (key, slot) in [
            ("STR_UI_AMMO", &mut f.ammo),
            ("STR_UI_AMMO_EMPTY", &mut f.ammo_empty),
            ("STR_UI_SPEED_FREEFALL", &mut f.speed_freefall),
            (
                "STR_UI_SPEED_VERTICAL_FREEFALL",
                &mut f.speed_vertical_freefall,
            ),
            ("STR_UI_ALT_FREEFALL", &mut f.alt_freefall),
        ] {
            if let Some(text) = localize(key) {
                *slot = text;
            }
        }
        f
    }
}

/// `CfgInGameUI >> CfgWeaponModeTextures`: fire mode texture by `textureType`, with the
/// `default` entry for unknown types. Keys compare case-sensitively, as the engine's map does.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModeTextures {
    textures: HashMap<String, String>,
}

impl ModeTextures {
    pub fn from_config(config: &ConfigTree) -> Self {
        let class = config
            .root()
            .get("CfgInGameUI")
            .get("CfgWeaponModeTextures");
        let textures = class
            .entries()
            .into_iter()
            .filter(|e| e.is_text())
            .map(|e| (e.name().to_owned(), e.text()))
            .collect();
        ModeTextures { textures }
    }

    /// The texture of `texture_type`, else of `default`.
    pub fn get(&self, texture_type: &str) -> Option<&str> {
        self.textures
            .get(texture_type)
            .or_else(|| self.textures.get("default"))
            .map(String::as_str)
    }
}

/// Everything [`fill`] needs besides the snapshot.
#[derive(Debug, Clone, Default)]
pub struct FillContext {
    pub colors: IguiColors,
    pub mode_textures: ModeTextures,
    pub formats: Formats,
}

/// The control the engine drives for `idc` in `display`: the last one created with that IDC
/// (each creation overwrites the engine's pointer for it).
pub fn driven_control(ui: &Ui, display: DisplayId, idc: i32) -> Option<ControlId> {
    let d = ui.display(display)?;
    let mut stack: Vec<ControlId> = d
        .background
        .iter()
        .chain(&d.controls)
        .chain(&d.objects)
        .copied()
        .collect();
    let mut found = None;
    while let Some(id) = stack.pop() {
        let Some(c) = ui.control(id) else { continue };
        if c.idc == idc && found.is_none_or(|f: ControlId| id > f) {
            found = Some(id);
        }
        stack.extend(c.children.iter().copied());
    }
    found
}

/// One display's engine-driven controls, looked up once per fill.
struct Controls<'a> {
    ui: &'a mut Ui,
    display: DisplayId,
}

impl Controls<'_> {
    fn with(&mut self, idc: i32, f: impl FnOnce(&mut a3_ui::Control)) -> bool {
        let Some(id) = driven_control(self.ui, self.display, idc) else {
            return false;
        };
        if let Some(c) = self.ui.control_mut(id) {
            f(c);
        }
        true
    }

    fn show(&mut self, idc: i32, show: bool) {
        self.with(idc, |c| c.show = show);
    }

    fn text(&mut self, idc: i32, text: String) {
        self.with(idc, |c| c.text = text);
    }

    fn color(&mut self, idc: i32, color: Rgba) {
        self.with(idc, |c| c.color_text = color);
    }

    fn shown(&self, idc: i32) -> bool {
        driven_control(self.ui, self.display, idc)
            .and_then(|id| self.ui.control(id))
            .is_some_and(|c| c.show)
    }

    /// Sets a progress bar's range and position (the engine clamps the position into the
    /// range).
    fn progress(&mut self, idc: i32, value: f32, max: f32) {
        self.with(idc, |c| {
            c.range = [0.0, max];
            c.value = value.clamp(0.0, max.max(0.0));
        });
    }
}

/// Fills the engine-driven controls of the unit info `display` from `info`.
pub fn fill(ui: &mut Ui, display: DisplayId, info: &UnitInfo, ctx: &FillContext) {
    let mut c = Controls { ui, display };
    // Freefall readouts (380-382) only while falling freely.
    for idc in [
        idc::SPEED_FREEFALL,
        idc::SPEED_VERTICAL_FREEFALL,
        idc::ALT_FREEFALL,
    ] {
        c.show(idc, info.freefall.is_some());
    }
    if let Some(f) = &info.freefall {
        let speed = f.speed.round() as f64;
        let vertical = f.vertical_speed.round() as f64;
        c.text(
            idc::SPEED_FREEFALL,
            sprintf(&ctx.formats.speed_freefall, &[speed.into()]),
        );
        c.text(
            idc::SPEED_VERTICAL_FREEFALL,
            sprintf(&ctx.formats.speed_vertical_freefall, &[vertical.into()]),
        );
        c.text(
            idc::ALT_FREEFALL,
            sprintf(&ctx.formats.alt_freefall, &[f64::from(f.altitude).into()]),
        );
    }

    // Text colours: the selected weapon's and the throwable's (`FUN_140aa9c10`).
    let weapon_color = if info.alive {
        ctx.colors.weapon(
            info.side,
            info.weapon
                .as_ref()
                .and_then(|w| w.loaded.as_ref())
                .map(|m| &m.timers),
        )
    } else {
        ctx.colors.unload
    };
    let throw_color = if info.alive {
        ctx.colors.weapon(
            info.side,
            info.throwable.as_ref().and_then(|t| t.loaded.as_ref()),
        )
    } else {
        ctx.colors.unload
    };

    fill_weapon(&mut c, info, ctx, weapon_color);

    // Throwables (`FUN_141180f20`): "x<count>" and the magazine's short name.
    match &info.throwable {
        Some(t) => {
            for idc in [idc::COUNTER_MEASURES_AMMO, idc::COUNTER_MEASURES_MODE] {
                c.show(idc, true);
                c.color(idc, throw_color);
            }
            c.text(
                idc::COUNTER_MEASURES_AMMO,
                sprintf("x%d", &[t.count.into()]),
            );
            c.text(idc::COUNTER_MEASURES_MODE, t.magazine_name.clone());
        }
        None => {
            c.show(idc::COUNTER_MEASURES_AMMO, false);
            c.show(idc::COUNTER_MEASURES_MODE, false);
        }
    }

    // The gunner's weapon line only shows for a vehicle gunner other than the player.
    c.show(idc::WEAPON_GUNNER, false);

    // The background shows while the weapon, gunner weapon or speed line does.
    if driven_control(c.ui, c.display, idc::BG).is_some() {
        let show = c.shown(idc::WEAPON) || c.shown(idc::WEAPON_GUNNER) || c.shown(121);
        c.show(idc::BG, show);
    }
}

fn fill_weapon(c: &mut Controls<'_>, info: &UnitInfo, ctx: &FillContext, color: Rgba) {
    let Some(w) = &info.weapon else {
        for idc in [
            idc::WEAPON,
            idc::AMMO,
            idc::AMMOCOUNT,
            idc::MAGCOUNT,
            idc::ZERO_26006,
            idc::TOTAL_CAPACITY_26106,
            idc::CAPACITY_26206,
            idc::WEAPON_MODE_TEXTURE,
            idc::WEAPON_MODE,
            idc::WEAPON_AMMO,
        ] {
            c.show(idc, false);
        }
        return;
    };
    let ammo = w.loaded.map_or(0, |m| m.ammo);
    let nothing_left = ammo + w.magazines < 1;
    if w.capacity < 1 {
        // A weapon without magazines: its name in the side's colour and the mode.
        c.show(idc::WEAPON, true);
        c.color(idc::WEAPON, ctx.colors.ready_for(info.side));
        c.text(idc::WEAPON, w.display_name.clone());
        for idc in [idc::AMMO, idc::AMMOCOUNT, idc::MAGCOUNT] {
            c.text(idc, String::new());
        }
        c.text(idc::WEAPON_MODE, w.mode_name.clone());
        if nothing_left && w.mode_name.is_empty() {
            c.show(idc::WEAPON_MODE, false);
        }
        return;
    }

    // Reload progress: the shot cycle of slow weapons, or the magazine reload.
    let (cycling, progress, total) = match &w.loaded {
        Some(m) if m.timers.reload_remaining <= 0.0 => (
            m.timers.cycle,
            m.timers.cycle * w.reload_time,
            w.reload_time,
        ),
        Some(m) => (
            1.0,
            w.reload_time * m.timers.cycle + m.timers.reload_remaining,
            m.timers.reload_duration + w.reload_time,
        ),
        None => (0.0, 0.0, 0.0),
    };
    let empty = ammo == 0;
    if cycling <= 0.001 || empty || total <= 0.5 {
        c.progress(idc::VALUE_RELOAD, 0.0, 1.0);
        c.show(idc::VALUE_RELOAD, false);
    } else {
        c.show(idc::VALUE_RELOAD, true);
        c.progress(idc::VALUE_RELOAD, progress, total);
    }

    c.show(idc::WEAPON, true);
    c.color(idc::WEAPON, color);
    c.text(idc::WEAPON, w.display_name.clone());
    if nothing_left && w.display_name.is_empty() {
        c.show(idc::WEAPON, false);
    }

    let ammo_text = if w.magazines < 1 {
        sprintf(&ctx.formats.ammo_empty, &[ammo.into()])
    } else {
        sprintf(&ctx.formats.ammo, &[ammo.into(), w.magazines.into()])
    };
    for (idc, text) in [
        (idc::AMMO, ammo_text),
        (idc::AMMOCOUNT, sprintf("%d", &[ammo.into()])),
        (
            idc::MAGCOUNT,
            if w.magazines > 0 {
                sprintf("| %d", &[w.magazines.into()])
            } else {
                String::new()
            },
        ),
        (idc::ZERO_26006, "0".to_owned()),
        (
            idc::TOTAL_CAPACITY_26106,
            sprintf("%d", &[w.total_capacity.into()]),
        ),
        (idc::CAPACITY_26206, sprintf("%d", &[w.capacity.into()])),
    ] {
        c.text(idc, text);
        c.show(idc, true);
        c.color(idc, color);
    }

    c.show(idc::WEAPON_MODE_TEXTURE, true);
    c.color(idc::WEAPON_MODE_TEXTURE, color);
    if let Some(texture) = ctx.mode_textures.get(&w.mode_texture_type) {
        c.text(idc::WEAPON_MODE_TEXTURE, texture.to_owned());
    }

    c.show(idc::WEAPON_MODE, true);
    c.color(idc::WEAPON_MODE, color);
    if !w.mode_name.is_empty() {
        c.text(idc::WEAPON_MODE, w.mode_name.clone());
    }
    if nothing_left && w.mode_name.is_empty() {
        c.show(idc::WEAPON_MODE, false);
    }

    c.color(idc::WEAPON_AMMO, color);
    if w.magazine_name.is_empty() {
        c.show(idc::WEAPON_AMMO, false);
    } else {
        c.show(idc::WEAPON_AMMO, true);
        c.text(idc::WEAPON_AMMO, w.magazine_name.clone());
    }
}
