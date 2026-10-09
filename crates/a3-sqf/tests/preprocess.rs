//! Script loading through the preprocessor: `preprocessFile`, `preprocessFileLineNumbers`,
//! `execVM`, `compileScript`, and config `__EVAL`/`__EXEC` run by the VM.

mod common;

use std::time::Duration;

use a3_preproc::{EvalValue, Evaluator, MemoryResolver, Options, Preprocessor};
use a3_sqf::{Namespace, SqfEvaluator, Sym, Value};
use common::vm;

const FRAME: Duration = Duration::from_millis(3);

fn files(vm: &mut a3_sqf::Vm<common::TestHost>, list: &[(&str, &str)]) {
    for (path, text) in list {
        vm.host
            .files
            .insert(path.to_ascii_lowercase(), (*text).to_owned());
    }
}

#[test]
fn preprocess_file_expands_macros_and_includes() {
    let mut vm = vm();
    files(
        &mut vm,
        &[
            ("scripts\\fn_a.sqf", "#include \"macros.hpp\"\nGVAR(x) = 1;"),
            ("scripts\\macros.hpp", "#define GVAR(v) tag_##v"),
        ],
    );
    let text = vm.eval("preprocessFile \"scripts\\fn_a.sqf\"").unwrap();
    assert_eq!(text.to_sqf_string(), "\"\ntag_x = 1;\"");
}

#[test]
fn preprocess_file_line_numbers_adds_line_directives() {
    let mut vm = vm();
    files(&mut vm, &[("a\\b.sqf", "#define X 1\nx = X;")]);
    let text = vm.eval("preprocessFileLineNumbers \"a\\b.sqf\"").unwrap();
    assert_eq!(
        text.to_sqf_string(),
        "\"#line 1 \"\"a\\b.sqf\"\"\n\nx = 1;\""
    );
}

#[test]
fn absolute_include_from_relative_file() {
    let mut vm = vm();
    files(
        &mut vm,
        &[
            ("init.sqf", "#include \"\\a3\\inc.hpp\"\nv = V;"),
            ("\\a3\\inc.hpp", "#define V 5"),
        ],
    );
    vm.eval("call compile preprocessFileLineNumbers \"init.sqf\"")
        .unwrap();
    assert_eq!(vm.get_global("v").to_sqf_string(), "5");
}

#[test]
fn missing_file_gives_empty_string() {
    let mut vm = vm();
    let text = vm.eval("preprocessFile \"nope.sqf\"").unwrap();
    assert_eq!(text.to_sqf_string(), "\"\"");
}

#[test]
fn exec_vm_runs_the_preprocessed_script() {
    let mut vm = vm();
    files(
        &mut vm,
        &[(
            "scripts\\init.sqf",
            "#define ADD(a,b) (a + b)\nloaded = ADD(_this,1);",
        )],
    );
    vm.eval("41 execVM \"scripts\\init.sqf\"").unwrap();
    vm.run_scheduled(FRAME);
    assert_eq!(vm.get_global("loaded").to_sqf_string(), "42");
}

#[test]
fn compile_script_compiles_preprocessed_file() {
    let mut vm = vm();
    files(&mut vm, &[("f.sqf", "#define TWO 2\n_this * TWO")]);
    let v = vm.eval("21 call compileScript [\"f.sqf\"]").unwrap();
    assert_eq!(v.to_sqf_string(), "42");
    let v = vm.eval("isFinal compileScript [\"f.sqf\", true]").unwrap();
    assert_eq!(v, Value::Bool(true));
    let v = vm.eval("isFinal compileScript [\"f.sqf\"]").unwrap();
    assert_eq!(v, Value::Bool(false));
}

#[test]
fn compile_script_prepends_header_after_preprocessing() {
    let mut vm = vm();
    files(&mut vm, &[("f.sqf", "_x + 1")]);
    // The header is not preprocessed: TWO stays a variable name.
    let v = vm
        .eval("TWO = 40; call compileScript [\"f.sqf\", false, \"private _x = TWO; \"]")
        .unwrap();
    assert_eq!(v.to_sqf_string(), "41");
}

