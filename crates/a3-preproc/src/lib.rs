//! The Real Virtuality preprocessor.
//!
//! Arma 3 runs this C-like macro stage over config.cpp, description.ext, text mission.sqm, rvmat
//! and every SQF file loaded through `preprocessFile`, `preprocessFileLineNumbers`, `execVM` or
//! `compileScript`. It resembles the C preprocessor but differs in many details; this crate
//! follows the engine (Arma 3 2.22), not GCC.
//!
//! ```
//! use a3_preproc::{MemoryResolver, Preprocessor};
//!
//! let files = MemoryResolver::new()
//!     .with_file("\\tag\\main\\script_macros.hpp", "#define GVAR(x) tag_main_##x");
//! let mut pp = Preprocessor::new(&files);
//! let out = pp
//!     .preprocess_str("\\tag\\main\\fn_init.sqf", "#include \"script_macros.hpp\"\nGVAR(ready) = true;")
//!     .unwrap();
//! assert_eq!(out.text, "\ntag_main_ready = true;");
//! ```
//!
//! # Supported language
//!
//! - **Comments**: `//` and `/* */` are removed. A block comment keeps its line breaks.
//! - **Strings**: only double quotes delimit strings for the preprocessor. Nothing inside
//!   `"..."` is expanded. Single-quoted SQF strings are ordinary text: macros expand inside
//!   `'...'`, and `//` inside `'...'` starts a comment.
//! - **`#define NAME body`**: the name is `[A-Za-z_][A-Za-z0-9_]*` and may be followed directly by
//!   the body (`#define A#b`). Spaces (not tabs) between name and body are dropped; trailing
//!   blanks are kept. `\` at the end of a line continues the body on the next line (the backslash
//!   and line break disappear, the next line's indentation stays). A `/* */` comment that spans
//!   lines also continues the body. A `//` comment ends the body even if it ends with `\`.
//! - **Function-like macros** `#define F(a, b) body`: parameter names are trimmed. A call needs
//!   `(` directly after the name; a function-like macro named without `(` is left as is.
//!   Arguments are **not** trimmed (`F( x )` passes `" x "`). Arguments are fully
//!   macro-expanded *before* they are substituted (so `#a` and `a##b` see expanded values), then
//!   the body is rescanned. Inside the macro its own name is not expanded again. A call with the
//!   wrong number of arguments is reported in [`Output::warnings`] and expands to nothing: the
//!   engine reports it and carries on, and shipped scripts depend on that.
//! - **`#`** in a body quotes the following identifier: the argument value for a parameter,
//!   otherwise the word itself (`#define M #word` gives `"word"`). The value is wrapped in `"`
//!   without escaping. `#` before anything that is not an identifier is dropped (`#33` gives
//!   `33`). **`##`** is removed, gluing its neighbours.
//! - **`#undef NAME`**.
//! - **`#include "path"`** / **`#include <path>`**: through an [`IncludeResolver`]. Paths
//!   starting with `\` are absolute in the VFS, others are relative to the including file and
//!   may use `..`.
//! - **`#ifdef` / `#ifndef` / `#if` / `#else` / `#endif`**, nestable. `#if` accepts a single
//!   value (true when it is a non-zero number), a comparison `A op B` with `== != < > <= >=`
//!   (numeric when both sides are numbers, otherwise only `==`/`!=` as text comparison), or
//!   `__has_include("\path")` (false for paths not starting with `\`). Macros on both sides are
//!   expanded; an undefined name is not a number, so `#if UNDEFINED` is false.
//! - **`#line N "file"`** renumbers the following lines (`__LINE__` and the source map).
//! - **`#pragma ...`** (including `#pragma hemtt ...`) is ignored.
//! - **Built-in macros**: `__LINE__`, `__FILE__` (quoted path without the leading `\`),
//!   `__FILE_NAME__`, `__FILE_SHORT__` (quoted name without the last extension), `__COUNTER__`,
//!   `__COUNTER_RESET__`, `__DATE_ARR__`, `__DATE_STR__`, `__DATE_STR_ISO8601__`, `__TIME__`,
//!   `__TIME_UTC__`, `__DAY__`, `__MONTH__`, `__YEAR__`, `__TIMESTAMP_UTC__`,
//!   `__RAND_INT{8,16,32,64}__`, `__RAND_UINT{8,16,32,64}__`, `__GAME_VER__` (`02.22.154103`),
//!   `__GAME_VER_MAJ__` (`02`), `__GAME_VER_MIN__` (`22`), `__GAME_BUILD__`, `__ARMA__` and
//!   `__ARMA3__` (`1`), and, only when enabled in [`Options`], `__A3_DEBUG__`, `__A3_DIAG__`,
//!   `__A3_EXPERIMENTAL__`, `__A3_PROFILING__`. Clock and random numbers are injected
//!   ([`Clock`], [`RandomSource`]) for deterministic tests.
//! - **`__EXEC(code)` / `__EVAL(expr)`** (configs only, [`Options::config_macros`]): run in
//!   order over the preprocessed text through an [`Evaluator`]. `__EXEC` ends at the first `)`
//!   (the engine cannot nest parentheses there); `__EVAL` balances them. `__EVAL` becomes a
//!   number or a quoted string.
//!
//! # Output shape
//!
//! The output is what `preprocessFile` returns, and its whitespace matters for SQF string
//! literals:
//!
//! - Every line loses its indentation (leading spaces and tabs), except lines that continue a
//!   multi-line string or comment.
//! - A directive line becomes an empty line; a `#define` spanning N lines becomes N empty lines.
//! - Lines in skipped conditional branches produce **nothing**, not even a line break. A
//!   conditional directive produces its line break only if code after it is active, so output
//!   line numbers drift from the source; [`Output::source_map`] and
//!   [`Output::with_line_directives`] (`preprocessFileLineNumbers`) account for that.
//! - Text after `#ifdef NAME`, `#else` or `#endif` on the same line is kept as ordinary code
//!   (`#endif foo` outputs `foo`).
//! - `\` at the end of an ordinary code line also joins it to the next line (configs write
//!   multi-line `__EXEC` this way, e.g. `\a3\3den\UI\macroExecs.inc`); the removed line breaks
//!   are emitted after the joined line.
//! - `\r\n` becomes `\n`; a leading UTF-8 BOM is dropped.
//!
//! # Quirks reproduced on purpose
//!
//! - Commas protected by a string or nested parentheses inside a macro argument are removed:
//!   `M("a, b")` passes `"a b"` and `M(f(a, b))` passes `f(a b)`. Brackets `[]` and braces `{}`
//!   do not protect commas at all, so `M([1,2])` is a call with two arguments. A nested call of
//!   a function-like macro keeps its commas (CBA's `TRIPLES(DOUBLES(A,B),fnc,x)` works).
//! - Quote escaping is not understood in macro arguments (each `"` toggles the string state).
//! - A `"` inside a single-quoted string opens a double-quoted string for the preprocessor,
//!   which can swallow comment markers and macros that follow.
//! - `STRINGIFY(MACRO)` where `MACRO` expands to `"text"` gives `""text""`.
//!
//! # Divergences from GCC's cpp (all intentional, matching the engine)
//!
//! Single quotes are not strings; arguments are pre-expanded even next to `#`/`##`; `#` works
//! in object-like macros and on non-parameter words; `#` before a non-identifier vanishes
//! instead of being an error; arguments are not trimmed; protected commas are dropped; brackets
//! and braces never group arguments; skipped lines are removed rather than blanked; indentation
//! is removed; a function-like macro call needs `(` right after the name; directives are
//! case-sensitive; `__FILE__` has no leading backslash.
//!
//! # Known gaps and uncertainties
//!
//! - **`#elif`** does not exist in Arma 3 2.22 and is reported as an unknown directive (engine
//!   error 7). `#if` takes no `&&`, `||`, `!` or `defined()`.
//! - **Variadic macros** (`...`, `__VA_ARGS__`, `__VA_OPT__`, `__VA_APPLY__`, `__VA_SELECT__`)
//!   arrive in 2.24 and are not implemented.
//! - The engine output for `\r\n` sources may keep `\r`; we normalize to `\n` _(uncertain)_.
//! - Indentation removal inside a string literal that spans lines is not applied here
//!   _(uncertain what the engine does)_.
//! - A macro whose expansion ends in a function-like macro name does not consume a `(` that
//!   follows the expansion in the source.
//! - Self-referencing macros (`#define X X+1`) stop after one level, as in GCC _(the engine's
//!   behaviour is unverified)_.
//! - The exact `#line` format of `preprocessFileLineNumbers` is `#line N "path"` _(path spelling
//!   unverified)_.
//! - [`SimpleEvaluator`] knows a small SQF subset only; configs using engine commands in
//!   `__EVAL` (e.g. `safeZoneW`, `getResolution`) need the real SQF VM, which will implement
//!   [`Evaluator`].

mod env;
mod error;
mod eval;
mod lines;
mod macros;
mod output;
mod preprocessor;
mod resolver;

pub use env::{
    Clock, DateTime, FixedClock, GameVersion, Now, RandomSource, SplitMix64, SystemClock,
};
pub use error::{ErrorKind, IncludeError, PreprocessError};
pub use eval::{EvalValue, Evaluator, SimpleEvaluator};
pub use output::{Output, SourceLocation, SourceMap};
pub use preprocessor::{Options, Preprocessor};
pub use resolver::{
    FsResolver, IncludeResolver, MemoryResolver, ResolvedInclude, join_virtual_path,
};
