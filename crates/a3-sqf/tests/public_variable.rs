//! The `publicVariable` family, pinned to the original dedicated server.
//!
//! Every expectation is the recorded result of a probe in
//! `tools/oracle/probes/15_publicvariable.probes` (the probe id is in the
//! test); the handlers are `FUN_140547370` (`publicVariable`), `FUN_1405474b0`
//! (`publicVariableServer`), `FUN_1405473f0` (`publicVariableClient`) and
//! `FUN_1401809f0` (`addPublicVariableEventHandler`), all thin wrappers over
//! the network manager's `PublicVariable(name, target)`.

mod common;

use a3_sqf::{Host, PublicTarget, ScriptError, Value, Vm};

use common::{err, s};

/// A host that records the publish calls and the errors.
#[derive(Default)]
struct PvHost {
    publishes: Vec<(String, PublicTarget)>,
    errors: Vec<String>,
}

impl Host for PvHost {
    fn publish_variable(&mut self, name: &str, target: PublicTarget) {
        self.publishes.push((name.to_string(), target));
    }

    fn report_error(&mut self, error: &ScriptError) {
        self.errors.push(error.report.clone());
    }
}

fn pvm() -> Vm<PvHost> {
    Vm::new(PvHost::default())
}

fn peval(vm: &mut Vm<PvHost>, code: &str) -> Value {
    vm.eval(code)
        .unwrap_or_else(|e| panic!("{code}: {}", e.report))
}

#[test]
fn public_variable_accepts_any_name_and_returns_nothing() {
    // pub.ok.defined_global, pub.ok.mission_namespace
    for code in [
        "a3ro_pv1 = 1; publicVariable \"a3ro_pv1\"; \"ok\"",
        "missionNamespace setVariable [\"a3ro_pv2\", 1]; publicVariable \"a3ro_pv2\"; \"ok\"",
    ] {
        let mut vm = pvm();
        assert_eq!(peval(&mut vm, code).to_sqf_string(), "\"ok\"");
        assert!(vm.host.errors.is_empty(), "{:?}", vm.host.errors);
    }
    let mut vm = pvm();
    peval(&mut vm, "a3ro_pv1 = 1; publicVariable \"a3ro_pv1\"");
    assert_eq!(
        vm.host.publishes,
        vec![("a3ro_pv1".to_string(), PublicTarget::All)]
    );
    // The command's own value is the empty value (pub.ret.value).
    assert_eq!(
        s("a3ro_pv3 = 1; isNil {publicVariable \"a3ro_pv3\"}"),
        "true"
    );
}

#[test]
fn publishing_an_undefined_or_local_name_is_not_an_error() {
    // pub.ok.undefined, pub.ok.local_name: the network manager resolves the
    // name, the command does not.
    for code in [
        "publicVariable \"a3ro_pv_never_set\"; \"ok\"",
        "private _pv = 1; publicVariable \"_pv\"; \"ok\"",
    ] {
        let mut vm = pvm();
        assert_eq!(peval(&mut vm, code).to_sqf_string(), "\"ok\"");
        assert!(vm.host.errors.is_empty(), "{:?}", vm.host.errors);
        assert_eq!(vm.host.publishes.len(), 1);
    }
}

#[test]
fn an_empty_name_is_the_reserved_variable_error() {
    // pub.err.empty_name: logged, and the script goes on.
    let mut vm = pvm();
    assert_eq!(
        peval(&mut vm, "publicVariable \"\"; \"ok\"").to_sqf_string(),
        "\"ok\""
    );
    assert_eq!(vm.host.errors.len(), 1);
    assert!(
        vm.host.errors[0].contains("Reserved variable in expression"),
        "{:?}",
        vm.host.errors
    );
    // Nothing was published.
    assert!(vm.host.publishes.is_empty());
}

