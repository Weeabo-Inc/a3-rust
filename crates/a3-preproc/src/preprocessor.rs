//! The preprocessor driver: directives, conditionals, includes and macro expansion.

use std::collections::HashMap;
use std::sync::Arc;

use crate::env::{Clock, GameVersion, Now, RandomSource, SplitMix64, SystemClock};
use crate::error::{ErrorKind, PreprocessError};
use crate::eval::{EvalValue, Evaluator, SimpleEvaluator, format_number};
use crate::lines::{Line, split_lines};
use crate::macros::{
    Macro, Piece, ident_len, is_ident_char, is_ident_start, parse_define, utf8_len,
};
use crate::output::{Emitter, Output};
use crate::resolver::IncludeResolver;

/// Maximum nesting of macro expansions before giving up.
const MAX_EXPANSION_DEPTH: usize = 256;

/// Settings for a [`Preprocessor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Evaluate `__EXEC(...)` and `__EVAL(...)` (configs: config.cpp, description.ext, rvmat,
    /// text mission.sqm). When false (SQF), they are left in the text untouched.
    pub config_macros: bool,
    /// Version reported by the `__GAME_VER*__` / `__GAME_BUILD__` macros.
    pub game_version: GameVersion,
    /// Define `__A3_DEBUG__` (game started with `-debug`).
    pub debug: bool,
    /// Define `__A3_DIAG__` (diag binary).
    pub diag: bool,
    /// Define `__A3_EXPERIMENTAL__` (development/profiling branch).
    pub experimental: bool,
    /// Define `__A3_PROFILING__` (profiling commands available).
    pub profiling: bool,
    /// Maximum `#include` nesting; deeper includes are reported as an error (usually a cycle).
    pub max_include_depth: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            config_macros: false,
            game_version: GameVersion::default(),
            debug: false,
            diag: false,
            experimental: false,
            profiling: false,
            max_include_depth: 32,
        }
    }
}

impl Options {
    /// Options for config files (`__EVAL`/`__EXEC` evaluated).
    pub fn config() -> Self {
        Self {
            config_macros: true,
            ..Self::default()
        }
    }
}

/// One open `#if`/`#ifdef`/`#ifndef` block.
#[derive(Debug, Clone, Copy)]
struct Conditional {
    /// Whether the enclosing code is active.
    parent_active: bool,
    /// Whether the current branch is taken.
    taking: bool,
    /// Whether any branch has been taken.
    taken: bool,
    seen_else: bool,
    opened_at: u32,
}

/// Per-file state while processing.
struct FileState {
    /// Path used for `__FILE__` and relative includes.
    path: Arc<str>,
    /// File name reported in the source map (changed by `#line`).
    map_file: Arc<str>,
    /// Logical line = physical line + offset (changed by `#line`).
    line_offset: i64,
    conditionals: Vec<Conditional>,
}

impl FileState {
    fn active(&self) -> bool {
        self.conditionals
            .last()
            .is_none_or(|c| c.parent_active && c.taking)
    }

    fn logical_line(&self, physical: u32) -> u32 {
        (i64::from(physical) + self.line_offset).max(1) as u32
    }
}

/// Context of a macro expansion: where the expanded text sits in the source.
struct Site<'a> {
    file: &'a FileState,
    /// Logical line for `__LINE__`.
    line: u32,
}

/// The Real Virtuality preprocessor. See the crate docs for the supported language.
pub struct Preprocessor<'r> {
    resolver: &'r dyn IncludeResolver,
    options: Options,
    clock: Box<dyn Clock + 'r>,
    random: Box<dyn RandomSource + 'r>,
    evaluator: Box<dyn Evaluator + 'r>,
    macros: HashMap<String, Arc<Macro>>,
    counter: u64,
    now: Option<Now>,
    warnings: Vec<PreprocessError>,
}

impl std::fmt::Debug for Preprocessor<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preprocessor")
            .field("options", &self.options)
            .field("macros", &self.macros.len())
            .field("counter", &self.counter)
            .finish_non_exhaustive()
    }
}

impl<'r> Preprocessor<'r> {
    /// A preprocessor with default [`Options`], the system clock, a time-seeded random source and
    /// a [`SimpleEvaluator`].
    pub fn new(resolver: &'r dyn IncludeResolver) -> Self {
        Self {
            resolver,
            options: Options::default(),
            clock: Box::new(SystemClock),
            random: Box::new(SplitMix64::from_time()),
            evaluator: Box::new(SimpleEvaluator::new()),
            macros: HashMap::new(),
            counter: 0,
            now: None,
            warnings: Vec::new(),
        }
    }

