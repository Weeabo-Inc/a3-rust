//! Compile and runtime errors and their engine-style rendering.

use std::rc::Rc;

use crate::source::{Location, SourceFile, Span};
use crate::types::{Type, TypeSet};

/// A syntax error found while tokenizing or parsing.
///
/// `message` follows the engine's wording ("Missing ;", "Missing ]",
/// "Invalid number in expression", ...).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct CompileError {
    pub message: String,
    pub span: Span,
}

impl CompileError {
    pub fn new(message: impl Into<String>, span: Span) -> CompileError {
        CompileError {
            message: message.into(),
            span,
        }
    }

    /// Renders the error the way the engine writes it to the RPT log:
    ///
    /// ```text
    /// Error in expression <_a = [1,2;>
    ///   Error position: <;>
    ///   Error Missing ]
    /// File x.sqf..., line 1
    /// ```
    pub fn render(&self, source: &Rc<SourceFile>) -> String {
        render_error(source, self.span.start, &self.message)
    }
}

/// What went wrong while running a command. The VM adds the position and
/// the command name to make a [`ScriptError`].
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SqfError {
    /// An argument of the wrong type.
    #[error("Type {got}, expected {}", expected.display_list())]
    Type { got: Type, expected: TypeSet },
    /// A `nil` argument read from an undefined variable.
    #[error("Undefined variable in expression: {0}")]
    UndefinedVariable(String),
    #[error("Zero divisor")]
    ZeroDivisor,
    #[error("Suspending not allowed in this context")]
    SuspendNotAllowed,
    /// The command is in the signature table but has no implementation yet.
    #[error("Unimplemented command: {0}")]
    Unimplemented(String),
    /// A `throw` that no `catch` handled.
    #[error("Unhandled exception: {0}")]
    UnhandledException(String),
    /// Any other error, with the engine's message.
    #[error("{0}")]
    Generic(String),
}

impl SqfError {
    /// A type error for `got` when `expected` was wanted.
    pub fn type_error(got: &crate::Value, expected: impl Into<TypeSet>) -> SqfError {
        SqfError::Type {
            got: got.ty(),
            expected: expected.into(),
        }
    }

    pub fn generic(msg: impl Into<String>) -> SqfError {
        SqfError::Generic(msg.into())
    }
}

/// A runtime error with its position, as reported to the host's error sink.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{report}")]
pub struct ScriptError {
    pub error: SqfError,
    /// The command that failed, if the error came from a command.
    pub command: Option<String>,
    pub location: Option<Location>,
    /// The engine-style multi-line report.
    pub report: String,
}

impl ScriptError {
    /// Builds the report for `error` raised by `command` at `offset` in
    /// `source`.
    pub fn new(
        error: SqfError,
        command: Option<&str>,
        source: Option<(&SourceFile, u32)>,
    ) -> ScriptError {
        let message = match (&error, command) {
            (SqfError::Type { .. }, Some(cmd)) => format!("{cmd}: {error}"),
            _ => error.to_string(),
        };
        let (report, location) = match source {
            Some((src, offset)) => (
                render_error(src, offset, &message),
                Some(src.locate(offset)),
            ),
            None => (format!("Error {message}"), None),
        };
        ScriptError {
            error,
            command: command.map(str::to_string),
            location,
            report,
        }
    }
}

/// Engine-style error report for an error at `offset` in `source`.
pub fn render_error(source: &SourceFile, offset: u32, message: &str) -> String {
    let text = source.text();
    let offset = (offset as usize).min(text.len());
    let context_start = floor_boundary(text, offset.saturating_sub(100));
    let context_end = ceil_boundary(text, (offset + 100).min(text.len()));
    let pos_end = ceil_boundary(text, (offset + 50).min(text.len()));
    let loc = source.locate(offset as u32);
    format!(
        "Error in expression <{}>\n  Error position: <{}>\n  Error {}\n{}",
        &text[context_start..context_end],
        &text[offset..pos_end],
        message,
        loc
    )
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}
