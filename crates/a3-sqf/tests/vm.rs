//! VM behaviour: expressions, scopes, control flow, errors.

mod common;

use common::{err, eval, s, vm};

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(s("1 + 2 * 3"), "7");
    assert_eq!(s("count [1] + 1"), "2");
    assert_eq!(s("-1 ^ 2"), "1");
    assert_eq!(s("- 2 ^ 2"), "4");
    assert_eq!(s("2 ^ 3 ^ 2"), "64");
    assert_eq!(s("[10, 20, 30] select 1 + 1"), "30");
    assert_eq!(s("10 mod 3"), "1");
    assert_eq!(s("7 / 2"), "3.5");
    assert_eq!(s("1 max 5 min 3"), "3");
}

#[test]
fn numbers_are_single_precision() {
    assert_eq!(s("16777216 + 1"), "1.67772e+007");
    assert_eq!(s("0.1 + 0.2"), "0.3");
    assert_eq!(s("1e6"), "1e+006");
}

#[test]
fn math_commands_use_degrees() {
    assert_eq!(s("sin 90"), "1");
    assert_eq!(s("round (cos 180)"), "-1");
    assert_eq!(s("1 atan2 1"), "45");
    assert_eq!(s("floor 2.7 + ceil 2.2 + round 2.5"), "8");
    assert_eq!(s("abs -3 + sqrt 16"), "7");
    assert_eq!(s("linearConversion [0, 10, 5, 0, 100, true]"), "50");
    assert_eq!(s("1.23456 toFixed 2"), "\"1.23\"");
}

#[test]
fn strings_compare_case_insensitively_with_eq() {
    assert_eq!(s("\"ABC\" == \"abc\""), "true");
    assert_eq!(s("\"ABC\" isEqualTo \"abc\""), "false");
    assert_eq!(s("\"a\" + \"b\""), "\"ab\"");
}

#[test]
fn arrays_are_shared_and_plus_copies() {
    assert_eq!(s("_a = [1]; _b = _a; _b pushBack 2; _a"), "[1,2]");
    assert_eq!(s("_a = [1]; _b = +_a; _b pushBack 2; _a"), "[1]");
    assert_eq!(s("_a = [1]; _b = _a + []; _b pushBack 2; _a"), "[1]");
    assert_eq!(s("[1, 2, 3, 2] - [2]"), "[1,3]");
}

#[test]
fn local_scopes_shadow_and_assign_outward() {
    assert_eq!(s("_a = 1; call { _a = 2 }; _a"), "2");
    assert_eq!(s("_a = 1; call { private _a = 2 }; _a"), "1");
    assert_eq!(s("_a = 1; call { private \"_a\"; _a = 2 }; _a"), "1");
    assert_eq!(s("call { _b = 5 }; isNil \"_b\""), "true");
    assert_eq!(s("_f = { _x * 2 }; _x = 21; call _f"), "42");
}

#[test]
fn this_is_passed_by_call() {
    assert_eq!(
        s("[1, 2] call { (_this select 0) + (_this select 1) }"),
        "3"
    );
    assert_eq!(s("_this = 5; call { _this }"), "5");
}

#[test]
fn if_then_else() {
    assert_eq!(s("if (1 > 0) then { \"y\" } else { \"n\" }"), "\"y\"");
    assert_eq!(s("if (1 < 0) then { \"y\" } else { \"n\" }"), "\"n\"");
    assert_eq!(s("typeName (if (false) then { 1 })"), "\"NOTHING\"");
    assert_eq!(s("if (true) then [{1}, {2}]"), "1");
}

#[test]
fn exit_with_leaves_the_current_scope() {
    assert_eq!(s("call { if (true) exitWith { 1 }; 2 }"), "1");
    assert_eq!(
        s("_r = 0; if (true) then { if (true) exitWith {}; _r = 1 }; _r"),
        "0"
    );
    assert_eq!(
        s("_n = 0; { if (_x > 2) exitWith {}; _n = _n + 1 } forEach [1,2,3,4]; _n"),
        "2"
    );
}