    /// Replaces the options.
    pub fn with_options(mut self, options: Options) -> Self {
        self.options = options;
        self
    }

    /// Replaces the clock used by the date/time macros.
    pub fn with_clock(mut self, clock: impl Clock + 'r) -> Self {
        self.clock = Box::new(clock);
        self
    }

    /// Replaces the random source used by `__RAND_INT*__` / `__RAND_UINT*__`.
    pub fn with_random(mut self, random: impl RandomSource + 'r) -> Self {
        self.random = Box::new(random);
        self
    }

    /// Replaces the `__EVAL` / `__EXEC` evaluator.
    pub fn with_evaluator(mut self, evaluator: impl Evaluator + 'r) -> Self {
        self.evaluator = Box::new(evaluator);
        self
    }

    /// The evaluator, e.g. to read variables `__EXEC` assigned.
    pub fn evaluator(&self) -> &dyn Evaluator {
        self.evaluator.as_ref()
    }

    /// Defines a macro as if by `#define <definition>`, e.g. `define("DEBUG")`,
    /// `define("VERSION 3")` or `define("ADD(a,b) a+b")`. Returns false if `definition` does not
    /// start with a valid macro name.
    pub fn define(&mut self, definition: &str) -> bool {
        match parse_define(definition) {
            Some((name, definition)) => {
                self.macros.insert(name, Arc::new(definition));
                true
            }
            None => false,
        }
    }

    /// Whether `name` is defined (by `#define` or as a built-in macro).
    pub fn is_defined(&self, name: &str) -> bool {
        self.macros.contains_key(name) || self.is_builtin(name)
    }

    /// Loads `path` through the resolver and preprocesses it.
    pub fn preprocess_file(&mut self, path: &str) -> Result<Output, PreprocessError> {
        let resolved = self
            .resolver
            .resolve("", path)
            .map_err(|source| PreprocessError {
                file: Arc::from(path),
                line: 0,
                kind: ErrorKind::Include {
                    path: path.to_owned(),
                    source,
                },
            })?;
        self.preprocess_str(&resolved.path, &resolved.source)
    }

    /// Preprocesses `source` as the contents of the file at virtual path `path`.
    pub fn preprocess_str(&mut self, path: &str, source: &str) -> Result<Output, PreprocessError> {
        self.now = None;
        self.warnings.clear();
        let mut emitter = Emitter::default();
        self.process_file(Arc::from(path), source, 0, &mut emitter)?;
        let mut output = emitter.finish(std::mem::take(&mut self.warnings));
        if self.options.config_macros {
            self.evaluate_config_macros(&mut output)?;
        }
        Ok(output)
    }

    fn process_file(
        &mut self,
        path: Arc<str>,
        source: &str,
        depth: usize,
        out: &mut Emitter,
    ) -> Result<(), PreprocessError> {
        let lines = split_lines(source);
        let mut file = FileState {
            map_file: path.clone(),
            path,
            line_offset: 0,
            conditionals: Vec::new(),
        };
        let mut i = 0;
        while i < lines.len() {
            if lines[i].is_directive() {
                i = self.directive(&lines, i, &mut file, depth, out)?;
                continue;
            }
            if !file.active() {
                i += 1;
                continue;
            }
            let start = i;
            let mut run = String::new();
            // Line breaks removed by `\` continuations, re-emitted at the next real break so
            // output lines stay aligned with source lines.
            let mut spliced = 0;
            while i < lines.len() && !lines[i].is_directive() {
                let line = &lines[i];
                let text = if line.starts_in_string || line.starts_in_comment {
                    line.text.as_str()
                } else {
                    // The engine drops indentation at the start of every line.
                    line.text.trim_start_matches([' ', '\t'])
                };
                let next_in_string = lines.get(i + 1).is_some_and(|l| l.starts_in_string);
                if line.has_newline && !next_in_string && text.ends_with('\\') {
                    // `\` + line break continues the line, outside `#define` too (configs use
                    // it for multi-line `__EXEC`).
                    run.push_str(&text[..text.len() - 1]);
                    spliced += 1;
                } else {
                    run.push_str(text);
                    if line.has_newline {
                        for _ in 0..=spliced {
                            run.push('\n');
                        }
                        spliced = 0;
                    }
                }
                i += 1;
            }
            for _ in 0..spliced {
                run.push('\n');
            }
            let first_line = file.logical_line(start as u32 + 1);
            let expanded = self.expand_top(&run, &file, first_line)?;
            out.push(&expanded, &file.map_file, first_line);
        }
        if let Some(open) = file.conditionals.last() {
            return Err(error(
                &file,
                lines.len() as u32,
                ErrorKind::UnterminatedConditional {
                    opened_at: open.opened_at,
                },
            ));
        }
        Ok(())
    }

