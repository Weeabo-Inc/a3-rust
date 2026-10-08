//! Preprocesses every text source in the game with the SQF-VM evaluator (after the function
//! library is initialised, as at game start), and compiles every `.sqf` with a3-sqf. Skipped
//! when `A3_ROOT` is unset.
//!
//! `cargo test -p a3-gamedata --release --test real_scripts -- --nocapture` prints the report.

use std::collections::BTreeMap;

use a3_gamedata::{
    CompileStats, GameData, LoadOptions, VfsResolver, compile_all, engine_command_table,
    error_category, init_functions, read_text, script_vm,
};
use a3_preproc::{ErrorKind, IncludeError, Options, Preprocessor};
use a3_sqf::{CommandTable, Namespace, SqfEvaluator, Sym};

fn preprocess_category(kind: &ErrorKind) -> String {
    match kind {
        ErrorKind::Include {
            source: IncludeError::NotFound(_),
            ..
        } => "include: not found".to_owned(),
        ErrorKind::Evaluation {
            macro_name,
            message,
        } => format!("{macro_name}: {}", error_category(message)),
        other => format!("{other:?}")
            .split(['(', ' ', '{'])
            .next()
            .unwrap_or_default()
            .to_owned(),
    }
}

fn print(title: &str, stats: &CompileStats) {
    let total = stats.ok + stats.failed();
    eprintln!(
        "{title}: {} / {total} ok ({:.2}%) in {:.2?}",
        stats.ok,
        100.0 * stats.ok as f64 / total.max(1) as f64,
        stats.elapsed
    );
    for (category, examples) in stats.top().iter().take(10) {
        eprintln!("    {:5}  {category}", examples.len());
        for example in examples.iter().take(2) {
            eprintln!("           e.g. {example}");
        }
    }
}

#[test]
fn preprocess_and_compile_every_script() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = GameData::load(&LoadOptions::new(root).with_optional_mods(true)).unwrap();
    let vfs = &data.vfs;
    let resolver = VfsResolver::new(vfs);

    // Configs: one VM with the function library initialised, as at game start; a fresh
    // parsingNamespace per file, as each mission/addon config is parsed on its own.
    let mut vm = script_vm(&data);
    let boot = init_functions(&mut vm);
    eprintln!(
        "function library: {} functions, {} errors",
        boot.compiled,
        boot.errors.len()
    );
    let started = std::time::Instant::now();
    let mut configs = CompileStats::default();
    let mut scripts = CompileStats::default();
    for path in vfs.walk("") {
        let path = format!("\\{}", path.as_str());
        let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
        let is_script = match ext {
            "sqf" => true,
            "hpp" | "inc" | "h" | "ext" | "cpp" | "sqm" => false,
            _ => continue,
        };
        let Ok(source) = read_text(vfs, &path) else {
            continue;
        };
        if source.starts_with("\0raP") {
            continue;
        }
        let parsing: Vec<Sym> = vm
            .namespace(Namespace::Parsing)
            .iter()
            .map(|(name, _)| name)
            .collect();
        for name in parsing {
            vm.namespace_mut(Namespace::Parsing).remove(name);
        }
        let stats = if is_script {
            &mut scripts
        } else {
            &mut configs
        };
        let options = Options {
            config_macros: !is_script,
            ..Options::default()
        };
        let result = Preprocessor::new(&resolver)
            .with_options(options)
            .with_evaluator(SqfEvaluator::new(&mut vm))
            .preprocess_str(&path, &source);
        match result {
            Ok(_) => stats.ok += 1,
            Err(e) => stats
                .failures
                .entry(preprocess_category(&e.kind))
                .or_default()
                .push(e.to_string()),
        }
    }
    configs.elapsed = started.elapsed();
    scripts.elapsed = configs.elapsed;
    print(
        "preprocess configs (.hpp/.inc/.h/.ext/.cpp/.sqm, SQF-VM __EVAL/__EXEC)",
        &configs,
    );
    print("preprocess .sqf", &scripts);
    print(
        "compile .sqf (a3-sqf builtin command table)",
        &compile_all(vfs, &CommandTable::builtin()),
    );
    print(
        "compile .sqf (builtin + docs/re engine command table)",
        &compile_all(vfs, &engine_command_table()),
    );

    // Preprocessing failures left are data defects (missing include targets) or `__EVAL`/
    // `__EXEC` that need commands or mission state the VM does not have.
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for (category, examples) in configs.failures.iter().chain(&scripts.failures) {
        *seen.entry(category).or_default() += examples.len();
        assert!(
            category == "include: not found" || category.starts_with("__E"),
            "unexpected preprocessing failure `{category}`: {examples:#?}"
        );
    }
}
