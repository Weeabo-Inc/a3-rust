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

/// Imports the command table extracted from the binary
/// (`docs/re/sqf-commands.tsv`, see `docs/re/sqf-command-table.md`) into
/// `data/commands.tsv`, merged with the core registrations:
///
/// ```sh
/// SQF_COMMANDS_FROM=docs/re/sqf-commands.tsv cargo test -p a3-sqf --test table import
/// ```
///
/// Simple-expression commands (config evaluator, `EXPRESSION` types) are
/// left out. A name's binary precedence is that of its `Default`-category
/// overload when it has one (the core operators the parser sees first),
/// else the most common one.
#[test]
fn import_re_command_table() {
    let Some(path) = std::env::var_os("SQF_COMMANDS_FROM") else {
        return;
    };
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap().split('\t').collect();
    let col = |name: &str| header.iter().position(|h| *h == name).unwrap();
    let (c_name, c_kind, c_left, c_right, c_ret, c_prio, c_cat) = (
        col("name"),
        col("kind"),
        col("left_type"),
        col("right_type"),
        col("return_type"),
        col("priority"),
        col("category"),
    );
    let types = |s: &str| -> a3_sqf::TypeSet {
        if s.is_empty() {
            return a3_sqf::TypeSet::EMPTY;
        }
        let mut set = a3_sqf::TypeSet::EMPTY;
        for tok in s.split('|') {
            match tok {
                "?" | "ANY" => return a3_sqf::TypeSet::ANYTHING,
                t => {
                    let ty = a3_sqf::Type::from_token(t)
                        .unwrap_or_else(|| panic!("unknown type {t:?} in {s:?}"));
                    set = set | ty;
                }
            }
        }
        set
    };
    let mut table = CommandTable::new();
    // name -> (default-category precedence, precedence counts)
    let mut prios: std::collections::HashMap<String, (Option<u8>, Vec<u8>)> = Default::default();
    for line in lines {
        let c: Vec<&str> = line.split('\t').collect();
        if c[c_cat] == "Simple expression"
            || [c_left, c_right, c_ret]
                .iter()
                .any(|&i| c[i].contains("EXPRESSION"))
        {
            continue;
        }
        let form = match c[c_kind] {
            "nular" => Form::Nular,
            "unary" => Form::Unary,
            "binary" => Form::Binary,
            k => panic!("bad kind {k}"),
        };
        let sig = a3_sqf::Signature {
            left: types(c[c_left]),
            right: types(c[c_right]),
            ret: types(c[c_ret]),
        };
        table.declare(c[c_name], form, sig);
        if form == Form::Binary {
            let p: u8 = c[c_prio].parse().unwrap();
            let e = prios.entry(c[c_name].to_ascii_lowercase()).or_default();
            if c[c_cat] == "Default" && e.0.is_none() {
                e.0 = Some(p);
            }
            e.1.push(p);
        }
    }
    for (name, (default, all)) in prios {
        let p = default.unwrap_or_else(|| {
            let mut best = (0usize, 4u8);
            for &p in &all {
                let n = all.iter().filter(|&&q| q == p).count();
                if n > best.0 {
                    best = (n, p);
                }
            }
            best.1
        });
        let id = table.lookup(&name).unwrap();
        table.set_precedence(id, p);
    }
    let reg = Registry::<NullHost>::with_core();
    for (_, info) in reg.table().iter() {
        if let Some(s) = info.nular {
            table.declare(&info.name, Form::Nular, s);
        }
        for s in &info.unary {
            table.declare(&info.name, Form::Unary, *s);
        }
        for s in &info.binary {
            table.declare(&info.name, Form::Binary, *s);
        }
    }
    let out = concat!(env!("CARGO_MANIFEST_DIR"), "/data/commands.tsv");
    let body = table.to_tsv();
    let header = "# Script command signatures of arma3_x64.exe 2.22.0.154103, imported from\n\
                  # docs/re/sqf-commands.tsv (simple-expression commands left out) and merged\n\
                  # with the a3-sqf core registrations. Regenerate: see tests/table.rs.\n";
    std::fs::write(out, format!("{header}{body}")).unwrap();
    let (n, u, b) = table.form_counts();
    eprintln!(
        "imported {} names: {n} nular, {u} unary, {b} binary overloads",
        table.len()
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
