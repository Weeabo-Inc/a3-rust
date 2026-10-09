//! Script boot: the VM the game's scripts run on, and the function-library initialisation the
//! engine performs at game start.
//!
//! # What the engine does (see `docs/re/functions-init.md`)
//!
//! At game start, and again when a mission starts, the engine reads
//! `configFile >> "CfgFunctions" >> "init"` (`A3\functions_f\initFunctions.sqf`), preprocesses
//! it with line numbers, compiles it and runs it unscheduled with `_this` undefined, in
//! `missionNamespace`. The script then auto-detects its mode: at game start (no
//! `bis_fnc_init` in uiNamespace) it compiles every `configFile` CfgFunctions entry into
//! uiNamespace, creates missionNamespace shortcuts and sets `uiNamespace bis_fnc_init`.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use a3_preproc::Preprocessor;
use a3_sqf::registry::Registry;
use a3_sqf::{
    Code, CommandTable, Form, Handle, HandleKind, Host, Instr, Namespace, SourceFile, Type,
    TypeSet, Value, Vm, compile_source,
};
use a3_vfs::Vfs;

use crate::GameData;
use crate::scripts::{ErrorLog, VfsHost, VfsResolver, read_text};
use crate::sqf_config::{ConfigHost, ConfigRoot, register_config_commands};

/// The command table game scripts compile against: a3-sqf's builtin table, which holds the
/// engine's full command list (#82), so every shipped command parses even before it has an
/// implementation.
pub fn engine_command_table() -> CommandTable {
    CommandTable::builtin()
}

/// Commands whose answer is fixed for a headless retail game at the main menu: no displays,
/// no editor, no cheats, single player, local server.
pub fn register_headless<H: Host>(r: &mut Registry<H>) {
    let boolean = TypeSet::of(Type::Bool);
    r.nular("cheatsEnabled", boolean, |_| Ok(Value::Bool(false)));
    r.nular("is3DEN", boolean, |_| Ok(Value::Bool(false)));
    r.nular("is3DENMultiplayer", boolean, |_| Ok(Value::Bool(false)));
    r.nular("is3DENPreview", boolean, |_| Ok(Value::Bool(false)));
    r.nular("isServer", boolean, |_| Ok(Value::Bool(true)));
    r.nular("isDedicated", boolean, |_| Ok(Value::Bool(false)));
    r.nular("hasInterface", boolean, |_| Ok(Value::Bool(true)));
    r.nular("isMultiplayer", boolean, |_| Ok(Value::Bool(false)));
    r.nular("isMultiplayerSolo", boolean, |_| Ok(Value::Bool(false)));
    r.nular("didJIP", boolean, |_| Ok(Value::Bool(false)));
    r.nular("worldName", TypeSet::of(Type::String), |_| {
        Ok(Value::from(""))
    });
    // No mission is running, so the world clock has not started _(uncertain: the engine's
    // value at the main menu)_. Needed as the default argument of BIS_fnc_timeToString.
    r.nular("dayTime", TypeSet::NUMBER, |_| Ok(Value::Number(0.0)));
    r.binary(
        "get3DENMissionAttribute",
        TypeSet::of(Type::String),
        TypeSet::of(Type::String),
        TypeSet::ANYTHING,
        |_, _, _| Ok(Value::Bool(false)),
    );
    r.unary(
        "findDisplay",
        TypeSet::NUMBER,
        TypeSet::of(Type::Display),
        |_, _| Ok(Value::Handle(Handle::null(HandleKind::Display))),
    );
    r.nular("displayNull", TypeSet::of(Type::Display), |_| {
        Ok(Value::Handle(Handle::null(HandleKind::Display)))
    });
    // Development-build logging; does nothing in the retail game.
    r.unary(
        "textLogFormat",
        TypeSet::of(Type::Array),
        TypeSet::of(Type::Nothing),
        |_, _| Ok(Value::Nothing),
    );
}

/// The command registry for game scripts: core commands over [`engine_command_table`], config
/// commands and the headless game state.
pub fn script_registry<H: ConfigHost>() -> Registry<H> {
    let mut r = Registry::with_core_table(engine_command_table());
    register_config_commands(&mut r);
    register_headless(&mut r);
    r
}

