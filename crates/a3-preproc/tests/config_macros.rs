//! `__EXEC` / `__EVAL` and the built-in evaluator.

mod common;
use a3_preproc::{
    ErrorKind, EvalValue, Evaluator, MemoryResolver, Options, Preprocessor, SimpleEvaluator,
};
use common::{pp, pp_with};

fn config(source: &str) -> String {
    pp_with(Options::config(), source).unwrap().text
}

#[test]
fn exec_assigns_and_eval_reads() {
    // wiki: __EXEC(cat = 5 + 1;) __EXEC(lev = cat - 2;)
    assert_eq!(
        config("__EXEC(cat = 5 + 1;)\n__EXEC(lev = cat - 2;)\nx = __EVAL(cat * lev);"),
        "\n\nx = 24;"
    );
}

#[test]
fn eval_supports_nested_parentheses() {
    assert_eq!(config("w = __EVAL(1 - (5 * ((1 / (2)) * 4)));"), "w = -9;");
}

#[test]
fn exec_ends_at_first_parenthesis() {
    // wiki: __EXEC(a = (1+2);) is an error
    let err = pp_with(Options::config(), "__EXEC(a = (1+2);)").unwrap_err();
    assert!(matches!(
        err.kind,
        ErrorKind::Evaluation {
            macro_name: "__EXEC",
            ..
        }
    ));
}

#[test]
fn eval_string_result_is_quoted() {
    assert_eq!(config("t = __EVAL(\"a\" + 'b');"), "t = \"ab\";");
    assert_eq!(
        config("t = __EVAL(\"say \"\"hi\"\"\");"),
        "t = \"say \"\"hi\"\"\";"
    );
}

#[test]
fn eval_boolean_becomes_string() {
    // wiki: Boolean results come out as "true"/"false"
    assert_eq!(config("b = __EVAL(1 < 2);"), "b = \"true\";");
}

#[test]
fn eval_fraction_uses_float_precision() {
    assert_eq!(config("x = __EVAL(1/4);"), "x = 0.25;");
    assert_eq!(config("x = __EVAL(1/3);"), "x = 0.33333334;");
}

#[test]
fn macros_expand_before_evaluation() {
    let src = "#define BASE 1000\n#define NEXT(n) __EVAL(BASE + n)\nidc = NEXT(5);";
    assert_eq!(config(src), "\n\nidc = 1005;");
}

#[test]
fn exec_in_macro_runs_in_text_order() {
    // wiki: DRAWBUTTON-style counters
    let src =
        "__EXEC(idc = 0)\n#define NEXT __EXEC(idc = idc + 4) __EVAL(idc)\na = NEXT;\nb = NEXT;";
    assert_eq!(config(src), "\n\na =  4;\nb =  8;");
}

#[test]
fn multi_line_exec_with_continuations() {
    // Shipped \a3\3den\UI\macroExecs.inc writes a whole __EXEC across `\`-continued lines.
    let src = "__EXEC(\\\n\t_a = 2;\\\n\t_b = _a * 3;\\\n)\nx = __EVAL(_b);";
    assert_eq!(config(src), "\n\n\n\nx = 6;");
}

#[test]
fn script_mode_leaves_them_alone() {
    assert_eq!(pp("x = __EVAL(1+1);"), "x = __EVAL(1+1);");
}

#[test]
fn inside_strings_they_are_text() {
    assert_eq!(config("s = \"__EVAL(1)\";"), "s = \"__EVAL(1)\";");
}

#[test]
fn unsupported_command_is_reported_with_location() {
    let err = pp_with(Options::config(), "a = 1;\nw = __EVAL(safeZoneW / 2);").unwrap_err();
    assert_eq!(err.line, 2);
    assert!(matches!(
        err.kind,
        ErrorKind::Evaluation {
            macro_name: "__EVAL",
            ..
        }
    ));
}

#[test]
fn unterminated_eval_is_an_error() {
    let err = pp_with(Options::config(), "w = __EVAL((1);").unwrap_err();
    assert_eq!(err.kind, ErrorKind::UnterminatedEvaluation("__EVAL"));
}

#[test]
fn custom_evaluator_plugs_in() {
    struct Fixed;
    impl Evaluator for Fixed {
        fn exec(&mut self, _code: &str) -> Result<(), String> {
            Ok(())
        }
        fn eval(&mut self, expression: &str) -> Result<EvalValue, String> {
            Ok(EvalValue::String(format!("<{expression}>")))
        }
    }
    let files = MemoryResolver::new();
    let out = Preprocessor::new(&files)
        .with_options(Options::config())
        .with_evaluator(Fixed)
        .preprocess_str("\\config.cpp", "x = __EVAL(safeZoneW);")
        .unwrap();
    assert_eq!(out.text, "x = \"<safeZoneW>\";");
}

// --- SimpleEvaluator ---------------------------------------------------------------------

fn eval(expr: &str) -> EvalValue {
    SimpleEvaluator::new().eval(expr).unwrap()
}

#[test]
fn evaluator_arithmetic_precedence() {
    assert_eq!(eval("1 + 2 * 3"), EvalValue::Number(7.0));
    assert_eq!(eval("2 ^ 3 * 2"), EvalValue::Number(16.0));
    assert_eq!(eval("10 - 4 - 3"), EvalValue::Number(3.0));
    assert_eq!(eval("7 % 4"), EvalValue::Number(3.0));
    assert_eq!(eval("7 mod 4 max 1"), EvalValue::Number(3.0));
    assert_eq!(eval("-(2 + 3)"), EvalValue::Number(-5.0));
}

#[test]
fn evaluator_literals() {
    assert_eq!(eval("0x10 + $10 + 1e2 + .5"), EvalValue::Number(132.5));
    assert_eq!(eval("'it''s'"), EvalValue::String("it's".into()));
    assert_eq!(eval("true && !false"), EvalValue::String("true".into()));
    assert_eq!(
        eval("str 5 + str \"x\""),
        EvalValue::String("5\"x\"".into())
    );
}

#[test]
fn evaluator_unary_commands() {
    assert_eq!(
        eval("floor 2.7 + ceil 0.2 + round 1.5 + abs -1 + sqrt 9"),
        EvalValue::Number(9.0)
    );
}

#[test]
fn evaluator_variables_are_case_insensitive_and_persist() {
    let mut e = SimpleEvaluator::new();
    e.exec("Cat = 5 + 1; private _tmp = 2;").unwrap();
    assert_eq!(e.eval("cat * _TMP").unwrap(), EvalValue::Number(12.0));
    assert_eq!(e.variable("CAT"), Some(EvalValue::Number(6.0)));
}

#[test]
fn evaluator_rejects_unknown_names_and_type_errors() {
    let mut e = SimpleEvaluator::new();
    assert!(e.eval("getResolution select 2").is_err());
    assert!(e.eval("1 + \"a\"").is_err());
    assert!(e.eval("{1}").is_err());
    assert!(e.eval("").is_err());
}