    /// Handles the directive on line `index`; returns the index of the next unprocessed line.
    fn directive(
        &mut self,
        lines: &[Line],
        index: usize,
        file: &mut FileState,
        depth: usize,
        out: &mut Emitter,
    ) -> Result<usize, PreprocessError> {
        let physical = index as u32 + 1;
        let line_no = file.logical_line(physical);
        let line = &lines[index];
        let text = line.text.trim_start_matches([' ', '\t']);
        let after_hash = text[1..].trim_start_matches([' ', '\t']);
        let name_len = ident_len(after_hash);
        let name = &after_hash[..name_len];
        let rest = &after_hash[name_len..];
        let active = file.active();
        let newline = |out: &mut Emitter, file: &FileState, l: &Line, line_no: u32| {
            if l.has_newline {
                out.push("\n", &file.map_file, line_no);
            }
        };

        match name {
            "define" => {
                let mut body = rest.to_owned();
                let mut last = index;
                loop {
                    let continues = if body.ends_with('\\') {
                        body.pop();
                        true
                    } else {
                        lines[last].ends_in_comment
                    };
                    if !continues || last + 1 >= lines.len() {
                        break;
                    }
                    last += 1;
                    body.push_str(&lines[last].text);
                }
                if active {
                    let (name, definition) = parse_define(&body).ok_or_else(|| {
                        error(
                            file,
                            line_no,
                            ErrorKind::MalformedDirective {
                                directive: "define",
                                reason: "expected a macro name".to_owned(),
                            },
                        )
                    })?;
                    self.macros.insert(name, Arc::new(definition));
                    for (k, l) in lines.iter().enumerate().take(last + 1).skip(index) {
                        newline(out, file, l, file.logical_line(k as u32 + 1));
                    }
                }
                return Ok(last + 1);
            }
            "ifdef" | "ifndef" => {
                let operand = rest.trim_start_matches([' ', '\t']);
                let len = ident_len(operand);
                if len == 0 && active {
                    return Err(error(
                        file,
                        line_no,
                        ErrorKind::MalformedDirective {
                            directive: if name == "ifdef" { "ifdef" } else { "ifndef" },
                            reason: "expected a macro name".to_owned(),
                        },
                    ));
                }
                let defined = self.is_defined(&operand[..len]);
                let taking = defined == (name == "ifdef");
                file.conditionals.push(Conditional {
                    parent_active: active,
                    taking,
                    taken: taking,
                    seen_else: false,
                    opened_at: line_no,
                });
                if file.active() {
                    self.emit_line_rest(&operand[len..], line, file, line_no, out)?;
                }
            }
            "if" => {
                let taking = active && self.evaluate_condition(rest, file, line_no)?;
                file.conditionals.push(Conditional {
                    parent_active: active,
                    taking,
                    taken: taking,
                    seen_else: false,
                    opened_at: line_no,
                });
                if file.active() {
                    newline(out, file, line, line_no);
                }
            }
            "else" => {
                let Some(open) = file.conditionals.last_mut() else {
                    return Err(error(
                        file,
                        line_no,
                        ErrorKind::UnmatchedConditional("else"),
                    ));
                };
                if open.seen_else {
                    return Err(error(file, line_no, ErrorKind::DuplicateElse));
                }
                open.seen_else = true;
                open.taking = !open.taken;
                open.taken = true;
                if file.active() {
                    let rest = rest.trim_start_matches([' ', '\t']);
                    self.emit_line_rest(rest, line, file, line_no, out)?;
                }
            }
            "endif" => {
                if file.conditionals.pop().is_none() {
                    return Err(error(
                        file,
                        line_no,
                        ErrorKind::UnmatchedConditional("endif"),
                    ));
                }
                if file.active() {
                    let rest = rest.trim_start_matches([' ', '\t']);
                    self.emit_line_rest(rest, line, file, line_no, out)?;
                }
            }
            _ if !active => {}
            "undef" => {
                let operand = rest.trim_start_matches([' ', '\t']);
                let len = ident_len(operand);
                self.macros.remove(&operand[..len]);
                newline(out, file, line, line_no);
            }
            "include" => {
                let target = parse_include_target(rest).ok_or_else(|| {
                    error(
                        file,
                        line_no,
                        ErrorKind::MalformedDirective {
                            directive: "include",
                            reason: "expected \"path\" or <path>".to_owned(),
                        },
                    )
                })?;
                if depth + 1 > self.options.max_include_depth {
                    return Err(error(
                        file,
                        line_no,
                        ErrorKind::IncludeDepth(self.options.max_include_depth),
                    ));
                }
                let resolved = self
                    .resolver
                    .resolve(&file.path, target)
                    .map_err(|source| {
                        error(
                            file,
                            line_no,
                            ErrorKind::Include {
                                path: target.to_owned(),
                                source,
                            },
                        )
                    })?;
                self.process_file(Arc::from(resolved.path), &resolved.source, depth + 1, out)?;
                newline(out, file, line, line_no);
            }
            "line" => {
                let operand = rest.trim();
                let (number, name) = operand
                    .split_once([' ', '\t'])
                    .map_or((operand, ""), |(n, f)| (n, f.trim()));
                let number: i64 = number.parse().map_err(|_| {
                    error(
                        file,
                        line_no,
                        ErrorKind::MalformedDirective {
                            directive: "line",
                            reason: format!("expected a line number, got `{number}`"),
                        },
                    )
                })?;
                newline(out, file, line, line_no);
                // The line after the directive becomes line `number`.
                file.line_offset = number - (i64::from(physical) + 1);
                let name = name.trim_matches('"');
                if !name.is_empty() {
                    file.map_file = Arc::from(name);
                }
            }
            "pragma" => newline(out, file, line, line_no),
            other => {
                return Err(error(
                    file,
                    line_no,
                    ErrorKind::UnknownDirective(other.to_owned()),
                ));
            }
        }
        Ok(index + 1)
    }