#[test]
fn loops() {
    assert_eq!(
        s("_s = 0; for \"_i\" from 1 to 10 do { _s = _s + _i }; _s"),
        "55"
    );
    assert_eq!(
        s("_s = 0; for \"_i\" from 10 to 1 step -3 do { _s = _s + _i }; _s"),
        "22"
    );
    assert_eq!(
        s("_s = 0; for [{_i = 0}, {_i < 5}, {_i = _i + 1}] do { _s = _s + _i }; _s"),
        "10"
    );
    assert_eq!(s("_i = 0; while { _i < 7 } do { _i = _i + 1 }; _i"), "7");
    assert_eq!(
        s("_r = []; { _r pushBack (_x + _forEachIndex) } forEach [10, 20]; _r"),
        "[10,21]"
    );
    assert_eq!(s("{ _x > 1 } count [1, 2, 3]"), "2");
    assert_eq!(s("[1, 2, 3] select { _x != 2 }"), "[1,3]");
    assert_eq!(s("[1, 2, 3] apply { _x * _x }"), "[1,4,9]");
    assert_eq!(s("[5, 6, 7] findIf { _x == 6 }"), "1");
    assert_eq!(s("[5, 6, 7] findIf { _x == 9 }"), "-1");
}

#[test]
fn unscheduled_while_stops_after_10000_iterations() {
    assert_eq!(s("_i = 0; while { true } do { _i = _i + 1 }; _i"), "10000");
}

#[test]
fn break_and_continue() {
    assert_eq!(
        s("_r = []; { if (_x == 3) then { break }; _r pushBack _x } forEach [1,2,3,4]; _r"),
        "[1,2]"
    );
    assert_eq!(
        s("_r = []; { if (_x == 2) then { continue }; _r pushBack _x } forEach [1,2,3]; _r"),
        "[1,3]"
    );
    assert_eq!(
        s(
            "_i = 0; _n = 0; while { _i < 10 } do { _i = _i + 1; if (_i > 3) then { break }; _n = _n + 1 }; _n"
        ),
        "3"
    );
}

#[test]
fn switch_case_default() {
    let sw = |v: &str| {
        s(&format!(
            "switch ({v}) do {{ case 1: {{ \"one\" }}; case 2; case 3: {{ \"two-three\" }}; default {{ \"other\" }} }}"
        ))
    };
    assert_eq!(sw("1"), "\"one\"");
    assert_eq!(sw("2"), "\"two-three\"");
    assert_eq!(sw("3"), "\"two-three\"");
    assert_eq!(sw("9"), "\"other\"");
}

#[test]
fn try_throw_catch() {
    assert_eq!(
        s("try { throw \"boom\"; 1 } catch { _exception + \"!\" }"),
        "\"boom!\""
    );
    assert_eq!(s("try { 5 } catch { 0 }"), "5");
    assert_eq!(
        s(
            "try { call { { if (_x == 2) then { throw _x } } forEach [1,2,3] } } catch { _exception * 10 }"
        ),
        "20"
    );
    assert_eq!(s("try { if (true) throw 7 } catch { _exception }"), "7");
}

#[test]
fn scope_name_break_out() {
    assert_eq!(
        s(
            "_r = call { scopeName \"main\"; { if (_x == 2) then { 99 breakOut \"main\" } } forEach [1,2,3]; 0 }; _r"
        ),
        "99"
    );
}

#[test]
fn lazy_and_or() {
    assert_eq!(s("false && { 1 / 0 > 0 }"), "false");
    assert_eq!(s("true || { 1 / 0 > 0 }"), "true");
    assert_eq!(s("true && { false }"), "false");
    assert_eq!(s("!true or not false"), "true");
}