/// A VM over a loaded game, as scripts see it at the main menu.
pub fn script_vm(data: &GameData) -> Vm<VfsHost> {
    Vm::with_registry(VfsHost::for_game(data), Rc::new(script_registry()))
}

/// The outcome of [`init_functions`].
#[derive(Debug, Clone, Default)]
pub struct FunctionsReport {
    /// The init script (`configFile >> "CfgFunctions" >> "init"`).
    pub init_script: String,
    /// Distinct `<tag>_fnc_<name>` functions declared in `configFile >> "CfgFunctions"`.
    pub declared: usize,
    /// Declared functions without code in uiNamespace afterwards (lower case).
    pub missing: Vec<String>,
    /// `<tag>_fnc_<name>` variables holding code in uiNamespace afterwards.
    pub compiled: usize,
    /// Whether `uiNamespace getVariable "bis_fnc_init"` is true afterwards.
    pub finished: bool,
    /// Script errors reported while it ran.
    pub errors: Vec<String>,
    /// Wall-clock time of the run.
    pub elapsed: Duration,
}

/// Runs the function-library initialisation the engine performs at game start.
pub fn init_functions<H: ConfigHost + ErrorLog>(vm: &mut Vm<H>) -> FunctionsReport {
    let start = Instant::now();
    let mut report = FunctionsReport::default();
    let errors_before = vm.host.error_log().len();
    let mut declared = std::collections::BTreeSet::new();
    {
        let config = std::sync::Arc::clone(vm.host.configs().tree(ConfigRoot::Game));
        let cfg = config.root() >> "CfgFunctions";
        report.init_script = (&cfg >> "init").text();
        for tag in cfg.entries().iter().filter(|t| t.is_class()) {
            let tag_name = match (tag >> "tag").text() {
                t if t.is_empty() => tag.name().to_owned(),
                t => t,
            };
            for category in tag.entries().iter().filter(|c| c.is_class()) {
                for function in category.entries().iter().filter(|f| f.is_class()) {
                    declared
                        .insert(format!("{tag_name}_fnc_{}", function.name()).to_ascii_lowercase());
                }
            }
        }
        report.declared = declared.len();
    }
    if report.init_script.is_empty() {
        report
            .errors
            .push("no configFile >> CfgFunctions >> init".to_owned());
        return report;
    }
    let path = report.init_script.clone();
    match vm.host.preprocess_file(&path, true) {
        Ok(text) => match vm.compile_file(&path, &text) {
            Ok(code) => {
                let _ = vm.call_in(&code, None, Namespace::Mission);
            }
            Err(e) => report
                .errors
                .push(e.render(&SourceFile::new(path.as_str(), text.as_str()))),
        },
        Err(e) => report.errors.push(e),
    }
    report
        .errors
        .extend(vm.host.error_log()[errors_before..].iter().cloned());
    let ui = vm.namespace(Namespace::Ui);
    report.compiled = ui
        .iter()
        .filter(|(name, value)| {
            let name = name.as_str();
            name.contains("_fnc_") && !name.ends_with("_meta") && matches!(value, Value::Code(_))
        })
        .count();
    report.missing = declared
        .into_iter()
        .filter(|name| !matches!(ui.get(a3_sqf::Sym::new(name)), Some(Value::Code(_))))
        .collect();
    report.finished = matches!(
        ui.get(a3_sqf::Sym::new("bis_fnc_init")),
        Some(Value::Bool(true))
    );
    report.elapsed = start.elapsed();
    report
}

/// How often each command without an implementation is used (statically) by the code values in
/// `namespace`, most used first. Shows what a script library still needs from the VM.
pub fn unimplemented_usage<H: Host>(vm: &Vm<H>, namespace: Namespace) -> Vec<(String, usize)> {
    let mut counts = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for (_, value) in vm.namespace(namespace).iter() {
        if let Value::Code(code) = value {
            scan_unimplemented(vm, code, &mut seen, &mut counts);
        }
    }
    ranked(counts)
}

