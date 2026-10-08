//! `#ifdef` / `#ifndef` / `#if` / `#else` / `#endif` and the output line structure.
//!
//! "engine" cases: observed results from <https://github.com/Krzmbrzl/ArmaPreprocessorTestCases>
//! (`if(n)def` folder) and the BI wiki.

mod common;
use a3_preproc::{ErrorKind, MemoryResolver, Options, Preprocessor};
use common::{pp, pp_with, try_pp};

#[test]
fn ifdef_true_branch_drops_else_branch_and_indentation() {
    // engine: Test03
    let src = "#define SWITCHER\n\n#ifdef SWITCHER\n\tI am so happy to see you\n#else\n\tSadly I could not make it\n#endif\n\nhint \"Is there still hope?\"";
    assert_eq!(
        pp(src),
        "\n\n\nI am so happy to see you\n\n\nhint \"Is there still hope?\""
    );
}

#[test]
fn skipped_lines_produce_no_line_breaks() {
    // engine: Test04
    let src = "#ifdef MAMA\n\nThat is interesting\n\nyes indeed\n\n#endif\n\n\n#ifdef PAPA\n\nNo way you are\nhere\n\n\n\n\n#else\n\n\nhint \"Papa is not here anymore\"\n\n\n#endif\n";
    assert_eq!(
        pp(src),
        "\n\n\n\n\n\nhint \"Papa is not here anymore\"\n\n\n\n"
    );
}

#[test]
fn fully_skipped_file_is_empty() {
    // engine: Test05
    assert_eq!(pp("#ifdef MIAU\n\n\nSome conentent\n\n\n#endif"), "");
}

#[test]
fn ifndef_undefined_is_taken() {
    // engine: Test06
    assert_eq!(
        pp("#ifndef MIAU\n\nSome content\n\n#endif"),
        "\n\nSome content\n\n"
    );
}

#[test]
fn else_branch_taken() {
    // engine: Test07
    assert_eq!(
        pp("#ifdef MIAU\n\nSome content\n\n#else\nOther content\n#endif"),
        "\nOther content\n"
    );
}

#[test]
fn text_after_conditional_directives_is_code() {
    // engine: Test09
    let src = "#define BLUBB Somehting in the water\n\n#ifdef BLUBB   \nI am here!\n#else No\nway\n#endif bla";
    assert_eq!(pp(src), "\n\n   \nI am here!\nbla");
}

#[test]
fn conditionals_nest() {
    let src = "#define A\n#ifdef A\n#ifdef B\nab\n#else\na\n#endif\n#endif\nend";
    assert_eq!(pp(src), "\n\n\na\n\n\nend");
}

#[test]
fn skipped_branch_ignores_other_directives() {
    let src = "#ifdef NOPE\n#define X 1\n#include \"missing.hpp\"\n#bogus\n#endif\nX";
    assert_eq!(pp(src), "\nX");
}

#[test]
fn define_inside_skipped_branch_consumes_continuations() {
    let src = "#ifdef NOPE\n#define X 1 \\\n#endif\n#endif\ny";
    assert_eq!(pp(src), "\ny");
}

#[test]
fn builtins_count_as_defined() {
    assert_eq!(pp("#ifdef __ARMA3__\nyes\n#endif"), "\nyes\n");
    assert_eq!(pp("#ifndef __A3_DEBUG__\nrelease\n#endif"), "\nrelease\n");
}

#[test]
fn if_compares_numbers() {
    // wiki: #if __GAME_VER_MAJ__ != 2
    assert_eq!(
        pp("#if __GAME_VER_MAJ__ != 2\nold\n#else\nnew\n#endif"),
        "\nnew\n"
    );
    assert_eq!(pp("#if __GAME_VER_MIN__ >= 16\nfixed\n#endif"), "\nfixed\n");
    assert_eq!(pp("#define V 3\n#if V < 4\nlow\n#endif"), "\n\nlow\n");
    assert_eq!(pp("#define V 3\n#if V <= 2\nlow\n#endif"), "\n");
    assert_eq!(pp("#define V 3\n#if V > 2\nhigh\n#endif"), "\n\nhigh\n");
    assert_eq!(pp("#define V 3\n#if V == 3.0\neq\n#endif"), "\n\neq\n");
}

#[test]
fn if_single_value_is_true_when_non_zero() {
    assert_eq!(pp("#define ON 1\n#if ON\non\n#endif"), "\n\non\n");
    assert_eq!(pp("#define OFF 0\n#if OFF\non\n#endif"), "\n");
}

#[test]
fn if_undefined_is_false() {
    // wiki: `#if __A3_EXPERIMENTAL__` on a stable build
    assert_eq!(pp("#if __A3_EXPERIMENTAL__\nexp\n#endif"), "");
    assert_eq!(pp("#if NOT_DEFINED\nx\n#else\ny\n#endif"), "\ny\n");
}

