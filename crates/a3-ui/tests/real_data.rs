//! Building displays from the real merged game config. Skipped when `A3_ROOT` is unset.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::ConfigTree;
use a3_gamedata::{GameData, LoadOptions};
use a3_sqf::{Host, Registry, Vm};
use a3_ui::commands::open_display;
use a3_ui::{ControlId, ControlType, NoEval, Screen, Ui, UiHost, UiMetrics, register_ui_commands};

struct TestHost {
    ui: Ui,
    config: Arc<ConfigTree>,
    errors: Vec<String>,
}

impl Host for TestHost {
    fn report_error(&mut self, error: &a3_sqf::ScriptError) {
        self.errors.push(error.report.clone());
    }
}

impl UiHost for TestHost {
    fn ui(&self) -> &Ui {
        &self.ui
    }
    fn ui_mut(&mut self) -> &mut Ui {
        &mut self.ui
    }
    fn ui_config(&self) -> Option<Arc<ConfigTree>> {
        Some(Arc::clone(&self.config))
    }
}

fn load() -> Option<GameData> {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return None;
    };
    Some(GameData::load(&LoadOptions::new(root)).unwrap())
}

fn vm(config: &Arc<ConfigTree>) -> Vm<TestHost> {
    let mut registry = Registry::with_core();
    register_ui_commands(&mut registry);
    let host = TestHost {
        ui: Ui::new(Screen::from_config(1920, 1080, config)),
        config: Arc::clone(config),
        errors: Vec::new(),
    };
    Vm::with_registry(host, Rc::new(registry))
}

/// Every control below `root`, groups included.
fn descendants(ui: &Ui, root: ControlId) -> Vec<ControlId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(c) = stack.pop() {
        for &child in &ui.control(c).unwrap().children {
            out.push(child);
            stack.push(child);
        }
    }
    out
}

fn by_class(ui: &Ui, display: a3_ui::DisplayId, class: &str) -> Option<ControlId> {
    ui.controls_in_order(display).into_iter().find(|&c| {
        ui.control(c)
            .unwrap()
            .class_name
            .eq_ignore_ascii_case(class)
    })
}

#[test]
fn the_real_config_gives_the_layout_and_builds_rsc_display_main() {
    let Some(data) = load() else {
        return;
    };
    let config = Arc::clone(&data.config);

    // `bin\config.bin`: uiScaleMaxGrids 60, uiScaleFactor 4. At 1080p: 1080 / 60 = 18, which is
    // 4.5 grid factors and rounds (ties to even) to 16.
    let metrics = UiMetrics::new(Screen::from_config(1920, 1080, &config));
    assert_eq!(metrics.screen.max_grids, 60.0);
    assert_eq!(metrics.screen.grid_factor, 4.0);
    assert_eq!(metrics.pixel_grid_no_ui_scale, 16.0);

    let mut vm = vm(&config);
    let d = open_display(&mut vm, "RscDisplayMain").expect("RscDisplayMain");
    let ui = &vm.host.ui;
    assert_eq!(ui.display(d).unwrap().idd, 0);

    // `Button3DEditor` (idc 115) is a button (type 1, style 2) in the display's `controls` list.
    let button = ui.control(ui.find_control(d, 115).unwrap()).unwrap();
    assert_eq!(button.class_name, "Button3DEditor");
    assert_eq!(button.kind, ControlType::Button);
    assert_eq!(button.style, 2);

    // `BackgroundSpotlight`'s position is an expression over the pixel grid:
    //   x = 0.5 - (1.5 * 10) * (pixelW * pixelGridNoUIScale * 2) - 2 * (2 * pixelW)
    //   w = 3 * 10 * (pixelW * pixelGridNoUIScale * 2) + 4 * (2 * pixelW)
    // with pixelW = 1/1008 at 1920x1080 / Normal.
    let spot = by_class(ui, d, "BackgroundSpotlight").unwrap();
    let spot = ui.control(spot).unwrap();
    let pixel = 1.0 / 1008.0;
    assert!((spot.position[0] - (0.5 - 15.0 * 32.0 * pixel - 4.0 * pixel)).abs() < 1e-5);
    assert!((spot.position[2] - (30.0 * 32.0 * pixel + 8.0 * pixel)).abs() < 1e-6);

    // `Spotlight1` (idc 1021) is a controls group holding a `GroupPicture` whose `Controls`
    // list holds a picture and a video.
    let group = ui.control(ui.find_control(d, 1021).unwrap()).unwrap();
    assert_eq!(group.class_name, "Spotlight1");
    assert_eq!(group.kind, ControlType::ControlsGroup);
    let below = descendants(ui, ui.find_control(d, 1021).unwrap());
    assert!(
        below.len() >= 8,
        "{} controls below Spotlight1",
        below.len()
    );
    // A `style = 48` static's texture comes from `text` (what the engine's picture draw uses).
    let picture = below
        .iter()
        .map(|&c| ui.control(c).unwrap())
        .find(|c| c.class_name == "Picture")
        .unwrap();
    assert!(picture.text.ends_with("spotlight_2_ca.paa"), "{picture:?}");
}

#[test]
fn a_real_display_loads_its_object_list() {
    let Some(data) = load() else {
        return;
    };
    let mut vm = vm(&Arc::clone(&data.config));
    let d = open_display(&mut vm, "RscDisplayMainMap").expect("RscDisplayMainMap");
    let ui = &vm.host.ui;
    assert_eq!(ui.display(d).unwrap().idd, 12);

    // `RscDisplayMainMap >> objects`: the compass and notepad 3D objects, not part of the 2D
    // draw order but found by `displayCtrl`.
    let names: Vec<String> = ui
        .display(d)
        .unwrap()
        .objects
        .iter()
        .map(|&c| ui.control(c).unwrap().class_name.clone())
        .collect();
    assert_eq!(names, ["Compass", "Notepad"]);
    let compass = ui.find_control(d, 102).expect("Compass");
    assert_eq!(ui.control(compass).unwrap().kind, ControlType::Object);
    assert!(!ui.controls_in_order(d).contains(&compass));
}

#[test]
fn every_real_rsc_display_class_builds() {
    let Some(data) = load() else {
        return;
    };
    let mut ui = Ui::new(Screen::from_config(1920, 1080, &data.config));
    let mut total = 0;
    let mut built = 0;
    let mut with_controls = 0;
    let mut largest = (String::new(), 0usize);
    for entry in data.config.root().entries() {
        let name = entry.name().to_owned();
        if !entry.is_class() || !name.starts_with("RscDisplay") {
            continue;
        }
        total += 1;
        if let Ok(d) = ui.create_display(&data.config, &name, None, false, &mut NoEval) {
            built += 1;
            let controls = ui.controls_in_order(d).len();
            if controls > 0 {
                with_controls += 1;
            }
            if controls > largest.1 {
                largest = (name, controls);
            }
        }
    }
    eprintln!(
        "RscDisplay* classes {total}: built {built}, with controls {with_controls}, \
         largest {} ({})",
        largest.0, largest.1
    );
    assert_eq!(built, total);
    assert!(total >= 150);
    assert!(with_controls * 4 >= total * 3, "{with_controls} of {total}");
}
