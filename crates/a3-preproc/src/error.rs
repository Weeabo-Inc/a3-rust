//! Error types.

use std::sync::Arc;

/// A preprocessing failure, located at the file and line where it was detected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{file}:{line}: {kind}")]
pub struct PreprocessError {
    /// Virtual path of the file being processed when the error happened.
    pub file: Arc<str>,
    /// 1-based line in `file`.
    pub line: u32,
    /// What went wrong.
    pub kind: ErrorKind,
}

/// The kinds of preprocessing failure.
///
/// Where the original engine reports a numbered "Preprocessor failed ... error N", the variant docs
/// name that number.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A `#` directive the preprocessor does not know (engine error 7). Includes `#elif`, which
    /// Arma 3 2.22 does not support.
    #[error("unknown directive `#{0}`")]
    UnknownDirective(String),
    /// A directive with missing or malformed arguments.
    #[error("malformed `#{directive}`: {reason}")]
    MalformedDirective {
        /// Directive name without `#`.
        directive: &'static str,
        /// Human-readable detail.
        reason: String,
    },
    /// `#else` or `#endif` without an open conditional (engine error 6).
    #[error("`#{0}` without a matching `#if`/`#ifdef`/`#ifndef`")]
    UnmatchedConditional(&'static str),
    /// A second `#else` in the same conditional.
    #[error("second `#else` in one conditional block")]
    DuplicateElse,
    /// The file ended inside a conditional block.
    #[error("missing `#endif` for conditional opened on line {opened_at}")]
    UnterminatedConditional {
        /// Line of the opening directive.
        opened_at: u32,
    },
    /// An `#if` condition that cannot be evaluated.
    #[error("invalid `#if` condition `{0}`")]
    InvalidCondition(String),
    /// The include resolver failed.
    #[error("cannot include `{path}`: {source}")]
    Include {
        /// The path as written in the directive.
        path: String,
        /// Resolver error.
        source: IncludeError,
    },
    /// Includes nested deeper than [`crate::Options::max_include_depth`] (usually a cycle).
    #[error("includes nested deeper than {0} levels")]
    IncludeDepth(usize),
    /// A function-like macro was called with the wrong number of arguments. Reported as a
    /// warning in [`crate::Output::warnings`]; the call expands to nothing.
    #[error("macro `{name}` expects {expected} argument(s), got {found}")]
    MacroArgCount {
        /// Macro name.
        name: String,
        /// Declared parameter count.
        expected: usize,
        /// Arguments supplied.
        found: usize,
    },
    /// A function-like macro call whose closing parenthesis is missing.
    #[error("unterminated call of macro `{0}`")]
    UnterminatedMacroCall(String),
    /// Macro expansion nested too deeply.
    #[error("macro expansion nested deeper than {0} levels")]
    ExpansionDepth(usize),
    /// `__EVAL` / `__EXEC` failed in the evaluator.
    #[error("{macro_name} failed: {message}")]
    Evaluation {
        /// `__EVAL` or `__EXEC`.
        macro_name: &'static str,
        /// Evaluator message.
        message: String,
    },
    /// `__EVAL(` / `__EXEC(` without a closing parenthesis.
    #[error("unterminated `{0}(`")]
    UnterminatedEvaluation(&'static str),
}

/// Failure reported by an [`crate::IncludeResolver`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IncludeError {
    /// No file exists at the resolved path.
    #[error("file `{0}` not found")]
    NotFound(String),
    /// The file exists but could not be read.
    #[error("cannot read `{path}`: {message}")]
    Io {
        /// The resolved path.
        path: String,
        /// The underlying error message.
        message: String,
    },
}
