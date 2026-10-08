//! Preprocesses every text source in the game with the SQF-VM evaluator, and compiles every
//! `.sqf` with a3-sqf. Skipped when `A3_ROOT` is unset.
//!
//! `cargo test -p a3-gamedata --release --test real_scripts -- --nocapture` prints the report.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use a3_gamedata::{VfsHost, VfsResolver, read_text};
use a3_preproc::{ErrorKind, IncludeError, Options, Preprocessor};
use a3_sqf::{
    CommandTable, Form, Registry, Signature, SourceFile, SqfEvaluator, TypeSet, Vm, compile_source,
};
use a3_vfs::Vfs;

/// The built-in table plus every command of the engine's command table exported by RE
/// (`docs/re/sqf-commands.tsv`), so that commands without an implementation still parse.
fn full_table() -> Option<CommandTable> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/re/sqf-commands.tsv");
    let text = std::fs::read_to_string(path).ok()?;
    let mut table = CommandTable::builtin();
    let types = |s: &str| {
        if s.is_empty() {
            return TypeSet::default();
        }
        TypeSet::parse(&s.replace('|', ",")).unwrap_or(TypeSet::ANYTHING)
    };
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 5 {
            continue;
        }
        let form = match cols[1] {
            "nular" => Form::Nular,
            "unary" => Form::Unary,
            "binary" => Form::Binary,
            _ => continue,
        };
        let sig = Signature {
            left: types(cols[2]),
            right: types(cols[3]),
            ret: types(cols[4]),
        };
        table.declare(cols[0], form, sig);
    }
    Some(table)
}

/// Groups compile errors: the message with quoted names, numbers and long tails cut off.
fn compile_category(message: &str) -> String {
    let mut out = String::new();
    for word in message.split_whitespace().take(4) {
        if word.chars().any(|c| c.is_ascii_digit()) || word.starts_with(['\'', '"', '<', '_']) {
            out.push_str(" ?");
        } else {
            out.push(' ');
            out.push_str(word);
        }
    }
    out.trim().to_owned()
}

fn preprocess_category(kind: &ErrorKind) -> String {
    match kind {
        ErrorKind::Include {
            source: IncludeError::NotFound(_),
            ..
        } => "include: not found".to_owned(),
        ErrorKind::Evaluation {
            macro_name,
            message,
        } => format!("{macro_name}: {}", compile_category(message)),
        other => format!("{other:?}")
            .split(['(', ' ', '{'])
            .next()
            .unwrap_or_default()
            .to_owned(),
    }
}

#[derive(Default)]
struct Tally {
    ok: usize,
    failures: BTreeMap<String, Vec<String>>,
}

impl Tally {
    fn failed(&self) -> usize {
        self.failures.values().map(Vec::len).sum()
    }

    fn fail(&mut self, category: String, example: String) {
        self.failures.entry(category).or_default().push(example);
    }

    fn print(&self, title: &str) {
        let total = self.ok + self.failed();
        eprintln!(
            "{title}: {} / {total} ok ({:.2}%)",
            self.ok,
            100.0 * self.ok as f64 / total.max(1) as f64
        );
        let mut categories: Vec<_> = self.failures.iter().collect();
        categories.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
        for (category, examples) in categories.iter().take(12) {
            eprintln!("    {:5}  {category}", examples.len());
            for example in examples.iter().take(2) {
                eprintln!("           e.g. {example}");
            }
        }
    }
}

#[test]
fn preprocess_and_compile_every_script() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let root = PathBuf::from(root);
    let vfs = Vfs::new();
    let report = vfs.mount_game(&root, &a3_vfs::optional_mod_dirs(&root));
    eprintln!("mounted {} PBOs, {} files", report.pbos, vfs.len());
    let resolver = VfsResolver::new(&vfs);
    let builtin = CommandTable::builtin();
    let full = full_table();
    // The evaluator VM knows every engine command, so `__EVAL`s using a command the VM does not
    // implement yet fail with "unimplemented" rather than a parse error.
    let registry = Rc::new(Registry::with_core_table(
        full.clone().unwrap_or_else(CommandTable::builtin),
    ));

    let mut configs = Tally::default();
    let mut scripts = Tally::default();
    let mut compiled = Tally::default();
    let mut compiled_full = Tally::default();
    for path in vfs.walk("") {
        let path = format!("\\{}", path.as_str());
        let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
        let is_script = match ext {
            "sqf" => true,
            "hpp" | "inc" | "h" | "ext" | "cpp" | "sqm" => false,
            _ => continue,
        };
        let Ok(source) = read_text(&vfs, &path) else {
            continue;
        };
        if source.starts_with("\0raP") {
            continue;
        }
        let tally = if is_script {
            &mut scripts
        } else {
            &mut configs
        };
        // A fresh parsingNamespace per file, as each mission/addon loads on its own.
        let mut vm = Vm::with_registry(VfsHost::new(vfs.clone()), Rc::clone(&registry));
        let options = Options {
            config_macros: !is_script,
            ..Options::default()
        };
        let output = Preprocessor::new(&resolver)
            .with_options(options)
            .with_evaluator(SqfEvaluator::new(&mut vm))
            .preprocess_str(&path, &source);
        let output = match output {
            Ok(output) => {
                tally.ok += 1;
                output
            }
            Err(e) => {
                tally.fail(preprocess_category(&e.kind), e.to_string());
                continue;
            }
        };
        if !is_script {
            continue;
        }
        let text = output.with_line_directives();
        let file = SourceFile::new(path.as_str(), text.as_str());
        for (table, tally) in [
            (Some(&builtin), &mut compiled),
            (full.as_ref(), &mut compiled_full),
        ] {
            let Some(table) = table else { continue };
            match compile_source(&file, table) {
                Ok(_) => tally.ok += 1,
                Err(e) => {
                    let at = file.locate(e.span.start);
                    tally.fail(
                        compile_category(&e.message),
                        format!("{}:{}: {}", at.file, at.line, e.message),
                    );
                }
            }
        }
    }

    configs.print("preprocess configs (.hpp/.inc/.h/.ext/.cpp/.sqm, SQF-VM __EVAL/__EXEC)");
    scripts.print("preprocess .sqf");
    compiled.print("compile .sqf (built-in command table)");
    if full.is_some() {
        compiled_full.print("compile .sqf (built-in + docs/re command table)");
    }

    // Preprocessing failures left are data defects (missing include targets) or `__EVAL`/
    // `__EXEC` that need commands the VM does not implement yet.
    for (category, examples) in configs.failures.iter().chain(&scripts.failures) {
        assert!(
            category == "include: not found" || category.starts_with("__E"),
            "unexpected preprocessing failure `{category}`: {examples:#?}"
        );
    }
}
