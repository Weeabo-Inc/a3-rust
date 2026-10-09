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
    // UCRT printf: two-digit exponents, `inf`, and the NaN type for
    // non-finite numbers (server oracle).
    assert_eq!(s("str (1e30 * 1e30)"), "\"inf\"");
    assert_eq!(s("typeName (1e30 * 1e30)"), "\"NaN\"");
    assert_eq!(s("str (pi / 100000)"), "\"3.14159e-05\"");
    assert_eq!(s("str (sqrt -1)"), "\"-nan(ind)\"");
    assert_eq!(s("str 1e-39"), "\"0\"");
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

#[test]
fn number_commands_match_the_server_oracle() {
    // round is floor(x + 0.5): halves go up.
    assert_eq!(
        s("[round -0.5, round -1.5, round -2.5, round 2.5, round 0.49999997]"),
        "[0,-1,-2,3,1]"
    );
    // toFixed rounds ties to even and prints the exact value.
    assert_eq!(
        s("[0.5 toFixed 0, 2.5 toFixed 0, 0.125 toFixed 2, 1 toFixed 25, 1 toFixed -1]"),
        "[\"0\",\"2\",\"0.12\",\"1.00000000000000000000\",\"1\"]"
    );
    // A degenerate source range gives minTo.
    assert_eq!(
        s("[linearConversion [5,5,5,0,1], linearConversion [5,5,7,2,1,true]]"),
        "[0,2]"
    );
    // min/max with NaN return the right operand.
    assert_eq!(
        s("[str ((sqrt -1) max 3), str (3 max (sqrt -1)), str ((sqrt -1) min 3)]"),
        "[\"3\",\"-nan(ind)\",\"3\"]"
    );
    // parseNumber is atof.
    assert_eq!(
        s(
            "[parseNumber \"0x10\", parseNumber \"  12abc\", parseNumber \"-.5e1\", str parseNumber \"inf\"]"
        ),
        "[16,12,-5,\"inf\"]"
    );
    assert_eq!(
        s("[typeName (parseNumber \"1e40\"), 1e39 isEqualType 0]"),
        "[\"NaN\",false]"
    );
    // Subnormal literals and results are zero (flush-to-zero).
    assert_eq!(
        s("[1e-39 == 0, str (1e-30 * 1e-10), str -1e-39]"),
        "[true,\"0\",\"-0\"]"
    );
    // Seeded random hashes the truncated seed.
    assert_eq!(
        s("[12345 random 100, 1.5 random 1, 0 random 1, 5 random [1, 2]]"),
        "[84.4356,0.348552,0.573565,0.31312]"
    );
}