#[test]
fn runtime_flags_come_from_options() {
    let options = Options {
        debug: true,
        experimental: true,
        ..Options::default()
    };
    let out = pp_with(
        options,
        "#if __A3_DEBUG__\ndebug\n#endif\n#ifdef __A3_EXPERIMENTAL__\nexp\n#endif",
    )
    .unwrap();
    assert_eq!(out.text, "\ndebug\n\n\nexp\n");
}

#[test]
fn if_compares_text_for_equality() {
    assert_eq!(
        pp("#define MODE dev\n#if MODE == dev\nd\n#endif"),
        "\n\nd\n"
    );
    assert_eq!(pp("#define MODE dev\n#if MODE != dev\nd\n#endif"), "\n");
}

#[test]
fn if_ordering_on_text_is_an_error() {
    let err = try_pp("#if abc < def\n#endif").unwrap_err();
    assert!(matches!(err.kind, ErrorKind::InvalidCondition(_)));
}

#[test]
fn has_include_checks_absolute_paths() {
    let files = MemoryResolver::new()
        .with_file("\\z\\ace\\addons\\main\\script_component.hpp", "")
        .with_file("\\test\\local.hpp", "");
    let mut pp = Preprocessor::new(&files);
    let src = "#if __has_include(\"\\z\\ace\\addons\\main\\script_component.hpp\")\nace\n#else\nnoace\n#endif\n#if __has_include(\"\\nope.hpp\")\nbad\n#endif\n#if __has_include(\"local.hpp\")\nrelative\n#endif";
    let out = pp.preprocess_str("\\test\\main.sqf", src).unwrap();
    // Relative paths silently evaluate to false (wiki).
    assert_eq!(out.text, "\nace\n\n\n");
}

#[test]
fn elif_is_unknown_in_2_22() {
    let err = try_pp("#if 1\n#elif 0\n#endif").unwrap_err();
    assert_eq!(err.kind, ErrorKind::UnknownDirective("elif".into()));
    assert_eq!(err.line, 2);
}

#[test]
fn unknown_directive_is_an_error() {
    // engine error 7
    let err = try_pp("#defien X").unwrap_err();
    assert_eq!(err.kind, ErrorKind::UnknownDirective("defien".into()));
}

#[test]
fn directives_are_case_sensitive() {
    let err = try_pp("#DEFINE X").unwrap_err();
    assert_eq!(err.kind, ErrorKind::UnknownDirective("DEFINE".into()));
}

#[test]
fn endif_without_if_is_an_error() {
    // engine error 6
    let err = try_pp("#endif").unwrap_err();
    assert_eq!(err.kind, ErrorKind::UnmatchedConditional("endif"));
    let err = try_pp("#else").unwrap_err();
    assert_eq!(err.kind, ErrorKind::UnmatchedConditional("else"));
}

#[test]
fn missing_endif_is_an_error() {
    let err = try_pp("x\n#ifdef A\ny").unwrap_err();
    assert_eq!(
        err.kind,
        ErrorKind::UnterminatedConditional { opened_at: 2 }
    );
}

#[test]
fn second_else_is_an_error() {
    let err = try_pp("#ifdef A\n#else\n#else\n#endif").unwrap_err();
    assert_eq!(err.kind, ErrorKind::DuplicateElse);
}

#[test]
fn pragma_is_ignored() {
    assert_eq!(pp("#pragma hemtt suppress pw3_padded_arg\nx"), "\nx");
    assert_eq!(pp("#pragma once\nx"), "\nx");
}

#[test]
fn indented_directive_is_recognised() {
    assert_eq!(pp("  #define X 1\n\tX"), "\n1");
}

#[test]
fn indentation_is_removed_from_every_line() {
    // engine: Test01 (a line of tabs becomes empty), Test03
    assert_eq!(pp("a\n\t\t\t\t\n    b\n\t c  "), "a\n\nb\nc  ");
}

#[test]
fn multi_line_string_keeps_inner_indentation() {
    assert_eq!(pp("s = \"a\n    b\";"), "s = \"a\n    b\";");
}

#[test]
fn comments_are_removed() {
    assert_eq!(
        pp("myArray = { \"apple\"/*, \"banana\"*/, \"pear\" }; // c\n// line"),
        "myArray = { \"apple\", \"pear\" }; \n"
    );
}

#[test]
fn comment_markers_in_strings_are_kept() {
    assert_eq!(pp("x = \"http://a /* b */\";"), "x = \"http://a /* b */\";");
}

#[test]
fn crlf_input_gives_lf_output() {
    assert_eq!(pp("#define A 1\r\nA\r\nb\r\n"), "\n1\nb\n");
}

#[test]
fn bom_is_dropped() {
    assert_eq!(pp("\u{feff}#define A 1\nA"), "\n1");
}