#[test]
fn is_nil_forms() {
    assert_eq!(s("isNil \"undefinedThing\""), "true");
    assert_eq!(s("x1 = 1; isNil \"x1\""), "false");
    assert_eq!(s("isNil { nil }"), "true");
    assert_eq!(s("isNil { 1 }"), "false");
}

#[test]
fn global_variables_and_namespaces() {
    let mut vm = vm();
    vm.eval("myGlobal = 42; uiNamespace setVariable [\"uiVar\", 7];")
        .unwrap();
    assert_eq!(vm.get_global("MYGLOBAL").to_sqf_string(), "42");
    assert_eq!(
        vm.eval("missionNamespace getVariable \"myGlobal\"")
            .unwrap()
            .to_sqf_string(),
        "42"
    );
    assert_eq!(
        vm.eval("missionNamespace getVariable [\"nope\", -1]")
            .unwrap()
            .to_sqf_string(),
        "-1"
    );
    assert_eq!(
        vm.eval("with uiNamespace do { uiVar }")
            .unwrap()
            .to_sqf_string(),
        "7"
    );
    vm.eval("myGlobal = nil").unwrap();
    assert!(vm.get_global("myGlobal").is_nil());
}

#[test]
fn compile_final_cannot_be_overwritten() {
    let mut vm = vm();
    vm.eval("fnc = compileFinal \"1\"").unwrap();
    let e = vm.eval("fnc = {2}").unwrap_err();
    assert!(e.report.contains("final"), "{}", e.report);
}

#[test]
fn compile_and_str_of_code() {
    assert_eq!(s("call compile \"1 + 1\""), "2");
    assert_eq!(s("str {a + b}"), "\"{a + b}\"");
    assert_eq!(s("toString {a + b}"), "\"a + b\"");
    assert_eq!(s("typeName {}"), "\"CODE\"");
}

#[test]
fn type_names() {
    for (src, name) in [
        ("1", "SCALAR"),
        ("true", "BOOL"),
        ("[]", "ARRAY"),
        ("\"\"", "STRING"),
        ("missionNamespace", "NAMESPACE"),
        ("west", "SIDE"),
        ("objNull", "OBJECT"),
        ("scriptNull", "SCRIPT"),
        ("if true", "IF"),
        ("while {true}", "WHILE"),
        ("for \"_i\"", "FOR"),
        ("switch 1", "SWITCH"),
        ("with missionNamespace", "WITH"),
        ("nil", "ANY"),
    ] {
        assert_eq!(
            s(&format!("typeName ({src})")),
            format!("\"{name}\""),
            "{src}"
        );
    }
}

#[test]
fn params_with_defaults_and_types() {
    assert_eq!(
        s("[1, \"x\"] call { params [\"_a\", \"_b\", [\"_c\", 3]]; [_a, _b, _c] }"),
        "[1,\"x\",3]"
    );
    let mut vm = vm();
    let v = vm
        .eval("[\"bad\"] call { private _ok = params [[\"_a\", 5, [0]]]; [_ok, _a] }")
        .unwrap();
    assert_eq!(v.to_sqf_string(), "[false,5]");
    assert_eq!(vm.host.errors.len(), 1, "{:?}", vm.host.errors);
    assert_eq!(s("5 call { params [\"_v\"]; _v }"), "5");
    assert_eq!(s("[1, 2] param [1, 0]"), "2");
    assert_eq!(s("[1] param [3, \"d\"]"), "\"d\"");
}

#[test]
fn format_and_str() {
    assert_eq!(s("format [\"%1-%2 %1\", 1, \"a\"]"), "\"1-a 1\"");
    assert_eq!(s("format [\"%1\", [1, \"a\"]]"), "\"[1,\"\"a\"\"]\"");
    assert_eq!(eval("str \"q\"\"\"").as_str(), Some("\"q\"\"\""));
    assert_eq!(
        s("str [true, nil, west, independent]"),
        "\"[true,any,WEST,GUER]\""
    );
}