    /// Emits the text following a conditional directive on its line, plus the line break.
    fn emit_line_rest(
        &mut self,
        rest: &str,
        line: &Line,
        file: &FileState,
        line_no: u32,
        out: &mut Emitter,
    ) -> Result<(), PreprocessError> {
        let mut text = self.expand_top(rest, file, line_no)?;
        if line.has_newline {
            text.push('\n');
        }
        out.push(&text, &file.map_file, line_no);
        Ok(())
    }

    fn evaluate_condition(
        &mut self,
        condition: &str,
        file: &FileState,
        line_no: u32,
    ) -> Result<bool, PreprocessError> {
        let condition = condition.trim();
        let invalid = || {
            error(
                file,
                line_no,
                ErrorKind::InvalidCondition(condition.to_owned()),
            )
        };
        if let Some(call) = condition.strip_prefix("__has_include") {
            let target = call
                .trim()
                .strip_prefix('(')
                .and_then(|c| c.trim_end().strip_suffix(')'))
                .and_then(parse_include_target)
                .ok_or_else(invalid)?;
            // Only absolute paths are looked up; anything else is silently false.
            return Ok(target.starts_with('\\') && self.resolver.exists(&file.path, target));
        }
        let (left, operator, right) = match find_operator(condition) {
            Some((at, op)) => (&condition[..at], Some(op), &condition[at + op.len()..]),
            None => (condition, None, ""),
        };
        let site = Site {
            file,
            line: line_no,
        };
        let left = self.expand(left, &site, &mut Vec::new(), 0)?;
        let left = left.trim();
        let Some(operator) = operator else {
            return Ok(left.parse::<f64>().is_ok_and(|v| v != 0.0));
        };
        let right = self.expand(right, &site, &mut Vec::new(), 0)?;
        let right = right.trim();
        let numbers = left.parse::<f64>().ok().zip(right.parse::<f64>().ok());
        Ok(match (operator, numbers) {
            ("==", Some((a, b))) => a == b,
            ("!=", Some((a, b))) => a != b,
            ("<", Some((a, b))) => a < b,
            (">", Some((a, b))) => a > b,
            ("<=", Some((a, b))) => a <= b,
            (">=", Some((a, b))) => a >= b,
            ("==", None) => left == right,
            ("!=", None) => left != right,
            _ => return Err(invalid()),
        })
    }

