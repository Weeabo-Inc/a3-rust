//! The HUD layer over a synthetic `RscInGameUI`: which controls the engine drives, with what.

use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_ingame_ui::HintText;
use a3_ingame_ui::{
    InGameUi, LoadedMagazine, MagazineTimers, Side, Stance, ThrowableState, UnitInfo, WeaponState,
    driven_control, idc,
};
use a3_ui::{Control, DisplayId, Fonts, Screen};

const CONFIG: &str = r##"
class RscText { type = 0; idc = -1; style = 0; x = 0; y = 0; w = 0.1; h = 0.03; text = ""; sizeEx = 0.03;
    colorText[] = {1, 1, 1, 1}; colorBackground[] = {0, 0, 0, 0}; };
class RscPicture: RscText { style = 48; };
class RscProgress { type = 8; idc = -1; style = 0; x = 0; y = 0; w = 0.1; h = 0.01;
    colorBar[] = {0, 1, 0, 1}; colorFrame[] = {0, 0, 0, 0}; };
class RscControlsGroup { type = 15; idc = -1; x = 0; y = 0; w = 1; h = 1; class Controls {}; };
class RscInGameUI {
    colorReady[] = {1, 1, 1, 1};
    colorReadyWest[] = {"(profilenamespace getvariable ['TEST_R', 0.25])", 0.5, 0.75, 1};
    colorReadyEast[] = {1, 0, 0, 1};
    colorReadyIndependent[] = {0, 1, 0, 1};
    colorReadyCivilian[] = {1, 0, 1, 1};
    colorPrepare[] = {0.8, 0.5, 0, 1};
    colorUnload[] = {0.8, 0, 0, 1};
    class RscUnitInfo {
        idd = 300;
        controls[] = {"Panel"};
        class Panel: RscControlsGroup {
            idc = 2302;
            class controls {
                class Weapon: RscText { idc = 118; };
                class Mode: RscPicture { idc = 187; };
                class Reload: RscProgress { idc = 154; };
                class AmmoCount: RscText { idc = 184; };
                class MagCount: RscText { idc = 185; };
                class AmmoType: RscText { idc = 155; };
                class GrenadeType: RscText { idc = 152; };
                class GrenadeCount: RscText { idc = 151; };
                class Gunner: RscText { idc = 150; };
                class Static: RscText { idc = -1; text = "static"; };
            };
        };
        class Unlisted: RscText { idc = 118; };
    };
    class RscUnitInfoSoldier: RscUnitInfo {
        controls[] = {"Panel", "Freefall"};
        class Freefall: RscText { idc = 380; };
    };
    class RscHint {
        idd = 301;
        controls[] = {"Background", "Hint"};
        class Background: RscText { idc = 101; x = 0.7; y = 0.24; w = 0.3; h = 0.32; colorBackground[] = {0, 0, 0, 0.7}; };
        class Hint: RscText { type = 13; style = 16; idc = 102; x = 0.71; y = 0.252; w = 0.28; h = 0.288; size = 0.032; sizeEx = 0.027; };
    };
    class RscStanceInfo {
        idd = 303;
        controls[] = {"StanceIndicator"};
        class StanceIndicator: RscPicture { idc = 188; };
    };
};
class CfgInGameUI {
    class PlayerInfo { dimmStartTime = 5; dimmEndTime = 10; top = 0.177; };
    class Hint { dimmStartTime = 30; dimmEndTime = 35; sound[] = {"hint.ogg", 0.05, 1}; };
    class CfgWeaponModeTextures { default = "#(argb,8,8,3)color(0,0,0,0)"; semi = "mode_1.paa"; };
    class CfgStanceIndicatorTextures {
        class Normal { textureStand = "si_stand.paa"; textureCrouch = "si_crouch.paa"; };
    };
};
class RscChatListMission {
    x = 0.1; y = 0.8; w = 0.5; h = 0.03; rows = 4; font = "Test"; size = 0.025;
    colorBackground[] = {0, 0, 0, 0.3};
    colorGlobalChannel[] = {0.8, 0.8, 0.8, 1};
    colorMessage[] = {1, 1, 1, 1};
    colorMessageProtocol[] = {0.65, 0.65, 0.65, 1};
    shadow = 1; shadowPlayer = 0; shadowColor[] = {0, 0, 0, 0.5};
};
class CfgDifficultyPresets {
    defaultPreset = "Regular";
    class Regular { class Options { weaponInfo = 2; stanceIndicator = 1; }; };
};
"##;

