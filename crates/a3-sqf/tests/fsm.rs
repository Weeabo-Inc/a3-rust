//! Scripted FSMs: `execFSM` and the commands around it, stepped by the scheduler.

mod common;

use std::time::Duration;

use common::{TestHost, vm};
use a3_sqf::Vm;

const FRAME: Duration = Duration::from_millis(100);

/// Counts up to 3, one state change per frame, then ends.
const COUNTER: &str = r#"
class FSM {
    fsmName = "Counter";
    class States {
        class Init {
            name = "Init";
            init = "_n = 0; trace = [""init""]; me = _this; handle = _thisFSM;";
            precondition = "trace pushBack ""pre-init"";";
            class Links {
                class Go { priority = 0; to = "Count"; precondition = ""; condition = "true"; action = "trace pushBack ""go"";"; };
            };
        };
        class Count {
            name = "Count";
            init = "_n = _n + 1; trace pushBack format [""count %1"", _n];";
            precondition = "trace pushBack ""pre-count"";";
            class Links {
                class Done { priority = 1; to = "End"; precondition = ""; condition = "_n >= 3"; action = ""; };
                class Again { priority = 0; to = "Count"; precondition = ""; condition = "true"; action = ""; };
            };
        };
        class End {
            name = "End";
            init = "trace pushBack ""end""; result = _n;";
            precondition = "";
            class Links {};
        };
    };
    initState = "Init";
    finalStates[] = {"End"};
};"#;

fn with_file(path: &str, text: &str) -> Vm<TestHost> {
    let mut vm = vm();
    vm.host.files.insert(path.to_ascii_lowercase(), text.to_owned());
    vm
}

fn log(vm: &Vm<TestHost>) -> String {
    vm.get_global("trace").to_sqf_string()
}

#[test]
fn an_fsm_takes_one_link_per_frame_and_ends_in_its_final_state() {
    let mut vm = with_file("counter.fsm", COUNTER);
    vm.eval("h = 42 execFSM \"counter.fsm\"").unwrap();
    assert_eq!(vm.get_global("h").to_sqf_string(), "1");
    // Started, not yet stepped.
    assert!(vm.get_global("trace").is_nil());
    assert_eq!(vm.eval("completedFSM h").unwrap().to_sqf_string(), "false");

    // Frame 1: the init state runs init and precondition at once, then its link fires and
    // the next state's init runs.
    vm.run_scheduled(FRAME);
    assert!(vm.host.errors.is_empty(), "{:?}", vm.host.errors);
    assert_eq!(
        log(&vm),
        r#"["init","pre-init","go","count 1"]"#
    );
    assert_eq!(vm.get_global("me").to_sqf_string(), "42");
    assert_eq!(vm.get_global("handle").to_sqf_string(), "1");
    assert_eq!(vm.eval("h getFSMVariable \"_n\"").unwrap().to_sqf_string(), "1");

    // Frame 2: the state resumes with its precondition, then loops to itself.
    vm.run_scheduled(FRAME);
    assert_eq!(
        log(&vm),
        r#"["init","pre-init","go","count 1","pre-count","count 2"]"#
    );
    vm.run_scheduled(FRAME);
    vm.run_scheduled(FRAME);
    assert_eq!(vm.get_global("result").to_sqf_string(), "3");
    assert!(log(&vm).ends_with(r#""pre-count","end"]"#), "{}", log(&vm));
    assert_eq!(vm.eval("completedFSM h").unwrap().to_sqf_string(), "true");
    assert!(vm.host.errors.is_empty(), "{:?}", vm.host.errors);
}

#[test]
fn fsm_variables_can_be_read_and_written_from_outside() {
    let mut vm = with_file("counter.fsm", COUNTER);
    vm.eval("h = execFSM \"counter.fsm\"").unwrap();
    vm.run_scheduled(FRAME);
    // Jump the count: the next condition check sees 10 >= 3.
    vm.eval("h setFSMVariable [\"_n\", 10]").unwrap();
    assert_eq!(vm.eval("h getFSMVariable \"_N\"").unwrap().to_sqf_string(), "10");
    assert_eq!(
        vm.eval("h getFSMVariable [\"_missing\", 7]").unwrap().to_sqf_string(),
        "7"
    );
    vm.run_scheduled(FRAME);
    assert_eq!(vm.get_global("result").to_sqf_string(), "10");
    assert_eq!(vm.eval("completedFSM h").unwrap().to_sqf_string(), "true");
}

#[test]
fn a_waiting_state_checks_its_links_every_frame_without_rerunning_its_precondition() {
    let text = r#"class FSM { fsmName = "Wait"; class States {
        class A { name = "A"; init = ""; precondition = ""; class Links {
            class Go { priority = 0; to = "B"; precondition = ""; condition = "true"; action = ""; };
        }; };
        class B { name = "B"; init = "pre = 0; checks = 0;"; precondition = "pre = pre + 1;"; class Links {
            class Open { priority = 0; to = "C"; precondition = "checks = checks + 1;"; condition = "!isNil ""open"""; action = ""; };
        }; };
        class C { name = "C"; init = "reached = true;"; precondition = ""; class Links {}; };
    }; initState = "A"; finalStates[] = {"C"}; };"#;
    let mut vm = with_file("wait.fsm", text);
    vm.eval("h = execFSM \"wait.fsm\"").unwrap();
    for _ in 0..4 {
        vm.run_scheduled(FRAME);
    }
    assert_eq!(vm.get_global("pre").to_sqf_string(), "1");
    assert_eq!(vm.get_global("checks").to_sqf_string(), "3");
    assert_eq!(
        vm.eval("diag_activeMissionFSMs").unwrap().to_sqf_string(),
        r#"[["wait.fsm","B",0]]"#
    );
    vm.eval("open = true").unwrap();
    vm.run_scheduled(FRAME);
    assert_eq!(vm.get_global("reached").to_sqf_string(), "true");
    assert_eq!(vm.eval("diag_activeMissionFSMs").unwrap().to_sqf_string(), "[]");
}

#[test]
fn a_missing_or_broken_file_returns_handle_zero() {
    let mut vm = with_file("broken.fsm", "class Nope {};");
    assert_eq!(vm.eval("execFSM \"missing.fsm\"").unwrap().to_sqf_string(), "0");
    assert_eq!(vm.eval("execFSM [\"broken.fsm\", true]").unwrap().to_sqf_string(), "0");
    assert_eq!(vm.host.errors.len(), 2, "{:?}", vm.host.errors);
}