    /// Expands a run of source text, tracking newlines for `__LINE__`.
    fn expand_top(
        &mut self,
        text: &str,
        file: &FileState,
        first_line: u32,
    ) -> Result<String, PreprocessError> {
        if !text.contains('\n') {
            let site = Site {
                file,
                line: first_line,
            };
            return self.expand(text, &site, &mut Vec::new(), 0);
        }
        // Expand line-aware: split only where no macro call or string spans the break, which is
        // found by expanding chunk by chunk and extending a chunk when it ends mid-call.
        let mut out = String::with_capacity(text.len());
        let mut line = first_line;
        let mut start = 0;
        while start < text.len() {
            let site = Site { file, line };
            let (chunk_end, expanded) = self.expand_chunk(text, start, &site)?;
            line += text[start..chunk_end].matches('\n').count() as u32;
            out.push_str(&expanded);
            start = chunk_end;
        }
        Ok(out)
    }

    /// Expands from `start` up to and including the next newline that is not inside a string or
    /// macro call. Returns the end offset and the expansion.
    fn expand_chunk(
        &mut self,
        text: &str,
        start: usize,
        site: &Site<'_>,
    ) -> Result<(usize, String), PreprocessError> {
        let mut out = String::new();
        let mut disabled = Vec::new();
        let end = self.scan(text, start, site, &mut disabled, 0, &mut out, true)?;
        Ok((end, out))
    }

    /// Fully expands `text` (no line tracking).
    fn expand(
        &mut self,
        text: &str,
        site: &Site<'_>,
        disabled: &mut Vec<String>,
        depth: usize,
    ) -> Result<String, PreprocessError> {
        if depth > MAX_EXPANSION_DEPTH {
            return Err(error(
                site.file,
                site.line,
                ErrorKind::ExpansionDepth(MAX_EXPANSION_DEPTH),
            ));
        }
        let mut out = String::with_capacity(text.len());
        self.scan(text, 0, site, disabled, depth, &mut out, false)?;
        Ok(out)
    }

    /// The expansion scanner. Copies `text[start..]` to `out`, replacing macros. With
    /// `stop_at_newline`, stops after the first newline outside strings and macro calls and
    /// returns its end offset.
    #[allow(clippy::too_many_arguments)]
    fn scan(
        &mut self,
        text: &str,
        start: usize,
        site: &Site<'_>,
        disabled: &mut Vec<String>,
        depth: usize,
        out: &mut String,
        stop_at_newline: bool,
    ) -> Result<usize, PreprocessError> {
        let bytes = text.as_bytes();
        let mut i = start;
        while i < bytes.len() {
            let c = bytes[i];
            match c {
                b'"' => {
                    let end = text[i + 1..]
                        .find('"')
                        .map_or(text.len(), |e| i + 1 + e + 1);
                    out.push_str(&text[i..end]);
                    i = end;
                }
                b'\n' => {
                    out.push('\n');
                    i += 1;
                    if stop_at_newline {
                        return Ok(i);
                    }
                }
                c if is_ident_start(c) => {
                    let len = ident_len(&text[i..]);
                    let word = &text[i..i + len];
                    i += len;
                    match self.expand_word(word, text, &mut i, site, disabled, depth)? {
                        Some(expansion) => out.push_str(&expansion),
                        None => out.push_str(word),
                    }
                }
                c if c.is_ascii_digit() => {
                    let len = bytes[i..].iter().take_while(|&&c| is_ident_char(c)).count();
                    out.push_str(&text[i..i + len]);
                    i += len;
                }
                _ => {
                    let len = utf8_len(c);
                    out.push_str(&text[i..i + len]);
                    i += len;
                }
            }
        }
        Ok(bytes.len())
    }