/// Like [`unimplemented_usage`], for code the caller still holds rather than a namespace (an
/// `init.sqf` a runner just compiled, say).
pub fn unimplemented_usage_in<'a, H: Host>(
    vm: &Vm<H>,
    codes: impl IntoIterator<Item = &'a Code>,
) -> Vec<(String, usize)> {
    let mut counts = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for code in codes {
        scan_unimplemented(vm, code, &mut seen, &mut counts);
    }
    ranked(counts)
}

/// Adds the commands of `code` (and the code values inside it) that the VM cannot run to `out`.
fn scan_unimplemented<H: Host>(
    vm: &Vm<H>,
    code: &Code,
    seen: &mut std::collections::HashSet<*const Instr>,
    out: &mut BTreeMap<String, usize>,
) {
    if !seen.insert(code.instructions().as_ptr()) {
        return;
    }
    for instr in code.instructions() {
        let (id, form) = match instr {
            Instr::Nular(id) => (*id, Form::Nular),
            Instr::Unary(id) => (*id, Form::Unary),
            Instr::Binary(id) => (*id, Form::Binary),
            Instr::Push(Value::Code(inner)) => {
                scan_unimplemented(vm, inner, seen, out);
                continue;
            }
            _ => continue,
        };
        let name = &vm.table().get(id).name;
        if !vm.registry().is_implemented(name, form) {
            *out.entry(format!("{name} ({})", form.as_str()))
                .or_default() += 1;
        }
    }
}

/// `counts` as a list, most used first, ties by name.
fn ranked(counts: BTreeMap<String, usize>) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Parse statistics of [`compile_all`].
#[derive(Debug, Clone, Default)]
pub struct CompileStats {
    /// Files that preprocessed and compiled.
    pub ok: usize,
    /// Failures by category (`"preprocess: ..."` / the compile error message with names
    /// masked), with `file:line: message` examples.
    pub failures: BTreeMap<String, Vec<String>>,
    pub elapsed: Duration,
}

impl CompileStats {
    /// Number of failed files.
    pub fn failed(&self) -> usize {
        self.failures.values().map(Vec::len).sum()
    }

    /// Categories, largest first.
    pub fn top(&self) -> Vec<(&str, &[String])> {
        let mut v: Vec<_> = self
            .failures
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
            .collect();
        v.sort_by_key(|(_, examples)| std::cmp::Reverse(examples.len()));
        v
    }
}

/// Groups error messages: words with digits, quotes or variable names are masked.
pub fn error_category(message: &str) -> String {
    let mut out = String::new();
    for word in message.split_whitespace().take(5) {
        out.push(' ');
        if word.chars().any(|c| c.is_ascii_digit()) || word.starts_with(['\'', '"', '<', '_']) {
            out.push('?');
        } else {
            out.push_str(word);
        }
    }
    out.trim().to_owned()
}

/// Preprocesses (with `#line` directives) and compiles every `.sqf` in `vfs` with `table`.
pub fn compile_all(vfs: &Vfs, table: &CommandTable) -> CompileStats {
    let start = Instant::now();
    let resolver = VfsResolver::new(vfs);
    let mut stats = CompileStats::default();
    for path in vfs.glob("**/*.sqf") {
        let path = format!("\\{}", path.as_str());
        let Ok(source) = read_text(vfs, &path) else {
            continue;
        };
        let output = match Preprocessor::new(&resolver).preprocess_str(&path, &source) {
            Ok(output) => output,
            Err(e) => {
                stats
                    .failures
                    .entry(format!(
                        "preprocess: {}",
                        error_category(&e.kind.to_string())
                    ))
                    .or_default()
                    .push(e.to_string());
                continue;
            }
        };
        let text = output.with_line_directives();
        let file = SourceFile::new(path.as_str(), text.as_str());
        match compile_source(&file, table) {
            Ok(_) => stats.ok += 1,
            Err(e) => {
                let at = file.locate(e.span.start);
                stats
                    .failures
                    .entry(error_category(&e.message))
                    .or_default()
                    .push(format!("{}:{}: {}", at.file, at.line, e.message));
            }
        }
    }
    stats.elapsed = start.elapsed();
    stats
}
