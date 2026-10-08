//! The shipped signature table covers every implemented command.
//!
//! Run with `UPDATE_SQF_COMMANDS=1` to merge the signatures of the core
//! implementations into `data/commands.tsv`.

use a3_sqf::{CommandTable, Form, NullHost, Registry};

#[test]
fn core_commands_are_in_the_shipped_table() {
    let reg = Registry::<NullHost>::with_core();
    let builtin = CommandTable::builtin();
    let mut missing = Vec::new();
    for (_, info) in reg.table().iter() {
        for form in [Form::Nular, Form::Unary, Form::Binary] {
            if info.has_form(form) && !builtin.has(&info.name, form) {
                missing.push(format!("{} {}", info.name, form.as_str()));
            }
        }
    }
    if std::env::var_os("UPDATE_SQF_COMMANDS").is_some() {
        let mut merged = builtin.clone();
        for (_, info) in reg.table().iter() {
            if let Some(s) = info.nular {
                merged.declare(&info.name, Form::Nular, s);
            }
            for s in &info.unary {
                merged.declare(&info.name, Form::Unary, *s);
            }
            for s in &info.binary {
                merged.declare(&info.name, Form::Binary, *s);
            }
        }
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/commands.tsv");
        std::fs::write(path, merged.to_tsv()).unwrap();
        return;
    }
    assert!(
        missing.is_empty(),
        "commands missing from data/commands.tsv (run with UPDATE_SQF_COMMANDS=1): {missing:?}"
    );
}

#[test]
fn coverage_is_reported() {
    let reg = Registry::<NullHost>::with_core();
    let c = reg.coverage();
    let (n, u, b) = (c.nular, c.unary, c.binary);
    eprintln!(
        "implemented: nular {}/{}, unary {}/{}, binary {}/{}",
        n.0, n.1, u.0, u.1, b.0, b.1
    );
    assert!(n.0 > 0 && u.0 > 0 && b.0 > 0);
}
