//! Parser behaviour: precedence, operand resolution and statement forms.

use a3_sqf::ast::{CodeBlock, Expr, ExprKind, Statement, print_block};
use a3_sqf::parser::{parse, parse_expression};
use a3_sqf::{CommandTable, compile_str};

/// Renders an expression as a fully parenthesised S-expression.
fn sexpr(e: &Expr, t: &CommandTable) -> String {
    match &e.kind {
        ExprKind::Number(n) => format!("{n}"),
        ExprKind::String(s) => format!("{s:?}"),
        ExprKind::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(|i| sexpr(i, t))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        ExprKind::Code(b) => format!("{{{}}}", block_sexpr(b, t)),
        ExprKind::Variable { text, .. } => text.to_string(),
        ExprKind::Nular(id) => t.get(*id).name.clone(),
        ExprKind::Unary { op, arg, .. } => format!("({} {})", t.get(*op).name, sexpr(arg, t)),
        ExprKind::Binary {
            op, left, right, ..
        } => format!(
            "({} {} {})",
            t.get(*op).name,
            sexpr(left, t),
            sexpr(right, t)
        ),
    }
}

fn block_sexpr(b: &CodeBlock, t: &CommandTable) -> String {
    b.statements
        .iter()
        .map(|s| match s {
            Statement::Expr(e) => sexpr(e, t),
            Statement::Assign {
                private,
                name_text,
                value,
                ..
            } => format!(
                "({}= {} {})",
                if *private { "private" } else { "" },
                name_text,
                sexpr(value, t)
            ),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn p(src: &str) -> String {
    let t = CommandTable::builtin();
    let e = parse_expression(src, &t).unwrap_or_else(|e| panic!("{src}: {e}"));
    sexpr(&e, &t)
}

fn stmts(src: &str) -> String {
    let t = CommandTable::builtin();
    let b = parse(src, &t).unwrap_or_else(|e| panic!("{src}: {e}"));
    block_sexpr(&b, &t)
}

fn err(src: &str) -> String {
    let t = CommandTable::builtin();
    parse(src, &t).unwrap_err().message
}

#[test]
fn arithmetic_precedence() {
    assert_eq!(p("1 + 2 * 3"), "(+ 1 (* 2 3))");
    assert_eq!(p("1 - 2 - 3"), "(- (- 1 2) 3)");
    assert_eq!(p("2 ^ 3 ^ 2"), "(^ (^ 2 3) 2)");
    assert_eq!(p("2 * 3 ^ 2"), "(* 2 (^ 3 2))");
    assert_eq!(p("10 mod 3 + 1"), "(+ (mod 10 3) 1)");
    assert_eq!(p("1 max 2 * 3"), "(max 1 (* 2 3))");
}

#[test]
fn unary_binds_tighter_than_binary() {
    assert_eq!(p("count [1] + 1"), "(+ (count [1]) 1)");
    assert_eq!(p("-1 ^ 2"), "(^ (- 1) 2)");
    assert_eq!(p("!true && false"), "(&& (! true) false)");
    assert_eq!(p("- - 1"), "(- (- 1))");
}

#[test]
fn hash_select_binds_tighter_than_power() {
    assert_eq!(p("_a # 1 ^ 2"), "(^ (# _a 1) 2)");
    assert_eq!(p("_a # 0 + 1"), "(+ (# _a 0) 1)");
}

#[test]
fn other_binary_commands_bind_looser_than_arithmetic() {
    assert_eq!(p("_a select 1 + 1"), "(select _a (+ 1 1))");
    assert_eq!(p("_a select 1 == 2"), "(== (select _a 1) 2)");
}

#[test]
fn comparisons_bind_tighter_than_logic() {
    assert_eq!(
        p("_a == 1 && _b != 2 || _c"),
        "(|| (&& (== _a 1) (!= _b 2)) _c)"
    );
    assert_eq!(p("_a and _b or _c and _d"), "(or (and _a _b) (and _c _d))");
}

#[test]
fn if_then_else() {
    assert_eq!(
        p("if (_a > 1) then {1} else {2}"),
        "(then (if (> _a 1)) (else {1} {2}))"
    );
}

#[test]
fn else_binds_tighter_than_other_binary() {
    assert_eq!(p("_x then _y else _z"), "(then _x (else _y _z))");
}

#[test]
fn config_path_is_left_associative() {
    assert_eq!(p("_c >> \"a\" >> \"b\""), "(>> (>> _c \"a\") \"b\")");
}

#[test]
fn nular_commands_are_values_and_unknown_names_are_variables() {
    assert_eq!(p("time + myVar"), "(+ time myVar)");
    assert_eq!(p("TRUE"), "true");
}

#[test]
fn for_loop_chain() {
    assert_eq!(
        p("for \"_i\" from 0 to 10 step 2 do {}"),
        "(do (step (to (from (for \"_i\") 0) 10) 2) {})"
    );
}

#[test]
fn statements_and_assignments() {
    assert_eq!(
        stmts("_a = 1; private _b = 2, c = 3;"),
        "(= _a 1); (private= _b 2); (= c 3)"
    );
    assert_eq!(stmts(";;1;;"), "1");
    assert_eq!(stmts("private _x"), "(private \"_x\")");
    assert_eq!(stmts("private [\"_x\"]"), "(private [\"_x\"])");
}

#[test]
fn nested_code_and_arrays() {
    assert_eq!(
        stmts("_f = {params [\"_a\"]; _a * 2}; [1, [2, 3], {}]"),
        "(= _f {(params [\"_a\"]); (* _a 2)}); [1 [2 3] {}]"
    );
}

#[test]
fn syntax_errors_use_engine_messages() {
    assert_eq!(err("_a = [1, 2"), "Missing ]");
    assert_eq!(err("_a = (1 + 2"), "Missing )");
    assert_eq!(err("call {1"), "Missing }");
    assert_eq!(err("_a = 1 _b = 2"), "Missing ;");
    assert_eq!(err("_a = \"abc"), "Missing \"\"");
}

#[test]
fn printing_round_trips() {
    let t = CommandTable::builtin();
    for src in [
        "_a = 1 + 2 * 3",
        "(1 + 2) * 3",
        "1 - (2 - 3)",
        "if (_a > 1) then {_b = [1, \"x\"\"y\"]} else {nil}",
        "private _c = count (_a + _b)",
        "_x select (_i + 1)",
    ] {
        let printed = print_block(&parse(src, &t).unwrap(), &t);
        let reparsed = print_block(&parse(&printed, &t).unwrap(), &t);
        assert_eq!(printed, reparsed, "{src}");
        assert_eq!(stmts(src), stmts(&printed), "{src} -> {printed}");
    }
}

#[test]
fn compiled_code_keeps_its_source_text() {
    let t = CommandTable::builtin();
    let code = compile_str("", "_f = { _x + 1 }; 2", &t).unwrap();
    assert_eq!(code.source(), "_f = { _x + 1 }; 2");
    let inner = code
        .instructions()
        .iter()
        .find_map(|i| match i {
            a3_sqf::Instr::Push(a3_sqf::Value::Code(c)) => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(inner.source(), " _x + 1 ");
}

#[test]
fn line_directives_map_error_positions() {
    let t = CommandTable::builtin();
    let src = a3_sqf::SourceFile::new("", "#line 20 \"x\\y.sqf\"\n_a = 1;\n_b = (2;");
    let e = a3_sqf::compile_source(&src, &t).unwrap_err();
    let loc = src.locate(e.span.start);
    assert_eq!(&*loc.file, "x\\y.sqf");
    assert_eq!(loc.line, 21);
    let report = e.render(&src);
    assert!(report.contains("Error Missing )"), "{report}");
    assert!(report.contains("line 21"), "{report}");
}
