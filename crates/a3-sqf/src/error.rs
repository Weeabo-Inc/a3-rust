//! Compile errors and their engine-style rendering.

use std::rc::Rc;

use crate::source::{SourceFile, Span};

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
