//! Static script command usage of a game install: which commands its code calls, and how often.
//!
//! Three kinds of shipped code are scanned:
//! - every `.sqf` file, preprocessed and compiled as [`crate::compile_all`] does;
//! - every `.fsm` file: the SQF in its states' `init` / `precondition` and its links'
//!   `condition` / `action` strings;
//! - SQF embedded in the merged config: event handler texts (`on<Event>` entries such as
//!   `onLoad`, and every string inside an `EventHandlers` class) and the `statement`,
//!   `condition`, `expression`, `action` and `init` entries. Identical texts count once, so a
//!   handler copied into hundreds of vehicle classes does not dominate.
//!
//! A use is one call instruction in the compiled code (`Nular`, `Unary`, `Binary`), including
//! the code blocks nested inside. Texts that do not compile are skipped and counted as failures.
//! Code built at run time (`compile format [...]`) is not seen.

use std::collections::{HashMap, HashSet};

use a3_config::{ConfigRef, ConfigTree};
use a3_preproc::Preprocessor;
use a3_sqf::{Code, CommandTable, Form, Instr, SourceFile, compile_source, compile_str};
use a3_vfs::Vfs;

use crate::scripts::{VfsResolver, read_text};

/// The kind of shipped code a command use was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UsageSource {
    Sqf = 0,
    Fsm = 1,
    Config = 2,
}

/// Command uses per source, and how much code was scanned.
#[derive(Debug, Clone, Default)]
pub struct CommandUsage {
    /// `(lower-case name, form)` → uses in `[sqf, fsm, config]` code.
    counts: HashMap<(String, Form), [usize; 3]>,
    /// Texts compiled per source.
    pub compiled: [usize; 3],
    /// Texts that failed to preprocess or compile, per source.
    pub failed: [usize; 3],
}

impl CommandUsage {
    /// Uses of `name` (any case) in `form`, per source `[sqf, fsm, config]`.
    pub fn get(&self, name: &str, form: Form) -> [usize; 3] {
        self.counts
            .get(&(name.to_ascii_lowercase(), form))
            .copied()
            .unwrap_or_default()
    }

    /// Total uses of `name` in `form` across all sources.
    pub fn total(&self, name: &str, form: Form) -> usize {
        self.get(name, form).iter().sum()
    }

    /// Every `(lower-case name, form, uses per source)`.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Form, [usize; 3])> {
        self.counts
            .iter()
            .map(|((name, form), counts)| (name.as_str(), *form, *counts))
    }

    /// Adds the command calls of `code` and the code blocks nested in it.
    pub fn add_code(&mut self, code: &Code, table: &CommandTable, source: UsageSource) {
        for instr in code.instructions() {
            let (id, form) = match instr {
                Instr::Nular(id) => (*id, Form::Nular),
                Instr::Unary(id) => (*id, Form::Unary),
                Instr::Binary(id) => (*id, Form::Binary),
                Instr::Push(a3_sqf::Value::Code(inner)) => {
                    self.add_code(inner, table, source);
                    continue;
                }
                _ => continue,
            };
            let name = table.get(id).name.to_ascii_lowercase();
            self.counts.entry((name, form)).or_default()[source as usize] += 1;
        }
    }

    /// Compiles `text` (a snippet without file context) and adds its calls; `false` when it
    /// does not compile.
    pub fn add_snippet(&mut self, text: &str, table: &CommandTable, source: UsageSource) -> bool {
        match compile_str("", text, table) {
            Ok(code) => {
                self.add_code(&code, table, source);
                self.compiled[source as usize] += 1;
                true
            }
            Err(_) => {
                self.failed[source as usize] += 1;
                false
            }
        }
    }

    /// Every `.sqf` file of `vfs`, preprocessed with `#line` directives and compiled.
    pub fn scan_sqf(&mut self, vfs: &Vfs, table: &CommandTable) {
        let resolver = VfsResolver::new(vfs);
        for path in vfs.glob("**/*.sqf") {
            let path = format!("\\{}", path.as_str());
            let Ok(source) = read_text(vfs, &path) else {
                continue;
            };
            let Ok(output) = Preprocessor::new(&resolver).preprocess_str(&path, &source) else {
                self.failed[UsageSource::Sqf as usize] += 1;
                continue;
            };
            let text = output.with_line_directives();
            match compile_source(&SourceFile::new(path.as_str(), text.as_str()), table) {
                Ok(code) => {
                    self.add_code(&code, table, UsageSource::Sqf);
                    self.compiled[UsageSource::Sqf as usize] += 1;
                }
                Err(_) => self.failed[UsageSource::Sqf as usize] += 1,
            }
        }
    }

    /// Every `.fsm` file of `vfs`: the SQF texts [`fsm_code`] finds.
    pub fn scan_fsm(&mut self, vfs: &Vfs, table: &CommandTable) {
        for path in vfs.glob("**/*.fsm") {
            let Ok(source) = read_text(vfs, &format!("\\{}", path.as_str())) else {
                continue;
            };
            let mut seen = HashSet::new();
            for text in fsm_code(&source) {
                if !text.trim().is_empty() && seen.insert(text.clone()) {
                    self.add_snippet(&text, table, UsageSource::Fsm);
                }
            }
        }
    }

    /// The SQF texts of the merged config (see the module docs), each distinct text once.
    pub fn scan_config(&mut self, config: &ConfigTree, table: &CommandTable) {
        let mut texts = HashSet::new();
        collect_config_code(&config.root(), false, &mut texts);
        for text in texts {
            self.add_snippet(&text, table, UsageSource::Config);
        }
    }
}