fn hud() -> InGameUi {
    let config = Arc::new(ConfigTree::from_config(&parse_text(CONFIG).unwrap()));
    InGameUi::new(config, None, None, Screen::new(1920, 1080))
}

fn rifleman() -> UnitInfo {
    UnitInfo {
        alive: true,
        side: Side::West,
        unit_info_types: vec!["RscUnitInfoSoldier".to_owned()],
        on_foot: true,
        weapon: Some(WeaponState {
            display_name: "MX 6.5 mm".to_owned(),
            mode_name: "Semi".to_owned(),
            mode_texture_type: "semi".to_owned(),
            reload_time: 0.096,
            magazine_name: "6.5mm".to_owned(),
            loaded: Some(LoadedMagazine {
                ammo: 30,
                timers: MagazineTimers::default(),
            }),
            magazines: 9,
            capacity: 30,
            total_capacity: 300,
        }),
        throwable: Some(ThrowableState {
            magazine_name: "RGO".to_owned(),
            count: 2,
            loaded: Some(MagazineTimers::default()),
        }),
        ..UnitInfo::default()
    }
}

fn unit_display(hud: &InGameUi) -> DisplayId {
    let slots = hud.unit_info_displays();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].0, "RscUnitInfoSoldier");
    slots[0].1.expect("display built")
}

fn control(hud: &InGameUi, d: DisplayId, idc: i32) -> &Control {
    let id = driven_control(hud.ui(), d, idc).unwrap_or_else(|| panic!("no idc {idc}"));
    hud.ui().control(id).unwrap()
}

#[test]
fn the_weapon_panel_shows_weapon_ammo_mode_and_grenades() {
    let mut hud = hud();
    hud.update(&rifleman(), 0.0);
    let d = unit_display(&hud);
    let west = [0.25, 0.5, 0.75, 1.0];
    let shown = |idc: i32| control(&hud, d, idc).show;
    let text = |idc: i32| control(&hud, d, idc).text.clone();

    assert_eq!(text(idc::WEAPON), "MX 6.5 mm");
    assert_eq!(
        control(&hud, d, idc::WEAPON).color_text,
        west,
        "ready: the side's colour"
    );
    assert_eq!(text(idc::AMMOCOUNT), "30");
    assert_eq!(text(idc::MAGCOUNT), "| 9");
    assert_eq!(text(idc::WEAPON_AMMO), "6.5mm");
    assert_eq!(text(idc::WEAPON_MODE_TEXTURE), "mode_1.paa");
    assert_eq!(control(&hud, d, idc::WEAPON_MODE_TEXTURE).color_text, west);
    assert_eq!(text(idc::COUNTER_MEASURES_AMMO), "x2");
    assert_eq!(text(idc::COUNTER_MEASURES_MODE), "RGO");
    assert!(
        !shown(idc::VALUE_RELOAD),
        "a fast rifle shows no reload bar"
    );
    assert!(!shown(idc::WEAPON_GUNNER), "no gunner weapon on foot");
    assert!(!shown(idc::SPEED_FREEFALL), "not falling");

    // The display's own `controls[]` decides what exists: the unlisted IDC 118 class does not.
    let weapons = hud
        .ui()
        .controls_in_order(d)
        .into_iter()
        .filter(|&c| hud.ui().control(c).unwrap().idc == idc::WEAPON)
        .count();
    assert_eq!(weapons, 1);
}

