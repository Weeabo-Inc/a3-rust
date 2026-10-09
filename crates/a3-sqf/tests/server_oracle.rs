//! SQF semantics confirmed against the original server with the oracle
//! (`tools/oracle/oracle.py`, `arma3server_x64.exe` 2.22.0.154103).
//!
//! Every expectation here is the recorded result of a probe in
//! `tools/oracle/probes/` (the probe id is in the test) or of a scratch
//! probe run against the same server while these fixes were written. They
//! are the regression tests for the fidelity issues #270-#276, #279, #280.

mod common;

use common::{s, vm};

/// Issues #274 and #276: `nil` and the empty value.
#[test]
fn nil_and_the_empty_value() {
    // `nil` is not a variable read; a command given it is skipped.
    assert_eq!(s("typeName nil"), "any");
    assert_eq!(s("str nil"), "any");
    assert!(common::eval("str nil").is_nil());
    assert!(common::eval("typeName (call {})").is_nil());
    assert_eq!(s("isNil {nil isEqualTo nil}"), "true");
    assert_eq!(s("isNil {nil}"), "true");
    // An array element that is nil prints `any`; the empty value (array
    // padding, a missing hash map key) prints `<null>`.
    assert_eq!(s("[nil, 1]"), "[any,1]");
    assert_eq!(s("format [\"%1\", nil]"), "\"any\"");
    assert_eq!(s("format [\"%1\", call {}]"), "\"<null>\"");
    assert_eq!(
        s("_a = [1, 2, 3]; _a set [5, 9]; _a"),
        "[1,2,3,<null>,<null>,9]"
    );
    assert_eq!(s("_a = [1]; _a resize 3; _a"), "[1,<null>,<null>]");
    assert_eq!(s("_a = [1, 2, 3]; [_a deleteAt 5, _a]"), "[<null>,[1,2,3]]");
    assert_eq!(
        s("_a = [1, 2, 3]; [_a deleteAt -1, _a]"),
        "[<null>,[1,2,3]]"
    );
    assert_eq!(
        s("_h = [\"a\", \"b\", \"c\"] createHashMapFromArray [1]; [count _h, _h get \"c\"]"),
        "[3,<null>]"
    );
    assert_eq!(
        s(
            "_h = createHashMapFromArray [[\"a\", 1]]; [_h deleteAt \"a\", count _h, _h deleteAt \"a\"]"
        ),
        "[1,0,<null>]"
    );
    assert_eq!(s("isNil {createHashMap get \"x\"}"), "true");
    // `if` without a matching branch gives the empty value.
    assert!(common::eval("if (false) then {1}").is_nil());
    assert_eq!(s("isNil {if (false) then {1}}"), "true");
}

/// Issue #271: an error is logged and the script goes on; the expression
/// yields nil (or `inf` for a zero divisor).
#[test]
fn errors_are_logged_and_the_script_goes_on() {
    // `1/0` (num.div_by_zero, ctl.error_zero_div) is inf and logs.
    let mut vm1 = vm();
    let v = vm1.eval("1 / 0").unwrap();
    assert_eq!(v.ty().type_name(), "NaN");
    assert_eq!(v.to_sqf_string(), "inf");
    assert_eq!(vm1.host.errors.len(), 1);
    assert!(vm1.host.errors[0].contains("Zero divisor"));
    // `> 5 % 0` (num.neg_mod_zero) is 0.
    assert_eq!(s("5 % 0"), "0");
    // `finite (1/0)` (num.finite_inf) is false.
    assert_eq!(s("finite (1/0)"), "false");
    // Out of range select/# (arr.select_out_of_range_far, arr.hash_out_of_range).
    assert_eq!(s("isNil {[1, 2, 3] select 4}"), "true");
    assert_eq!(s("isNil {[10, 20, 30] # 5}"), "true");
    // An undefined variable (ctl.error_undefined_var) reads as nil.
    assert_eq!(s("isNil {a3ro_undefined + 1}"), "true");
    // The class body runs to its end after an error (ctl.params_bad_name).
    assert_eq!(s("[1] call {params [\"a\"]; 1}"), "1");
    // A body that errors gives nil per element (arr.apply_index).
    assert_eq!(s("[5, 6] apply {_forEachIndex}"), "[any,any]");
    assert_eq!(s("[1, 2, 3] select {_forEachIndex > 0}"), "[]");
    // A non-boolean condition is not counted (ctl.count_code_bool_required).
    assert_eq!(s("{1} count [1, 2]"), "0");
    // A regexp that does not compile is false (rex.bad_pattern).
    assert_eq!(s("\"abc\" regexMatch \"(\""), "false");
    // A hash map key of a bad type leaves the map empty (hash.object_key).
    assert_eq!(s("_h = createHashMap; _h set [objNull, 1]; count _h"), "0");
    // `compile` of broken text logs and gives the empty value
    // (ctl.error_missing_semicolon).
    assert_eq!(s("isNil {call compile \"1 2\"}"), "true");
    // An error inside `isNil {...}` ends only the block
    // (ctl.error_continue_after).
    assert_eq!(s("_r = 1; isNil {_r = 1 + \"x\"}; _r"), "1");
    // Only the first error of a script is logged.
    let mut vm2 = vm();
    vm2.eval("1 / 0; 5 % 0; 7 % 0").unwrap();
    assert_eq!(vm2.host.errors.len(), 1);
}