#[test]
fn array_commands() {
    assert_eq!(s("_a = [1,2,3]; _a set [5, 9]; _a"), "[1,2,3,any,any,9]");
    assert_eq!(s("_a = [1,2,3]; [_a deleteAt 1, _a]"), "[2,[1,3]]");
    assert_eq!(s("_a = [1,2,3,4]; _a deleteRange [1, 2]; _a"), "[1,4]");
    assert_eq!(s("_a = [1,4]; _a insert [1, [2,3]]; _a"), "[1,2,3,4]");
    assert_eq!(s("_a = [3,1,2]; _a sort true; _a"), "[1,2,3]");
    assert_eq!(s("_a = [\"b\",\"a\"]; _a sort false; _a"), "[\"b\",\"a\"]");
    assert_eq!(
        s("_a = [[2,\"b\"],[1,\"a\"]]; _a sort true; _a"),
        "[[1,\"a\"],[2,\"b\"]]"
    );
    assert_eq!(s("[1,2,3] select [1]"), "[2,3]");
    assert_eq!(s("[1,2,3] select [0, 2]"), "[1,2]");
    assert_eq!(s("[1,2] select true"), "2");
    assert!(err("[1,2,3] # 5").contains("Error 3 elements provided, 6 expected"));
    assert_eq!(s("[1,2,3] select -1"), "3");
    assert_eq!(s("[1,2,3] select [5, 1]"), "[]");
    assert_eq!(s("[1,2] select 2"), "any");
    assert_eq!(s("2 in [1,2]"), "true");
    assert_eq!(s("\"A\" in [\"a\"]"), "false");
    assert_eq!(s("[1,2,3,1] arrayIntersect [1,3,5]"), "[1,3]");
    assert_eq!(
        s("_a = []; [_a pushBackUnique 1, _a pushBackUnique 1]"),
        "[0,-1]"
    );
    assert_eq!(s("flatten [1,[2,[3]]]"), "[1,2,3]");
    assert_eq!(s("_a = [1,2]; _a resize 1; _a"), "[1]");
    assert_eq!(s("_a = [1,2]; reverse _a; _a"), "[2,1]");
    assert_eq!(s("[1,2,3] find 2"), "1");
}

#[test]
fn select_rounds_indices() {
    // Round half to even (cvtss2si), as the engine's select does.
    assert_eq!(s("[1,2,3] select 0.5"), "1");
    assert_eq!(s("[1,2,3] select 0.6"), "2");
    assert_eq!(s("[1,2,3] select 1.5"), "3");
    assert_eq!(s("[1,2,3] select 2.5"), "3");
}

#[test]
fn string_commands() {
    assert_eq!(s("count \"hello\""), "5");
    assert_eq!(s("toUpper \"abc\""), "\"ABC\"");
    assert_eq!(s("\"hello\" select [1, 3]"), "\"ell\"");
    assert_eq!(s("\"hello\" find \"l\""), "2");
    assert_eq!(s("\"a,b;;c\" splitString \",;\""), "[\"a\",\"b\",\"c\"]");
    assert_eq!(s("[1,\"a\"] joinString \"-\""), "\"1-a\"");
    assert_eq!(s("parseNumber \"12.5abc\""), "12.5");
    assert_eq!(s("parseNumber \"x\""), "0");
    assert_eq!(s("toArray \"AB\""), "[65,66]");
    assert_eq!(s("toString [65,66]"), "\"AB\"");
    assert_eq!(s("trim \"  x \""), "\"x\"");
    assert_eq!(s("\"ell\" in \"hello\""), "true");
}

#[test]
fn type_errors_use_engine_format() {
    let report = err("_a = 1 + \"x\"");
    assert!(
        report.contains("Error in expression <_a = 1 + \"x\">"),
        "{report}"
    );
    assert!(report.contains("Error position: <+ \"x\">"), "{report}");
    assert!(
        report.contains("Error +: Type String, expected Number"),
        "{report}"
    );
}

