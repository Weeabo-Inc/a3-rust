//! Engine semantics confirmed against the binary and the community wiki
//! (`docs/re/sqf-semantics.md`).

mod common;

use common::{err, s, vm};

#[test]
fn reading_an_undefined_variable_is_an_error() {
    let report = err("_a = _undefined");
    assert!(
        report.contains("Error Undefined variable in expression: _undefined"),
        "{report}"
    );
    assert!(err("_x = nil; _y = _x").contains("Undefined variable in expression: _x"));
    assert!(err("hint str someGlobal").contains("Undefined variable in expression: someglobal"));
}

#[test]
fn is_nil_code_reads_undefined_variables_without_error() {
    assert_eq!(s("isNil { _undefined }"), "true");
    assert_eq!(s("isNil { call { someGlobal } }"), "true");
    assert_eq!(s("x1 = 1; isNil { x1 }"), "false");
}

#[test]
fn commands_skip_nil_arguments_and_return_nil() {
    // `nil` itself is not a variable read: the command is skipped.
    assert_eq!(s("isNil { nil + 1 }"), "true");
    assert_eq!(s("isNil { count nil }"), "true");
    // Commands that accept anything still see nil.
    assert_eq!(s("str nil"), "\"any\"");
    assert_eq!(s("typeName nil"), "\"ANY\"");
}

#[test]
fn nothing_and_numbers_print_like_the_engine() {
    assert_eq!(s("str (if false then {1})"), "\"nothing\"");
    assert_eq!(s("str (1e30 * 1e30)"), "\"1.#INF\"");
    assert_eq!(s("typeName (1e30 * 1e30)"), "\"SCALAR\"");
    assert_eq!(s("str (pi / 100000)"), "\"3.14159e-005\"");
}

#[test]
fn switch_without_a_match_returns_true() {
    assert_eq!(s("switch (5) do { case 1: { \"one\" }; }"), "true");
}

#[test]
fn for_loop_variable_can_be_moved_by_the_body() {
    assert_eq!(
        s("_r = []; for \"_i\" from 0 to 5 do { _r pushBack _i; _i = _i + 1 }; _r"),
        "[0,2,4]"
    );
    assert_eq!(s("_i = 100; for \"_i\" from 0 to 4 do {}; _i"), "100");
}

#[test]
fn for_array_form_has_a_loop_scope() {
    // From the wiki's `for` page.
    assert_eq!(
        s("_i = 100; for [{ _i = 0 }, { _i < 5 }, { _i = _i + 1 }] do {}; _i"),
        "5"
    );
    assert_eq!(
        s("_i = 100; for [{ private _i = 0 }, { _i < 5 }, { _i = _i + 1 }] do {}; _i"),
        "100"
    );
    assert_eq!(
        s("_n = 0; for [{ private _i = 0 }, { _i < 5 }, { _i = _i + 1 }] do { _n = _n + _i }; _n"),
        "10"
    );
    // The body can move the loop variable.
    assert_eq!(
        s(
            "_r = []; for [{ private _i = 0 }, { _i < 6 }, { _i = _i + 1 }] do { _r pushBack _i; _i = _i + 1 }; _r"
        ),
        "[0,2,4]"
    );
}

#[test]
fn strings_are_bytes_unless_force_unicode() {
    // From the wiki's `forceUnicode` page.
    let mut vm = vm();
    let v = vm
        .eval(
            "private _s = \"привет\"; private _r = [count _s];
             call { _r pushBack count _s; forceUnicode 0; _r pushBack count _s };
             _r pushBack count _s;
             forceUnicode -1;
             call { _r pushBack count _s; forceUnicode 1; _r pushBack count _s; _r pushBack count _s };
             _r",
        )
        .unwrap();
    assert_eq!(v.to_sqf_string(), "[12,12,6,6,12,6,12]");
    assert_eq!(s("\"aéb\" find \"b\""), "3");
    assert_eq!(s("forceUnicode 1; \"aéb\" find \"b\""), "2");
    assert_eq!(s("forceUnicode 0; \"aéb\" select [1, 1]"), "\"é\"");
}