#[test]
fn compile_errors_point_into_the_included_file() {
    let mut vm = vm();
    files(
        &mut vm,
        &[
            ("m\\main.sqf", "a = 1;\n#include \"inc.sqf\"\nb = 2;"),
            ("m\\inc.sqf", "ok = 1;\nbad = (;\n"),
        ],
    );
    let text = a3_sqf::Host::preprocess_file(&mut vm.host, "m\\main.sqf", true).unwrap();
    let source = a3_sqf::SourceFile::new("m\\main.sqf", text.as_str());
    let err = a3_sqf::compile_source(&source, vm.table()).unwrap_err();
    let location = source.locate(err.span.start);
    assert_eq!((&*location.file, location.line), ("m\\inc.sqf", 2));
}

#[test]
fn localize_unknown_key_is_empty_and_logged() {
    let mut vm = vm();
    let v = vm.eval("localize \"$STR_Nope\"").unwrap();
    assert_eq!(v.to_sqf_string(), "\"\"");
    assert_eq!(vm.host.log, ["String STR_Nope not found"]);
}

// --- Evaluator -----------------------------------------------------------------------------

#[test]
fn evaluator_runs_exec_in_parsing_namespace() {
    let mut vm = vm();
    let mut eval = SqfEvaluator::new(&mut vm);
    eval.exec("cat = 5 + 1; lev = cat - 2;").unwrap();
    assert_eq!(eval.eval("cat * lev").unwrap(), EvalValue::Number(24.0));
    let parsing = vm.namespace(Namespace::Parsing);
    assert_eq!(parsing.get(Sym::new("cat")), Some(&Value::Number(6.0)));
    assert!(
        vm.namespace(Namespace::Mission)
            .get(Sym::new("cat"))
            .is_none()
    );
}

#[test]
fn evaluator_converts_results() {
    let mut vm = vm();
    let mut eval = SqfEvaluator::new(&mut vm);
    assert_eq!(
        eval.eval("\"a\" + \"b\"").unwrap(),
        EvalValue::String("ab".into())
    );
    assert_eq!(
        eval.eval("1 < 2").unwrap(),
        EvalValue::String("true".into())
    );
    assert_eq!(
        eval.eval("[1, \"x\"]").unwrap(),
        EvalValue::String("[1,\"x\"]".into())
    );
    assert_eq!(
        eval.eval("call {private _a = 0; {_a = _a + _x} forEach [1,2,3]; _a}")
            .unwrap(),
        EvalValue::Number(6.0)
    );
}

/// The game loads configs whose `__EXEC` fails (Laws of War's menu scene runs the campaign's
/// `description.inc`, whose `__EXEC` reads `_overviewLines`, which only the campaign missions
/// define): the lenient evaluator reports the error to the host and the config goes on.
#[test]
fn a_lenient_evaluator_reports_errors_and_goes_on() {
    let mut vm = vm();
    let files = MemoryResolver::new();
    let out = Preprocessor::new(&files)
        .with_options(Options::config())
        .with_evaluator(SqfEvaluator::lenient(&mut vm))
        .preprocess_str(
            "\\cfg\\description.ext",
            "__EXEC(_t = \"\"; {_t = _t + _x} forEach _undefinedLines)\na = __EVAL(1 + \"x\");\n__EXEC(x = (;)\nb = __EVAL(2 + 3);",
        )
        .expect("the config goes on");
    assert_eq!(out.text, "\na = \"\";\n\nb = 5;");
    assert_eq!(vm.host.errors.len(), 3, "{:#?}", vm.host.errors);
    assert!(vm.host.errors[0].contains("_undefinedlines"), "{}", vm.host.errors[0]);
}

#[test]
fn evaluator_errors_are_messages() {
    let mut vm = vm();
    let mut eval = SqfEvaluator::new(&mut vm);
    assert!(eval.eval("1 + \"a\"").is_err());
    assert!(eval.exec("x = (;").is_err());
}

#[test]
fn config_eval_and_exec_run_real_sqf() {
    let mut vm = vm();
    let files = MemoryResolver::new();
    let out = Preprocessor::new(&files)
        .with_options(Options::config())
        .with_evaluator(SqfEvaluator::new(&mut vm))
        .preprocess_str(
            "\\cfg\\config.cpp",
            "__EXEC(_list = [10, 20, 30])\nx = __EVAL(_list select 1);\ns = __EVAL(format [\"%1-%2\", 1, 2]);",
        )
        .unwrap();
    assert_eq!(out.text, "\nx = 20;\ns = \"1-2\";");
    // Locals of __EXEC persist in parsingNamespace too (wiki).
    assert_eq!(
        vm.namespace(Namespace::Parsing)
            .get(Sym::new("_list"))
            .map(Value::to_sqf_string),
        Some("[10,20,30]".to_owned())
    );
}