/// Scans `.sqf`, `.fsm` and config code of a loaded game (see the module docs).
pub fn command_usage(vfs: &Vfs, config: &ConfigTree, table: &CommandTable) -> CommandUsage {
    let mut usage = CommandUsage::default();
    usage.scan_sqf(vfs, table);
    usage.scan_fsm(vfs, table);
    usage.scan_config(config, table);
    usage
}

/// Whether a config entry named `name` holds SQF: `on<Event>` handlers and the action /
/// condition entries listed in the module docs.
pub fn is_code_entry(name: &str) -> bool {
    let b = name.as_bytes();
    if b.len() > 2 && b[..2].eq_ignore_ascii_case(b"on") && b[2].is_ascii_uppercase() {
        return true;
    }
    ["statement", "condition", "expression", "action", "init"]
        .iter()
        .any(|k| name.eq_ignore_ascii_case(k))
}

fn collect_config_code(class: &ConfigRef<'_>, in_handlers: bool, out: &mut HashSet<String>) {
    for entry in class.entries() {
        if entry.is_class() {
            let handlers = in_handlers || entry.name().eq_ignore_ascii_case("EventHandlers");
            collect_config_code(&entry, handlers, out);
        } else if entry.is_text() && (in_handlers || is_code_entry(entry.name())) {
            let text = entry.text();
            if !text.trim().is_empty() && !out.contains(&text) {
                out.insert(text);
            }
        }
    }
}

/// The SQF texts of an FSM file: the values of its `init`, `precondition`, `condition` and
/// `action` entries. Values are config strings, possibly several joined by `\n` (a line
/// break); comments are skipped.
pub fn fsm_code(source: &str) -> Vec<String> {
    let tokens = fsm_tokens(source);
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 < tokens.len() {
        let is_key = matches!(&tokens[i], FsmToken::Ident(name)
            if ["init", "precondition", "condition", "action"]
                .iter()
                .any(|k| name.eq_ignore_ascii_case(k)));
        if !is_key || tokens[i + 1] != FsmToken::Equals {
            i += 1;
            continue;
        }
        let mut j = i + 2;
        let mut text = String::new();
        let mut any = false;
        loop {
            match tokens.get(j) {
                Some(FsmToken::Str(s)) => {
                    text.push_str(s);
                    any = true;
                }
                Some(FsmToken::LineBreak) => text.push('\n'),
                _ => break,
            }
            j += 1;
        }
        if any {
            out.push(text);
        }
        i = j;
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
enum FsmToken {
    Ident(String),
    Equals,
    Str(String),
    /// `\n` between two strings.
    LineBreak,
    Other,
}

fn fsm_tokens(source: &str) -> Vec<FsmToken> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i += 2;
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                while i < chars.len() {
                    if chars[i] == '"' {
                        if chars.get(i + 1) == Some(&'"') {
                            s.push('"');
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    s.push(chars[i]);
                    i += 1;
                }
                i += 1;
                out.push(FsmToken::Str(s));
            }
            '\\' if chars.get(i + 1) == Some(&'n') => {
                out.push(FsmToken::LineBreak);
                i += 2;
            }
            '=' => {
                out.push(FsmToken::Equals);
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push(FsmToken::Ident(chars[start..i].iter().collect()));
            }
            c if c.is_whitespace() => i += 1,
            _ => {
                out.push(FsmToken::Other);
                i += 1;
            }
        }
    }
    out
}