    /// Expands `word` if it names a macro. `pos` points just after the word and is advanced past
    /// a consumed argument list.
    fn expand_word(
        &mut self,
        word: &str,
        text: &str,
        pos: &mut usize,
        site: &Site<'_>,
        disabled: &mut Vec<String>,
        depth: usize,
    ) -> Result<Option<String>, PreprocessError> {
        if disabled.iter().any(|d| d == word) {
            return Ok(None);
        }
        let Some(definition) = self.macros.get(word).cloned() else {
            return Ok(self.builtin(word, site));
        };
        let Some(params) = &definition.params else {
            let body: String = definition
                .body
                .iter()
                .map(|piece| match piece {
                    Piece::Text(t) => t.as_str(),
                    Piece::Param(_) | Piece::Stringify(_) => "",
                })
                .collect();
            disabled.push(word.to_owned());
            let result = self.expand(&body, site, disabled, depth + 1);
            disabled.pop();
            return result.map(Some);
        };
        if text.as_bytes().get(*pos) != Some(&b'(') {
            return Ok(None);
        }
        let macros = &self.macros;
        let is_call = |name: &str| {
            macros.get(name).is_some_and(|m| m.params.is_some())
                && !disabled.iter().any(|d| d == name)
        };
        let (mut args, end) = split_arguments(text, *pos + 1, is_call).ok_or_else(|| {
            error(
                site.file,
                site.line,
                ErrorKind::UnterminatedMacroCall(word.to_owned()),
            )
        })?;
        *pos = end;
        if params.is_empty() && args.len() == 1 && args[0].is_empty() {
            args.clear();
        }
        if args.len() != params.len() {
            // The engine reports this but carries on; the call expands to nothing. Shipped
            // scripts rely on it (e.g. a 1-parameter macro called with 2 arguments).
            self.warnings.push(error(
                site.file,
                site.line,
                ErrorKind::MacroArgCount {
                    name: word.to_owned(),
                    expected: params.len(),
                    found: args.len(),
                },
            ));
            return Ok(Some(String::new()));
        }
        let mut expanded_args = Vec::with_capacity(args.len());
        for arg in &args {
            expanded_args.push(self.expand(arg, site, disabled, depth + 1)?);
        }
        let mut body = String::new();
        for piece in &definition.body {
            match piece {
                Piece::Text(t) => body.push_str(t),
                Piece::Param(index) => body.push_str(&expanded_args[*index]),
                Piece::Stringify(index) => {
                    body.push('"');
                    body.push_str(&expanded_args[*index]);
                    body.push('"');
                }
            }
        }
        disabled.push(word.to_owned());
        let result = self.expand(&body, site, disabled, depth + 1);
        disabled.pop();
        result.map(Some)
    }

    fn is_builtin(&self, name: &str) -> bool {
        match name {
            "__LINE__"
            | "__FILE__"
            | "__FILE_NAME__"
            | "__FILE_SHORT__"
            | "__COUNTER__"
            | "__COUNTER_RESET__"
            | "__DATE_ARR__"
            | "__DATE_STR__"
            | "__DATE_STR_ISO8601__"
            | "__TIME__"
            | "__TIME_UTC__"
            | "__DAY__"
            | "__MONTH__"
            | "__YEAR__"
            | "__TIMESTAMP_UTC__"
            | "__RAND_INT8__"
            | "__RAND_INT16__"
            | "__RAND_INT32__"
            | "__RAND_INT64__"
            | "__RAND_UINT8__"
            | "__RAND_UINT16__"
            | "__RAND_UINT32__"
            | "__RAND_UINT64__"
            | "__GAME_VER__"
            | "__GAME_VER_MAJ__"
            | "__GAME_VER_MIN__"
            | "__GAME_BUILD__"
            | "__ARMA__"
            | "__ARMA3__" => true,
            "__A3_DEBUG__" => self.options.debug,
            "__A3_DIAG__" => self.options.diag,
            "__A3_EXPERIMENTAL__" => self.options.experimental,
            "__A3_PROFILING__" => self.options.profiling,
            _ => false,
        }
    }

    fn now(&mut self) -> Now {
        *self.now.get_or_insert_with(|| self.clock.now())
    }