/// Issue #276: `format` and `parseSimpleArray`.
#[test]
fn format_percent_and_parse_simple_array() {
    assert_eq!(s("format [\"100%\"]"), "\"100\"");
    assert_eq!(s("format [\"%1%%\", 5]"), "\"5%\"");
    assert_eq!(s("format [\"a%bc\"]"), "\"abc\"");
    assert_eq!(s("format [\"%\"]"), "\"\"");
    assert_eq!(s("format [\"%%\"]"), "\"%\"");
    assert_eq!(s("format [\"%1%%%2\", 5, 6]"), "\"5%6\"");
    assert_eq!(s("format [\"%2\", 5]"), "\"\"");
    // `parseSimpleArray` returns what it parsed so far.
    assert_eq!(s("parseSimpleArray \"[1, b]\""), "[1]");
    assert_eq!(s("parseSimpleArray \"1\""), "[]");
    assert_eq!(s("parseSimpleArray \"[1, 2\""), "[1,2]");
    assert_eq!(s("parseSimpleArray \"[1, 2,]\""), "[1,2]");
    assert_eq!(s("parseSimpleArray \"\""), "[]");
    let mut vm1 = vm();
    vm1.eval("parseSimpleArray \"[1, b]\"").unwrap();
    assert!(
        vm1.host.errors[0].contains("parseSimpleArray format error"),
        "{:?}",
        vm1.host.errors
    );
    // A namespace prints `Namespace`.
    assert_eq!(s("str missionNamespace"), "\"Namespace\"");
    assert_eq!(s("str uiNamespace"), "\"Namespace\"");
    assert_eq!(s("str profileNamespace"), "\"Namespace\"");
    // The plain text of structured text drops every tag, `<br/>` included.
    assert_eq!(s("str lineBreak"), "\"\"");
    assert_eq!(s("str parseText \"a<br/>b\""), "\"ab\"");
}

/// Issue #275: array, string and hash map edges.
#[test]
fn array_string_and_hash_edges() {
    // A negative select start is empty; `set` counts from the end and
    // further out is the Zero divisor error.
    assert_eq!(s("\"hello\" select [-2, 2]"), "\"\"");
    assert_eq!(s("\"hello\" select [-1, 2]"), "\"\"");
    assert_eq!(s("_a = [1, 2, 3]; _a set [-1, 9]; _a"), "[1,2,9]");
    assert_eq!(s("_a = [1, 2, 3]; _a set [-2, 9]; _a"), "[1,9,3]");
    assert_eq!(s("_a = [1, 2, 3]; _a set [-4, 9]; _a"), "[1,2,3]");
    // `sort` compares strings without regard to ASCII case, uppercase
    // first when only the case differs.
    assert_eq!(
        s("_a = [\"b\", \"A\", \"a\", \"B\"]; _a sort true; _a"),
        "[\"A\",\"a\",\"B\",\"b\"]"
    );
    assert_eq!(
        s("_a = [\"é\", \"e\", \"f\", \"E\"]; _a sort true; _a"),
        "[\"é\",\"E\",\"e\",\"f\"]"
    );
    assert_eq!(s("_a = [2, 1, 3]; _a sort false; _a"), "[3,2,1]");
    // `vectorAdd` keeps the size of the longer vector.
    assert_eq!(s("[1, 2] vectorAdd [3, 4]"), "[4,6]");
    assert_eq!(s("[1, 2, 3] vectorAdd [3, 4]"), "[4,6,3]");
    assert_eq!(s("[1, 2] vectorAdd [3, 4, 5]"), "[4,6,5]");
    // `set` returns whether an existing value was overwritten.
    assert_eq!(
        s(
            "_h = createHashMap; [_h set [\"a\", 1], _h set [\"a\", 2], _h set [\"b\", 3], count _h]"
        ),
        "[false,true,false,2]"
    );
    assert_eq!(
        s("_h = createHashMap; _h set [\"a\", 1]; [_h set [\"a\", 2, true], _h get \"a\"]"),
        "[false,1]"
    );
    assert_eq!(
        s("_h = createHashMap; [_h set [\"b\", 2, true], _h get \"b\"]"),
        "[false,2]"
    );
    // Null objects compare unequal, a null config equal to itself.
    assert_eq!(s("objNull == objNull"), "false");
    assert_eq!(s("grpNull == grpNull"), "false");
    assert_eq!(s("controlNull == controlNull"), "false");
    assert_eq!(s("configNull == configNull"), "true");
    assert_eq!(s("objNull isEqualTo objNull"), "true");
}

/// Issue #280: `isNil {code}` runs unscheduled.
#[test]
fn isnil_code_runs_unscheduled() {
    assert_eq!(s("_r = false; isNil {_r = canSuspend}; _r"), "false");
}

/// `private` declares a variable but does not clear one that is already in
/// the scope, so `private ["_this"]` keeps the caller's `_this` (the bug
/// behind part of #281).
#[test]
fn private_does_not_clear_an_existing_variable() {
    assert_eq!(s("[1] call {private [\"_this\"]; _this}"), "[1]");
    assert_eq!(
        s("[1] call {private [\"_this\"]; isNil \"_this\"}"),
        "false"
    );
    // A declared but uninitialised variable reads as an error (logged, and
    // the read is nil), as before.
    let mut vm1 = vm();
    assert!(vm1.eval("private _x; _x").unwrap().is_nil());
    assert!(
        vm1.host.errors[0].contains("Undefined variable in expression: _x"),
        "{:?}",
        vm1.host.errors
    );
}