#[test]
fn colours_and_texts_follow_the_weapon_state() {
    let mut hud = hud();
    let mut info = rifleman();
    let w = info.weapon.as_mut().unwrap();
    w.magazines = 0;
    w.loaded = Some(LoadedMagazine {
        ammo: 0,
        timers: MagazineTimers {
            cycle: 0.0,
            reload_remaining: 1.5,
            reload_duration: 3.0,
        },
    });
    hud.update(&info, 0.0);
    let d = unit_display(&hud);
    let unload = hud.context().colors.unload;
    assert_eq!(control(&hud, d, idc::WEAPON).color_text, unload);
    assert_eq!(control(&hud, d, idc::AMMOCOUNT).text, "0");
    assert_eq!(
        control(&hud, d, idc::MAGCOUNT).text,
        "",
        "no spare magazines: empty"
    );
    assert!(
        !control(&hud, d, idc::VALUE_RELOAD).show,
        "an empty magazine shows no reload bar"
    );

    // A slow weapon between shots: the bar runs over the shot cycle, prepare colour near the end.
    let w = info.weapon.as_mut().unwrap();
    w.reload_time = 1.5;
    w.loaded = Some(LoadedMagazine {
        ammo: 4,
        timers: MagazineTimers {
            cycle: 0.1,
            ..MagazineTimers::default()
        },
    });
    hud.update(&info, 0.1);
    let bar = control(&hud, d, idc::VALUE_RELOAD);
    assert!(bar.show);
    assert_eq!(bar.range, [0.0, 1.5]);
    assert!((bar.value - 0.15).abs() < 1e-6);
    assert_eq!(
        control(&hud, d, idc::WEAPON).color_text,
        hud.context().colors.prepare
    );

    info.weapon = None;
    info.throwable = None;
    hud.update(&info, 0.2);
    for idc in [
        idc::WEAPON,
        idc::AMMOCOUNT,
        idc::MAGCOUNT,
        idc::WEAPON_AMMO,
        idc::WEAPON_MODE_TEXTURE,
        idc::COUNTER_MEASURES_AMMO,
        idc::COUNTER_MEASURES_MODE,
    ] {
        assert!(
            !control(&hud, d, idc).show,
            "idc {idc} hidden without a weapon"
        );
    }
    assert_eq!(
        control(&hud, d, -1).text,
        "static",
        "undriven controls keep their config"
    );
}

#[test]
fn a_dead_unit_shows_everything_in_the_unload_colour() {
    let mut hud = hud();
    let mut info = rifleman();
    info.alive = false;
    hud.update(&info, 0.0);
    let d = unit_display(&hud);
    let unload = hud.context().colors.unload;
    assert_eq!(control(&hud, d, idc::WEAPON).color_text, unload);
    assert_eq!(
        control(&hud, d, idc::COUNTER_MEASURES_AMMO).color_text,
        unload
    );
}

#[test]
fn unit_info_displays_follow_the_vehicle() {
    let mut hud = hud();
    let mut info = rifleman();
    hud.update(&info, 0.0);
    let first = unit_display(&hud);
    hud.update(&info, 0.1);
    assert_eq!(unit_display(&hud), first, "kept while the class stays");
    info.unit_info_types = vec!["RscUnitInfo".to_owned(), "RscNoSuchInfo".to_owned()];
    hud.update(&info, 0.2);
    let slots = hud.unit_info_displays().to_vec();
    assert_eq!(slots.len(), 2);
    assert!(slots[0].1.is_some() && slots[1].1.is_none());
    assert!(hud.ui().display(first).is_none(), "the old display is gone");
    assert!(
        hud.ui().stack().contains(&hud.stance_display().unwrap()),
        "the stance display stays open"
    );
}

#[test]
fn the_stance_indicator_fades_after_a_change() {
    let mut hud = hud();
    let mut info = rifleman();
    info.stance = Stance::Stand;
    hud.update(&info, 0.0);
    let d = hud.stance_display().expect("stance display");
    let stance = |hud: &InGameUi| control(hud, d, idc::STANCE_INDICATOR).text.clone();
    let alpha = |hud: &InGameUi| hud.ui().display(d).unwrap().alpha;
    assert_eq!(stance(&hud), "si_stand.paa");
    assert_eq!(alpha(&hud), 1.0);
    // stanceIndicator = 1: full for dimmStartTime, then fading out until dimmEndTime.
    hud.update(&info, 7.5);
    assert!((alpha(&hud) - 0.5).abs() < 1e-6);
    hud.update(&info, 11.0);
    assert_eq!(alpha(&hud), 0.0);
    info.stance = Stance::Crouch;
    hud.update(&info, 12.0);
    assert_eq!(stance(&hud), "si_crouch.paa");
    assert_eq!(alpha(&hud), 1.0, "a change shows it again");
    info.on_foot = false;
    hud.update(&info, 12.1);
    assert_eq!(alpha(&hud), 0.0, "not in vehicles");
}

