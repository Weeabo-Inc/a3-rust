//! Building displays from config, layout, animations and hit testing.

use a3_config::{ConfigTree, parse_text};
use a3_ui::{ControlType, Eval, NoEval, Screen, Ui, style};

const CONFIG: &str = r#"
class RscText {
    type = 0; idc = -1; style = 0;
    x = 0; y = 0; w = 0.3; h = 0.04;
    font = "RobotoCondensed"; sizeEx = 0.04;
    colorBackground[] = {0, 0, 0, 0};
    colorText[] = {1, 1, 1, 1};
    text = "";
};
class RscPicture: RscText { style = 48; };
class RscControlsGroup {
    type = 15; idc = -1; style = 16;
    x = 0; y = 0; w = 1; h = 1;
    class Controls {};
};
class RscObject { type = 80; idc = -1; x = 0; y = 0; w = 0; h = 0; };
class RscTest {
    idd = 46;
    onLoad = "loaded = true";
    class Objects {
        class Preview: RscObject { idc = 789; model = "\a3\weapons_f\empty"; };
    };
    class ControlsBackground {
        class Back: RscPicture {
            text = "\a3\ui_f\data\back.paa";
            x = "safeZoneX"; y = "safeZoneY"; w = "safeZoneW"; h = "safeZoneH";
        };
    };
    class Controls {
        class Title: RscText {
            idc = 100;
            text = "$STR_Title";
            x = 0.1; y = 0.1;
            colorBackground[] = {"(profileNamespace getVariable ['GUI_BCG_RGB_R', 0.13])", 0.5, 0.25, 1};
            onButtonClick = "clicked = true";
        };
        class Group: RscControlsGroup {
            idc = 200;
            x = 0.5; y = 0.5; w = 0.2; h = 0.2;
            class Controls {
                class Inner: RscText { idc = 201; x = 0.05; y = 0.05; w = 0.1; h = 0.1; };
            };
        };
    };
};
"#;

fn config() -> ConfigTree {
    ConfigTree::from_config(&parse_text(CONFIG).unwrap())
}

/// Evaluates the few expressions the test config uses.
struct TestEval;

impl Eval for TestEval {
    fn number(&mut self, expression: &str) -> Option<f32> {
        let m = a3_ui::UiMetrics::new(Screen::new(1920, 1080));
        match expression.trim() {
            "safeZoneX" => Some(m.safe_zone_x),
            "safeZoneY" => Some(m.safe_zone_y),
            "safeZoneW" => Some(m.safe_zone_w),
            "safeZoneH" => Some(m.safe_zone_h),
            e if e.contains("GUI_BCG_RGB_R") => Some(0.13),
            e => e.parse().ok(),
        }
    }

    fn localize(&mut self, text: &str) -> String {
        if text == "$STR_Title" {
            "Title".to_owned()
        } else {
            text.to_owned()
        }
    }
}

fn ui_with_test_display() -> (Ui, a3_ui::DisplayId) {
    let mut ui = Ui::new(Screen::new(1920, 1080));
    let d = ui
        .create_display(&config(), "RscTest", None, false, &mut TestEval)
        .unwrap();
    (ui, d)
}

#[test]
fn display_is_built_from_config() {
    let (ui, d) = ui_with_test_display();
    let display = ui.display(d).unwrap();
    assert_eq!(display.idd, 46);
    assert_eq!(display.background.len(), 1);
    assert_eq!(display.controls.len(), 2);
    assert_eq!(ui.find_display(46), Some(d));
    assert!(display.events.iter().any(|e| e.event == "load"));

    let title = ui.control(ui.find_control(d, 100).unwrap()).unwrap();
    assert_eq!(title.text, "Title");
    assert_eq!(title.position, [0.1, 0.1, 0.3, 0.04]);
    assert_eq!(title.color_background, [0.13, 0.5, 0.25, 1.0]);
    assert_eq!(title.font, "RobotoCondensed");
    assert!(title.events.iter().any(|e| e.event == "buttonclick"));

    let back = ui.control(display.background[0]).unwrap();
    assert_eq!(back.style & style::TYPE, style::PICTURE);
    assert!((back.position[0] - (-0.452381)).abs() < 1e-5);
}

#[test]
fn controls_groups_hold_children_with_relative_positions() {
    let (ui, d) = ui_with_test_display();
    let group = ui.find_control(d, 200).unwrap();
    let inner = ui.find_control(d, 201).unwrap();
    assert_eq!(ui.control(group).unwrap().kind, ControlType::ControlsGroup);
    assert_eq!(ui.control(inner).unwrap().parent, Some(group));
    let abs = ui.absolute_position(inner).unwrap();
    assert!((abs[0] - 0.55).abs() < 1e-6 && (abs[1] - 0.55).abs() < 1e-6);
    assert_eq!(ui.clip_rect(inner), Some([0.5, 0.5, 0.2, 0.2]));
    assert_eq!(ui.controls_in_order(d).len(), 4);
}