#[test]
fn a_wrong_argument_type_ends_the_script() {
    // pub.err.number, pub.err.array, srv.err.number
    for (code, expected) in [
        (
            "publicVariable 1",
            "publicVariable: Type Number, expected String",
        ),
        (
            "publicVariable [\"a3ro_pv1\"]",
            "publicVariable: Type Array, expected String",
        ),
        (
            "publicVariableServer 1",
            "publicVariableServer: Type Number, expected String",
        ),
        (
            "\"0\" publicVariableClient \"a3ro_pvc\"",
            "publicVariableClient: Type String, expected Number",
        ),
        (
            "0 publicVariableClient 1",
            "publicVariableClient: Type Number, expected String",
        ),
        (
            "\"a3ro_ev3\" addPublicVariableEventHandler 5",
            "addPublicVariableEventHandler: Type Number, expected Array,Code",
        ),
        (
            "1 addPublicVariableEventHandler {1}",
            "addPublicVariableEventHandler: Type Number, expected String",
        ),
    ] {
        let report = err(code);
        assert!(report.contains(expected), "{code}: {report}");
    }
    // pub.err.nil, cli.err.nil_right: a nil argument skips the command.
    assert_eq!(s("isNil {publicVariable nil}"), "true");
    assert_eq!(s("isNil {0 publicVariableClient nil}"), "true");
}

#[test]
fn public_variable_server_and_client_take_their_targets() {
    let mut vm = pvm();
    assert_eq!(
        peval(
            &mut vm,
            "a3ro_pvs = 1; publicVariableServer \"a3ro_pvs\"; \"ok\""
        )
        .to_sqf_string(),
        "\"ok\""
    );
    assert_eq!(
        vm.host.publishes,
        vec![("a3ro_pvs".to_string(), PublicTarget::Server)]
    );
    // cli.ok.zero, cli.ok.big_id, cli.ok.undefined: any client id is
    // accepted, and an unknown one is a silent no-op in the manager.
    let mut vm = pvm();
    assert_eq!(
        peval(
            &mut vm,
            "a3ro_pvc = 1; 0 publicVariableClient \"a3ro_pvc\"; 100000 publicVariableClient \"a3ro_pvc\"; 0 publicVariableClient \"a3ro_pv_never_set\"; \"ok\""
        )
        .to_sqf_string(),
        "\"ok\""
    );
    assert!(vm.host.errors.is_empty(), "{:?}", vm.host.errors);
    assert_eq!(
        vm.host.publishes,
        vec![
            ("a3ro_pvc".to_string(), PublicTarget::Client(0)),
            ("a3ro_pvc".to_string(), PublicTarget::Client(100000)),
            ("a3ro_pv_never_set".to_string(), PublicTarget::Client(0)),
        ]
    );
    // The id is rounded before the lookup (ROUND in FUN_1405473f0).
    let mut vm = pvm();
    peval(&mut vm, "0.6 publicVariableClient \"a3ro_pvc\"");
    assert_eq!(
        vm.host.publishes,
        vec![("a3ro_pvc".to_string(), PublicTarget::Client(1))]
    );
    // srv.ret.value, cli.ret.value: the empty value.
    assert_eq!(
        s("a3ro_pvs2 = 1; isNil {publicVariableServer \"a3ro_pvs2\"}"),
        "true"
    );
    assert_eq!(
        s("a3ro_pvc2 = 1; isNil {0 publicVariableClient \"a3ro_pvc2\"}"),
        "true"
    );
}

#[test]
fn the_handler_does_not_fire_on_the_publishing_machine() {
    // evh.code, evh.array_target_code, evh.this_value: the original runs the
    // handler only where the broadcast arrives.
    assert_eq!(
        s(
            "a3ro_hit = 0; \"a3ro_ev\" addPublicVariableEventHandler {a3ro_hit = a3ro_hit + 1}; a3ro_ev = 1; publicVariable \"a3ro_ev\"; a3ro_hit"
        ),
        "0"
    );
    assert_eq!(
        s(
            "a3ro_hit2 = 0; \"a3ro_ev2\" addPublicVariableEventHandler [objNull, {a3ro_hit2 = 1}]; a3ro_ev2 = 1; publicVariable \"a3ro_ev2\"; a3ro_hit2"
        ),
        "0"
    );
    assert_eq!(
        s(
            "a3ro_this = []; \"a3ro_ev8\" addPublicVariableEventHandler {a3ro_this = _this}; a3ro_ev8 = 5; publicVariable \"a3ro_ev8\"; a3ro_this"
        ),
        "[]"
    );
}