#[test]
fn undefined_variables_in_expressions_are_errors() {
    let report = err("_b = _undefined + 1");
    assert!(
        report.contains("Error Undefined variable in expression: _undefined"),
        "{report}"
    );
}

#[test]
fn errors_abort_the_whole_script() {
    let mut vm = vm();
    let r = vm.eval("g1 = 1; call { 1 / 0 }; g1 = 2");
    assert!(r.unwrap_err().report.contains("Zero divisor"));
    assert_eq!(vm.get_global("g1").to_sqf_string(), "1");
    assert_eq!(vm.host.errors.len(), 1);
}

#[test]
fn unimplemented_commands_parse_but_raise_a_clear_error() {
    let mut table = a3_sqf::CommandTable::builtin();
    table.declare(
        "setDamage",
        a3_sqf::Form::Binary,
        a3_sqf::Signature::binary(
            a3_sqf::TypeSet::of(a3_sqf::Type::Object),
            a3_sqf::TypeSet::NUMBER,
            a3_sqf::TypeSet::of(a3_sqf::Type::Nothing),
        ),
    );
    let mut reg = a3_sqf::Registry::new(table);
    a3_sqf::commands::register_core(&mut reg);
    let mut vm = a3_sqf::Vm::with_registry(common::TestHost::default(), std::rc::Rc::new(reg));
    let e = vm.eval("objNull setDamage 1").unwrap_err();
    assert!(
        e.report.contains("Unimplemented command: setDamage"),
        "{}",
        e.report
    );
}

#[test]
fn hosts_register_their_own_commands() {
    use a3_sqf::{Type, TypeSet, Value};
    let mut reg = a3_sqf::Registry::<common::TestHost>::with_core();
    reg.nular("missionTick", TypeSet::NUMBER, |ctx| {
        Ok(Value::Number(ctx.host.time * 10.0))
    });
    reg.binary(
        "plusChat",
        TypeSet::of(Type::String),
        TypeSet::of(Type::String),
        TypeSet::of(Type::Nothing),
        |ctx, a, b| {
            let msg = format!("{}{}", a.as_str().unwrap_or(""), b.as_str().unwrap_or(""));
            ctx.host.chat.push(msg);
            Ok(Value::Nothing)
        },
    );
    let mut vm = a3_sqf::Vm::with_registry(common::TestHost::default(), std::rc::Rc::new(reg));
    vm.host.time = 1.5;
    assert_eq!(vm.eval("missionTick + 1").unwrap().to_sqf_string(), "16");
    vm.eval("\"a\" plusChat \"b\"").unwrap();
    assert_eq!(vm.host.chat, vec!["ab"]);
}

#[test]
fn sleep_is_not_allowed_unscheduled() {
    let report = err("sleep 1");
    assert!(report.contains("Suspending not allowed"), "{report}");
    assert_eq!(s("canSuspend"), "false");
}

#[test]
fn diag_log_goes_to_the_host() {
    let mut vm = vm();
    vm.eval("diag_log \"hello\"; diag_log [1, \"a\"]").unwrap();
    assert_eq!(vm.host.log, vec!["hello", "[1,\"a\"]"]);
}

#[test]
fn eval_returns_last_statement_value() {
    assert_eq!(s("1; 2; 3"), "3");
    assert_eq!(s("typeName (call { _a = 1 })"), "\"NOTHING\"");
    assert!(eval("").to_sqf_string() == "nothing");
}

#[test]
fn number_as_assignment_target_discards_the_value() {
    // Shipped functions use `0 = [] spawn {...}` to silence a result.
    assert_eq!(s("x = 0; 0 = call { x = 2 }; x"), "2");
    assert_eq!(s("0 = 1 + 1; 7"), "7");
}