#[test]
fn display_objects_are_a_third_list() {
    // `objects` are 3D object controls (the engine loads them after `controls`); they are not
    // part of the 2D draw order.
    let (ui, d) = ui_with_test_display();
    let preview = ui.find_control(d, 789).unwrap();
    assert_eq!(ui.control(preview).unwrap().kind, ControlType::Object);
    assert_eq!(ui.display(d).unwrap().objects, vec![preview]);
    assert!(!ui.controls_in_order(d).contains(&preview));
}

#[test]
fn commit_animates_position_and_fade() {
    let (mut ui, d) = ui_with_test_display();
    let title = ui.find_control(d, 100).unwrap();
    {
        let c = ui.control_mut(title).unwrap();
        c.pending_position = [0.3, 0.1, 0.3, 0.04];
        c.pending_fade = 1.0;
    }
    ui.commit(title, 2.0);
    assert!(!ui.committed(title));
    ui.update(1.0);
    let c = ui.control(title).unwrap();
    assert!((c.position[0] - 0.2).abs() < 1e-6);
    assert!((c.fade - 0.5).abs() < 1e-6);
    ui.update(2.5);
    assert!(ui.committed(title));
    assert_eq!(ui.control(title).unwrap().position[0], 0.3);
    assert!(!ui.is_visible(title));
}

#[test]
fn hit_testing_finds_the_topmost_control() {
    let (ui, d) = ui_with_test_display();
    let inner = ui.find_control(d, 201).unwrap();
    assert_eq!(ui.control_at(d, 0.56, 0.56), Some(inner));
    let title = ui.find_control(d, 100).unwrap();
    assert_eq!(ui.control_at(d, 0.15, 0.12), Some(title));
}

#[test]
fn closing_and_deleting() {
    let (mut ui, d) = ui_with_test_display();
    let group = ui.find_control(d, 200).unwrap();
    assert!(ui.delete_control(group));
    assert!(ui.find_control(d, 201).is_none());
    assert_eq!(ui.close_display(d), vec![d]);
    assert!(ui.display(d).is_none());
    assert!(ui.stack().is_empty());
}

#[test]
fn unknown_class_is_an_error() {
    let mut ui = Ui::new(Screen::new(800, 600));
    assert!(
        ui.create_display(&config(), "RscNope", None, false, &mut NoEval)
            .is_err()
    );
}

#[test]
fn ctrl_create_uses_display_or_root_classes() {
    let (mut ui, d) = ui_with_test_display();
    let c = ui
        .create_control(&config(), "RscText", 300, d, None, &mut NoEval)
        .unwrap();
    assert_eq!(ui.control(c).unwrap().idc, 300);
    assert_eq!(ui.find_control(d, 300), Some(c));
}

fn idcs(ui: &Ui, ids: &[a3_ui::ControlId]) -> Vec<i32> {
    ids.iter().map(|&c| ui.control(c).unwrap().idc).collect()
}

/// The `RscInGameUI` displays name their controls with an array of class names instead of a
/// class (`controls[] = {"A", "C"};`): the engine looks each name up in the display class
/// (through inheritance) and loads only the listed ones, in list order.
#[test]
fn a_control_list_can_be_an_array_of_class_names() {
    let config = ConfigTree::from_config(
        &parse_text(
            r#"
            class RscText { type = 0; idc = -1; x = 0; y = 0; w = 0.1; h = 0.1; };
            class RscBase {
                idd = 300;
                controls[] = {"A"};
                class A: RscText { idc = 1; };
                class B: RscText { idc = 2; };
                class C: RscText { idc = 3; };
            };
            class RscDerived: RscBase {
                controls[] = {"C", "A", "Missing"};
                controlsBackground[] = {"B"};
            };
            "#,
        )
        .unwrap(),
    );
    let mut ui = Ui::new(Screen::new(1920, 1080));
    let base = ui
        .create_display(&config, "RscBase", None, false, &mut NoEval)
        .unwrap();
    let controls = ui.display(base).unwrap().controls.clone();
    assert_eq!(idcs(&ui, &controls), [1], "only the listed classes load");

    let derived = ui
        .create_display(&config, "RscDerived", None, false, &mut NoEval)
        .unwrap();
    let display = ui.display(derived).unwrap().clone();
    assert_eq!(
        idcs(&ui, &display.controls),
        [3, 1],
        "list order, classes inherited from the base display, unknown names skipped"
    );
    assert_eq!(idcs(&ui, &display.background), [2]);
    let c = ui.control(display.controls[0]).unwrap();
    assert_eq!(c.class_name, "C");
    assert_eq!(c.config_path, ["RscDerived", "C"]);
}