#[test]
fn a_received_broadcast_runs_the_handler_with_its_payload() {
    // The engine's payload is [varName, value, target].
    let mut vm = pvm();
    peval(
        &mut vm,
        "a3ro_seen = []; \"a3ro_ev\" addPublicVariableEventHandler {a3ro_seen = _this}; \"other\" addPublicVariableEventHandler {a3ro_seen = [\"wrong\"]}",
    );
    let ran = vm
        .public_variable_received("a3ro_ev", Value::from(5), Value::Nothing)
        .expect("no error");
    assert_eq!(ran, 1);
    assert_eq!(
        vm.get_global("a3ro_seen").to_sqf_string(),
        "[\"a3ro_ev\",5,<null>]"
    );
    // Names compare without regard to case, as variable names do.
    let ran = vm
        .public_variable_received("A3RO_EV", Value::from(6), Value::Nothing)
        .expect("no error");
    assert_eq!(ran, 1);
    assert_eq!(
        vm.get_global("a3ro_seen").to_sqf_string(),
        "[\"A3RO_EV\",6,<null>]"
    );
    // The array form only matches its target (registered for objNull above).
    let mut vm = pvm();
    peval(
        &mut vm,
        "a3ro_seen = 0; \"a3ro_ev2\" addPublicVariableEventHandler [objNull, {a3ro_seen = 1}]",
    );
    assert_eq!(
        vm.public_variable_received("a3ro_ev2", Value::from(1), Value::from(7)),
        Ok(0)
    );
    assert_eq!(vm.get_global("a3ro_seen").to_sqf_string(), "0");
}

#[test]
fn the_handler_registration_contract() {
    // evh.ret.value, evh.ret.array_form, evh.ok.undefined_name, evh.ok.twice
    assert_eq!(
        s("isNil {\"a3ro_ev4\" addPublicVariableEventHandler {1}}"),
        "true"
    );
    assert_eq!(
        s("isNil {\"a3ro_ev9\" addPublicVariableEventHandler [objNull, {1}]}"),
        "true"
    );
    assert_eq!(
        s("\"a3ro_pv_never_set\" addPublicVariableEventHandler {1}; \"ok\""),
        "\"ok\""
    );
    assert_eq!(
        s(
            "\"a3ro_ev5\" addPublicVariableEventHandler {1}; \"a3ro_ev5\" addPublicVariableEventHandler {2}; \"ok\""
        ),
        "\"ok\""
    );
    let mut vm = pvm();
    peval(
        &mut vm,
        "\"a3ro_ev5\" addPublicVariableEventHandler {1}; \"a3ro_ev5\" addPublicVariableEventHandler {2}",
    );
    // Both stay registered: there is no way to remove one.
    assert_eq!(vm.public_handlers().len(), 2);
    // evh.err.array_one_element: the target of the array form is checked
    // first, and that error is logged, not fatal. (The engine lists the
    // types as "Object, Group, Namespace"; we list them in our canonical
    // order, the wording of a type error is not part of the contract.)
    let mut vm = pvm();
    assert!(peval(&mut vm, "\"a3ro_ev6\" addPublicVariableEventHandler [{1}]").is_nil());
    assert_eq!(vm.host.errors.len(), 1);
    assert!(
        vm.host.errors[0].contains("Type Code, expected Namespace,Object,Group"),
        "{:?}",
        vm.host.errors
    );
    assert!(vm.public_handlers().is_empty());
}