    /// The expansion of built-in macro `name`, if it is one.
    fn builtin(&mut self, name: &str, site: &Site<'_>) -> Option<String> {
        if !self.is_builtin(name) {
            return None;
        }
        let version = self.options.game_version;
        let file_name = || {
            let path = &site.file.path;
            path.rsplit(['\\', '/']).next().unwrap_or(path).to_owned()
        };
        Some(match name {
            "__LINE__" => site.line.to_string(),
            "__FILE__" => format!("\"{}\"", site.file.path.trim_start_matches('\\')),
            "__FILE_NAME__" => format!("\"{}\"", file_name()),
            "__FILE_SHORT__" => {
                let name = file_name();
                let short = name.rfind('.').map_or(name.as_str(), |dot| &name[..dot]);
                format!("\"{short}\"")
            }
            "__COUNTER__" => {
                let value = self.counter;
                self.counter += 1;
                value.to_string()
            }
            "__COUNTER_RESET__" => {
                self.counter = 0;
                String::new()
            }
            "__DATE_ARR__" => {
                let t = self.now().local;
                format!(
                    "{},{},{},{},{},{}",
                    t.year, t.month, t.day, t.hour, t.minute, t.second
                )
            }
            "__DATE_STR__" => {
                let t = self.now().local;
                format!(
                    "\"{}/{:02}/{:02}, {:02}:{:02}:{:02}\"",
                    t.year, t.month, t.day, t.hour, t.minute, t.second
                )
            }
            "__DATE_STR_ISO8601__" => {
                let t = self.now().utc;
                format!(
                    "\"{}-{:02}-{:02}T{:02}:{:02}:{:02}Z\"",
                    t.year, t.month, t.day, t.hour, t.minute, t.second
                )
            }
            "__TIME__" => {
                let t = self.now().local;
                format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second)
            }
            "__TIME_UTC__" => {
                let t = self.now().utc;
                format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second)
            }
            "__DAY__" => self.now().utc.day.to_string(),
            "__MONTH__" => self.now().utc.month.to_string(),
            "__YEAR__" => self.now().utc.year.to_string(),
            "__TIMESTAMP_UTC__" => self.now().unix.to_string(),
            "__RAND_INT8__" => (self.random.next_u64() as i8).to_string(),
            "__RAND_INT16__" => (self.random.next_u64() as i16).to_string(),
            "__RAND_INT32__" => (self.random.next_u64() as i32).to_string(),
            "__RAND_INT64__" => (self.random.next_u64() as i64).to_string(),
            "__RAND_UINT8__" => (self.random.next_u64() as u8).to_string(),
            "__RAND_UINT16__" => (self.random.next_u64() as u16).to_string(),
            "__RAND_UINT32__" => (self.random.next_u64() as u32).to_string(),
            "__RAND_UINT64__" => self.random.next_u64().to_string(),
            "__GAME_VER__" => format!(
                "{:02}.{:02}.{}",
                version.major, version.minor, version.build
            ),
            "__GAME_VER_MAJ__" => format!("{:02}", version.major),
            "__GAME_VER_MIN__" => format!("{:02}", version.minor),
            "__GAME_BUILD__" => version.build.to_string(),
            _ => "1".to_owned(),
        })
    }

    /// Runs `__EXEC(...)` and `__EVAL(...)` over the finished text, in order, as the config
    /// parser does.
    fn evaluate_config_macros(&mut self, output: &mut Output) -> Result<(), PreprocessError> {
        let text = &output.text;
        if !text.contains("__EXEC") && !text.contains("__EVAL") {
            return Ok(());
        }
        let bytes = text.as_bytes();
        let mut result = String::with_capacity(text.len());
        let mut i = 0;
        let mut copied = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'"' {
                i = text[i + 1..]
                    .find('"')
                    .map_or(text.len(), |e| i + 1 + e + 1);
                continue;
            }
            if !is_ident_start(c) {
                i += 1;
                continue;
            }
            let len = ident_len(&text[i..]);
            let word = &text[i..i + len];
            let is_exec = word == "__EXEC";
            if !(is_exec || word == "__EVAL") || bytes.get(i + len) != Some(&b'(') {
                i += len;
                continue;
            }
            let macro_name = if is_exec { "__EXEC" } else { "__EVAL" };
            let open = i + len + 1;
            let line = text[..i].matches('\n').count() + 1;
            let locate = |kind: ErrorKind| {
                let location = output.source_map.location(line);
                PreprocessError {
                    file: location.map_or_else(|| Arc::from(""), |l| l.file.clone()),
                    line: location.map_or(0, |l| l.line),
                    kind,
                }
            };
            // `__EXEC` ends at the first `)`; `__EVAL` balances parentheses.
            let close = if is_exec {
                text[open..].find(')').map(|e| open + e)
            } else {
                find_balanced_close(text, open)
            }
            .ok_or_else(|| locate(ErrorKind::UnterminatedEvaluation(macro_name)))?;
            let code = &text[open..close];
            let replacement = if is_exec {
                self.evaluator.exec(code).map(|()| String::new())
            } else {
                self.evaluator.eval(code).map(|value| match value {
                    EvalValue::Number(n) => format_number(n),
                    EvalValue::String(s) => format!("\"{}\"", s.replace('"', "\"\"")),
                })
            }
            .map_err(|message| {
                locate(ErrorKind::Evaluation {
                    macro_name,
                    message,
                })
            })?;
            result.push_str(&text[copied..i]);
            result.push_str(&replacement);
            // Keep the line structure intact.
            for _ in 0..code.matches('\n').count() {
                result.push('\n');
            }
            i = close + 1;
            copied = i;
        }
        result.push_str(&text[copied..]);
        output.text = result;
        Ok(())
    }
}