struct NoFonts;

impl a3_ui::FontLoader for NoFonts {
    fn load_font(&mut self, _: &str) -> Option<a3_fonts::Font> {
        None
    }
}

#[test]
fn a_hint_fits_its_text_moves_to_the_player_info_top_and_fades() {
    let mut hud = hud();
    let mut fonts = Fonts::from_config(&ConfigTree::new(), Box::new(NoFonts));
    let info = rifleman();
    hud.update(&info, 0.0);
    let d = hud.hint_display().expect("hint display");
    let alpha = |hud: &InGameUi| hud.ui().display(d).unwrap().alpha;
    assert_eq!(alpha(&hud), 0.0, "no hint yet");

    let sound = hud.show_hint(
        &HintText::Plain(
            "Line one
Line two"
                .into(),
        ),
        true,
    );
    assert_eq!(sound.map(|s| s.path), Some("hint.ogg".to_owned()));
    assert_eq!(
        hud.show_hint(
            &HintText::Plain(
                "Line one
Line two"
                    .into()
            ),
            false
        ),
        None
    );
    hud.update(&info, 1.0);
    hud.draw(&mut fonts);
    let bg = control(&hud, d, 101).position;
    let text = control(&hud, d, 102).position;
    // Two lines of `size` 0.032; the background keeps its 0.032 margin below the text box.
    assert!((text[3] - 0.064).abs() < 1e-6, "{text:?}");
    assert!((bg[3] - (0.32 - 0.288 + 0.064)).abs() < 1e-6, "{bg:?}");
    assert!((bg[1] - 0.177).abs() < 1e-6 && (text[1] - 0.189).abs() < 1e-6);
    assert_eq!(
        control(&hud, d, 102).text,
        "Line one
Line two"
    );
    assert_eq!(alpha(&hud), 1.0);
    // Shown at 0 for 35 s: full until 5 s are left, then fading.
    hud.update(&info, 32.5);
    assert!((alpha(&hud) - 0.5).abs() < 1e-5, "{}", alpha(&hud));
    hud.update(&info, 36.1);
    assert_eq!(alpha(&hud), 0.0);
}

#[test]
fn scripts_show_hints_and_system_chat() {
    let mut hud = hud();
    let mut fonts = Fonts::from_config(&ConfigTree::new(), Box::new(NoFonts));
    let info = rifleman();
    hud.update(&info, 0.0);
    hud.exec("systemChat 'Game saved'; hintSilent 'Hello'")
        .unwrap();
    let chat = hud.chat().messages();
    assert_eq!(chat.len(), 1);
    assert_eq!(chat[0].text, "Game saved");
    assert!(chat[0].protocol);
    assert_eq!(hud.chat().config.rows, 4);
    hud.update(&info, 1.0);
    let list = hud.draw(&mut fonts);
    let d = hud.hint_display().unwrap();
    assert_eq!(control(&hud, d, 102).text, "Hello");
    // The chat line's background: on the bottom row (y + 3 rows), 0.3 black.
    let m = hud.ui().metrics;
    let bottom = m.rect_to_px([0.0, 0.8 + 3.0 * 0.03, 0.0, 0.0])[1];
    assert!(
        list.quads.iter().any(|q| q.texture.is_none()
            && q.color == [0.0, 0.0, 0.0, 0.3]
            && (q.rect[1] - bottom).abs() < 0.01),
        "no chat background at y {bottom}"
    );
    // Gone after 30 s.
    hud.update(&info, 31.0);
    let list = hud.draw(&mut fonts);
    assert!(
        !list
            .quads
            .iter()
            .any(|q| (q.rect[1] - bottom).abs() < 0.01 && q.texture.is_none())
    );
}
