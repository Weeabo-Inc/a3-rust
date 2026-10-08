//! The scheduled environment: spawn, sleep, waitUntil, terminate, execVM.

mod common;

use std::time::Duration;

use common::vm;

const FRAME: Duration = Duration::from_millis(100);

#[test]
fn spawned_scripts_run_on_the_next_frame() {
    let mut vm = vm();
    vm.eval("[1, 2] spawn { done1 = (_this select 0) + (_this select 1); canSus = canSuspend }")
        .unwrap();
    assert!(vm.get_global("done1").is_nil());
    vm.run_scheduled(FRAME);
    assert_eq!(vm.get_global("done1").to_sqf_string(), "3");
    assert_eq!(vm.get_global("canSus").to_sqf_string(), "true");
    assert_eq!(vm.scheduled_count(), 0);
}

#[test]
fn sleep_waits_for_mission_time() {
    let mut vm = vm();
    vm.eval("0 spawn { stage = 1; sleep 5; stage = 2 }")
        .unwrap();
    vm.run_scheduled(FRAME);
    assert_eq!(vm.get_global("stage").to_sqf_string(), "1");
    vm.host.time = 4.9;
    vm.run_scheduled(FRAME);
    assert_eq!(vm.get_global("stage").to_sqf_string(), "1");
    vm.host.time = 5.0;
    vm.run_scheduled(FRAME);
    assert_eq!(vm.get_global("stage").to_sqf_string(), "2");
}

#[test]
fn suspension_inside_nested_loops_and_calls_resumes_in_place() {
    let mut vm = vm();
    vm.eval(
        "0 spawn { private _out = []; call { { _out pushBack _x; uiSleep 1 } forEach [1,2,3] }; result = _out }",
    )
    .unwrap();
    for t in 0..5 {
        vm.host.tick = t as f32;
        vm.run_scheduled(FRAME);
    }
    assert_eq!(vm.get_global("result").to_sqf_string(), "[1,2,3]");
}

#[test]
fn wait_until_rechecks_each_frame() {
    let mut vm = vm();
    vm.eval("0 spawn { waitUntil { !isNil \"go\" }; finished = go * 2 }")
        .unwrap();
    vm.run_scheduled(FRAME);
    vm.run_scheduled(FRAME);
    assert!(vm.get_global("finished").is_nil());
    vm.set_global("go", 21.into());
    vm.run_scheduled(FRAME);
    assert_eq!(vm.get_global("finished").to_sqf_string(), "42");
}

#[test]
fn script_done_and_terminate() {
    let mut vm = vm();
    vm.eval("h1 = 0 spawn { sleep 100 }; h2 = 0 spawn { }")
        .unwrap();
    vm.run_scheduled(FRAME);
    assert_eq!(
        vm.eval("[scriptDone h1, scriptDone h2]")
            .unwrap()
            .to_sqf_string(),
        "[false,true]"
    );
    vm.eval("terminate h1").unwrap();
    vm.run_scheduled(FRAME);
    assert_eq!(vm.eval("scriptDone h1").unwrap().to_sqf_string(), "true");
    assert_eq!(vm.scheduled_count(), 0);
}

#[test]
fn scripts_can_terminate_themselves() {
    let mut vm = vm();
    vm.eval("0 spawn { a1 = 1; terminate _thisScript; a1 = 2 }")
        .unwrap();
    vm.run_scheduled(FRAME);
    assert_eq!(vm.get_global("a1").to_sqf_string(), "1");
}

#[test]
fn scheduled_while_has_no_iteration_cap() {
    let mut vm = vm();
    vm.eval("0 spawn { _i = 0; while { _i < 20000 } do { _i = _i + 1 }; count1 = _i }")
        .unwrap();
    vm.run_until_idle(100, |_| {});
    assert_eq!(vm.get_global("count1").to_sqf_string(), "20000");
}

#[test]
fn budget_exhaustion_resumes_next_frame() {
    let mut vm = vm();
    vm.eval("0 spawn { _i = 0; while { _i < 200000 } do { _i = _i + 1 }; big = _i }")
        .unwrap();
    let mut frames = 0;
    while vm.scheduled_count() > 0 && frames < 100_000 {
        vm.run_scheduled(Duration::from_micros(200));
        frames += 1;
    }
    assert!(frames > 1, "expected the loop to span frames");
    assert_eq!(vm.get_global("big").to_sqf_string(), "200000");
}

#[test]
fn exec_vm_loads_and_spawns_a_file() {
    let mut vm = vm();
    vm.host
        .files
        .insert("scripts\\init.sqf".into(), "loaded = _this + 1".into());
    vm.eval("41 execVM \"scripts\\init.sqf\"").unwrap();
    vm.run_scheduled(FRAME);
    assert_eq!(vm.get_global("loaded").to_sqf_string(), "42");
}

#[test]
fn errors_in_scheduled_scripts_are_reported() {
    let mut vm = vm();
    vm.eval("0 spawn { x = 1 + \"a\" }").unwrap();
    let report = vm.run_scheduled(FRAME);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(vm.host.errors.len(), 1);
}
