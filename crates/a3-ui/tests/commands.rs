//! SQF UI commands on a test host.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_sqf::{Host, Registry, Value, Vm};
use a3_ui::commands::{fire_control_event, open_display};
use a3_ui::{Screen, Ui, UiHost, register_ui_commands};

const CONFIG: &str = r#"
class RscText { type = 0; idc = -1; style = 0; x = 0; y = 0; w = 0.3; h = 0.04; text = ""; };
class RscButton: RscText { type = 1; };
class RscListBox: RscText { type = 5; };
class RscTest {
    idd = 46;
    onLoad = "uiNamespace setVariable ['loaded', _this select 0]";
    class Controls {
        class Title: RscText { idc = 100; text = "Hello"; x = "safeZoneX"; onLoad = "titleLoaded = true"; };
        class Button: RscButton { idc = 101; action = "actionRan = true"; };
        class List: RscListBox { idc = 102; };
    };
};
class RscChild { idd = 47; class Controls { class T: RscText { idc = 1; }; }; };
"#;

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

fn vm() -> Vm<TestHost> {
    let mut registry = Registry::with_core();
    register_ui_commands(&mut registry);
    let host = TestHost {
        ui: Ui::new(Screen::new(1920, 1080)),
        config: Arc::new(ConfigTree::from_config(&parse_text(CONFIG).unwrap())),
        errors: Vec::new(),
    };
    Vm::with_registry(host, Rc::new(registry))
}

fn eval(vm: &mut Vm<TestHost>, src: &str) -> String {
    match vm.eval(src) {
        Ok(v) => v.to_sqf_string(),
        Err(e) => panic!("{src}\n{}", e.report),
    }
}

#[test]
fn layout_commands_follow_the_metrics() {
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "safeZoneX"), "-0.452381");
    assert_eq!(eval(&mut vm, "safeZoneH"), "1.42857");
    assert_eq!(eval(&mut vm, "pixelGrid"), "12");
    assert_eq!(eval(&mut vm, "getResolution select 3"), "756");
    assert_eq!(eval(&mut vm, "disableSerialization; 1"), "1");
}

#[test]
fn opening_a_display_runs_load_events() {
    let mut vm = vm();
    let d = open_display(&mut vm, "RscTest").unwrap();
    assert_eq!(eval(&mut vm, "titleLoaded"), "true");
    assert_eq!(
        eval(&mut vm, "ctrlIDD (uiNamespace getVariable 'loaded')"),
        "46"
    );
    assert_eq!(eval(&mut vm, "ctrlIDD findDisplay 46"), "46");
    assert_eq!(eval(&mut vm, "count allDisplays"), "1");
    // Expressions in config are evaluated on the VM.
    let title = vm.host.ui.find_control(d, 100).unwrap();
    assert!((vm.host.ui.control(title).unwrap().position[0] + 0.452381).abs() < 1e-5);
    assert!(vm.host.errors.is_empty(), "{:?}", vm.host.errors);
}

#[test]
fn control_commands() {
    let mut vm = vm();
    open_display(&mut vm, "RscTest").unwrap();
    let src = r#"
        private _d = findDisplay 46;
        private _c = _d displayCtrl 100;
        _c ctrlSetText "World";
        _c ctrlSetPosition [0.2, 0.3];
        _c ctrlCommit 0;
        _c ctrlShow false;
        [ctrlText _c, ctrlPosition _c, ctrlShown _c, ctrlIDC _c, ctrlClassName _c, ctrlType _c, isNull (_d displayCtrl 999)]
    "#;
    assert_eq!(
        eval(&mut vm, src),
        "[\"World\",[0.2,0.3,0.3,0.04],false,100,\"Title\",0,true]"
    );
}

#[test]
fn commit_with_duration_animates() {
    let mut vm = vm();
    open_display(&mut vm, "RscTest").unwrap();
    eval(
        &mut vm,
        "private _c = findDisplay 46 displayCtrl 100; _c ctrlSetPosition [1, 0]; _c ctrlCommit 1",
    );
    assert_eq!(
        eval(&mut vm, "ctrlCommitted (findDisplay 46 displayCtrl 100)"),
        "false"
    );
    vm.host.ui.update(0.5);
    let x = eval(
        &mut vm,
        "ctrlPosition (findDisplay 46 displayCtrl 100) select 0",
    );
    assert!(x.starts_with("0.27"), "{x}");
    vm.host.ui.update(1.0);
    assert_eq!(
        eval(&mut vm, "ctrlCommitted (findDisplay 46 displayCtrl 100)"),
        "true"
    );
}

#[test]
fn event_handlers_and_activation() {
    let mut vm = vm();
    let d = open_display(&mut vm, "RscTest").unwrap();
    eval(
        &mut vm,
        "(findDisplay 46 displayCtrl 101) ctrlAddEventHandler ['ButtonClick', { clickedWith = ctrlIDC (_this select 0) }]",
    );
    eval(&mut vm, "ctrlActivate (findDisplay 46 displayCtrl 101)");
    assert_eq!(eval(&mut vm, "clickedWith"), "101");
    assert_eq!(eval(&mut vm, "actionRan"), "true");
    let button = vm.host.ui.find_control(d, 101).unwrap();
    vm.eval("clickedWith = 0").unwrap();
    fire_control_event(&mut vm, button, "buttonclick", Vec::new());
    assert_eq!(eval(&mut vm, "clickedWith"), "101");
}

#[test]
fn list_commands() {
    let mut vm = vm();
    open_display(&mut vm, "RscTest").unwrap();
    let src = r#"
        private _l = findDisplay 46 displayCtrl 102;
        _l lbAdd "a"; private _i = _l lbAdd "b";
        _l lbSetData [_i, "data-b"]; _l lbSetValue [_i, 7]; _l lbSetCurSel 1;
        [lbSize _l, _l lbText 0, _l lbData 1, _l lbValue 1, lbCurSel _l]
    "#;
    assert_eq!(eval(&mut vm, src), "[2,\"a\",\"data-b\",7,1]");
}

#[test]
fn create_and_close_displays_and_controls() {
    let mut vm = vm();
    open_display(&mut vm, "RscTest").unwrap();
    let src = r#"
        private _child = findDisplay 46 createDisplay "RscChild";
        private _new = findDisplay 46 ctrlCreate ["RscText", 555];
        private _r = [ctrlIDD _child, ctrlIDC _new, count allDisplays];
        _child closeDisplay 1;
        ctrlDelete _new;
        _r + [count allDisplays, isNull (findDisplay 46 displayCtrl 555)]
    "#;
    assert_eq!(eval(&mut vm, src), "[47,555,2,1,true]");
}

#[test]
fn variables_on_displays_and_controls() {
    let mut vm = vm();
    open_display(&mut vm, "RscTest").unwrap();
    let src = r#"
        private _d = findDisplay 46; private _c = _d displayCtrl 100;
        _d setVariable ["a", 1]; _c setVariable ["b", 2];
        [_d getVariable "a", _c getVariable ["b", 0], _c getVariable ["missing", 3]]
    "#;
    assert_eq!(eval(&mut vm, src), "[1,2,3]");
    let _ = Value::Nil;
}