fn error(file: &FileState, line: u32, kind: ErrorKind) -> PreprocessError {
    PreprocessError {
        file: file.path.clone(),
        line,
        kind,
    }
}

/// Extracts `path` from `"path"` or `<path>` (surrounding blanks allowed).
fn parse_include_target(text: &str) -> Option<&str> {
    let text = text.trim();
    let (open, close) = match text.as_bytes().first()? {
        b'"' => ('"', '"'),
        b'<' => ('<', '>'),
        _ => return None,
    };
    let inner = &text[open.len_utf8()..];
    let end = inner.find(close)?;
    Some(&inner[..end])
}

/// Finds the first comparison operator outside double quotes.
fn find_operator(condition: &str) -> Option<(usize, &'static str)> {
    let bytes = condition.as_bytes();
    let mut in_string = false;
    for (i, &c) in bytes.iter().enumerate() {
        if c == b'"' {
            in_string = !in_string;
        }
        if in_string {
            continue;
        }
        let next = bytes.get(i + 1).copied();
        match (c, next) {
            (b'=', Some(b'=')) => return Some((i, "==")),
            (b'!', Some(b'=')) => return Some((i, "!=")),
            (b'<', Some(b'=')) => return Some((i, "<=")),
            (b'>', Some(b'=')) => return Some((i, ">=")),
            (b'<', _) => return Some((i, "<")),
            (b'>', _) => return Some((i, ">")),
            _ => {}
        }
    }
    None
}

/// Index of the `)` closing the parenthesis opened just before `open` (quote-aware).
fn find_balanced_close(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 1usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = text[i + 1..].find('"').map(|e| i + 1 + e)?;
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Splits a macro call's arguments, starting just after the opening parenthesis. Returns the raw
/// arguments and the offset just past the closing parenthesis.
///
/// Engine quirks: only top-level commas separate arguments, double-quoted strings and nested
/// parentheses protect commas from splitting, but such protected commas are *dropped* from the
/// argument (`M("a, b")` passes `"a b"`). Brackets and braces do not protect commas. A nested call
/// of a function-like macro (`is_call`) is taken verbatim, its commas intact, so
/// `F(G(a,b),c)` works while `F(g(a,b),c)` passes `g(ab)`.
fn split_arguments(
    text: &str,
    start: usize,
    is_call: impl Fn(&str) -> bool,
) -> Option<(Vec<String>, usize)> {
    let bytes = text.as_bytes();
    let mut args = vec![String::new()];
    let mut depth = 1usize;
    let mut in_string = false;
    let mut i = start;
    let mut run = start;
    let flush = |args: &mut Vec<String>, from: usize, to: usize| {
        if let Some(last) = args.last_mut() {
            last.push_str(&text[from..to]);
        }
    };
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            match c {
                b'"' => in_string = false,
                b',' => {
                    flush(&mut args, run, i);
                    run = i + 1;
                }
                _ => {}
            }
            i += 1;
            continue;
        }
        if is_ident_start(c) && (i == start || !is_ident_char(bytes[i - 1])) {
            let len = ident_len(&text[i..]);
            if bytes.get(i + len) == Some(&b'(') && is_call(&text[i..i + len]) {
                i = find_balanced_close(text, i + len + 1)? + 1;
                continue;
            }
            i += len;
            continue;
        }
        match c {
            b'"' => in_string = true,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    flush(&mut args, run, i);
                    return Some((args, i + 1));
                }
            }
            b',' => {
                flush(&mut args, run, i);
                run = i + 1;
                if depth == 1 {
                    args.push(String::new());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}
