//! The HUD over the real game config and stringtables. Skipped when `A3_ROOT` is unset.

use std::sync::Arc;

use a3_gamedata::{GameData, LoadOptions, Localizer};
use a3_ingame_ui::loadout::spawned_unit_info;
use a3_ingame_ui::{InGameUi, Side, driven_control, idc};
use a3_ui::{Fonts, Screen};

struct Strings(a3_stringtable::Localizer);

impl Localizer for Strings {
    fn localize(&self, key: &str) -> Option<String> {
        self.0.get(key).map(str::to_owned)
    }
}

struct NoFonts;

impl a3_ui::FontLoader for NoFonts {
    fn load_font(&mut self, _: &str) -> Option<a3_fonts::Font> {
        None
    }
}

#[test]
fn the_rifleman_hud_from_the_real_config() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = GameData::load(&LoadOptions::new(root)).unwrap();
    let (strings, _) = a3_stringtable::Localizer::load_vfs(&data.vfs, "English");
    let localizer: Arc<dyn Localizer> = Arc::new(Strings(strings));
    let mut hud = InGameUi::new(
        Arc::clone(&data.config),
        Some(data.vfs.clone()),
        Some(Arc::clone(&localizer)),
        Screen::from_config(1920, 1080, &data.config),
    );

    // `CfgUIColors` default preset PresetA3: TEXT_RGB {0.95, 0.95, 0.95, 1} into the profile.
    let colors = hud.context().colors;
    assert_eq!(colors.ready_west, [0.95, 0.95, 0.95, 1.0], "{colors:?}");
    assert_eq!(colors.prepare, [0.8, 0.5, 0.0, 1.0]);
    assert_eq!(colors.unload, [0.8, 0.0, 0.0, 1.0]);

    let info = spawned_unit_info(&data.config, "B_Soldier_F", &|k| localizer.localize(k));
    assert_eq!(info.side, Side::West);
    assert_eq!(info.unit_info_types, ["RscUnitInfoSoldier"]);
    let weapon = info.weapon.as_ref().expect("the MX");
    // The stringtable's name has a no-break space.
    assert_eq!(weapon.display_name, "MX 6.5\u{a0}mm");
    assert_eq!(weapon.mode_texture_type, "semi");
    assert_eq!(weapon.loaded.map(|m| m.ammo), Some(30));
    assert_eq!(weapon.magazines, 9);

    hud.update(&info, 0.0);
    let errors = &hud.vm().host.errors;
    eprintln!(
        "{} script errors, first: {:?}",
        errors.len(),
        errors.first()
    );
    let d = hud.unit_info_displays()[0].1.expect("RscUnitInfoSoldier");
    let ui = hud.ui();
    let text = |idc: i32| {
        let c = driven_control(ui, d, idc).unwrap_or_else(|| panic!("no idc {idc}"));
        ui.control(c).unwrap().text.clone()
    };
    assert_eq!(text(idc::WEAPON), weapon.display_name);
    assert_eq!(text(idc::AMMOCOUNT), "30");
    assert_eq!(text(idc::MAGCOUNT), "| 9");
    assert_eq!(text(idc::WEAPON_AMMO), weapon.magazine_name);
    assert!(text(idc::WEAPON_MODE_TEXTURE).ends_with("mode_1_ca.paa"));
    assert_eq!(text(idc::COUNTER_MEASURES_AMMO), "x2");

    // The panel sits at the top right of the screen (`IGUI_GRID_WEAPON_X` default).
    let group = driven_control(ui, d, 2302).expect("WeaponInfoControlsGroupLeft");
    let pos = ui.absolute_position(group).unwrap();
    let px = ui.metrics.rect_to_px(pos);
    // At 1080p / Normal one IGUI grid unit is ((safezoneW / safezoneH) min 1.2) / 40 = 0.03 of
    // the 1008 px viewport: 30.24 px. x = right edge - (10 + 4.3) units, y = 0.5 units.
    let unit = 30.24;
    let expected = [1920.0 - 14.3 * unit, 0.5 * unit, 10.0 * unit, 6.0 * unit];
    for (got, want) in px.iter().zip(expected) {
        assert!((got - want).abs() < 0.01, "{px:?} vs {expected:?}");
    }

    let stance = hud.stance_display().expect("stance");
    let c = driven_control(ui, stance, idc::STANCE_INDICATOR).unwrap();
    assert!(
        ui.control(c)
            .unwrap()
            .text
            .to_ascii_lowercase()
            .ends_with("si_stand_ca.paa")
    );

    let mut fonts = Fonts::from_config(&data.config, Box::new(NoFonts));
    let quads = hud.draw(&mut fonts);
    assert!(!quads.quads.is_empty());
}
