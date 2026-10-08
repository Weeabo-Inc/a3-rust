//! Preprocessed text and its source map.

use std::sync::Arc;

use crate::error::PreprocessError;

/// Where an output line came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceLocation {
    /// Virtual path of the source file.
    pub file: Arc<str>,
    /// 1-based line in `file`.
    pub line: u32,
}

/// Maps each output line to its [`SourceLocation`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceMap {
    lines: Vec<SourceLocation>,
}

impl SourceMap {
    /// The source of 1-based output line `line`.
    pub fn location(&self, line: usize) -> Option<&SourceLocation> {
        line.checked_sub(1).and_then(|index| self.lines.get(index))
    }

    /// Number of mapped output lines (equals the number of lines in the output text).
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Whether the map is empty (empty output).
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Locations in output-line order.
    pub fn iter(&self) -> impl Iterator<Item = &SourceLocation> {
        self.lines.iter()
    }
}

/// The result of preprocessing one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The preprocessed text, as `preprocessFile` returns it.
    pub text: String,
    /// Source location of every line of `text`.
    pub source_map: SourceMap,
    /// Problems the engine reports but recovers from (e.g. a macro called with the wrong number
    /// of arguments, which expands to nothing).
    pub warnings: Vec<PreprocessError>,
}

impl Output {
    /// The text with `#line N "file"` directives inserted wherever the next line does not follow
    /// the previous one in the same file, as `preprocessFileLineNumbers` returns it. The SQF
    /// compiler reads these directives to report error locations.
    pub fn with_line_directives(&self) -> String {
        let mut out = String::with_capacity(self.text.len() + 64);
        let mut expected: Option<SourceLocation> = None;
        for (index, line) in self.text.split_inclusive('\n').enumerate() {
            if let Some(location) = self.source_map.lines.get(index) {
                if expected.as_ref() != Some(location) {
                    out.push_str(&format!("#line {} \"{}\"\n", location.line, location.file));
                }
                expected = Some(SourceLocation {
                    file: location.file.clone(),
                    line: location.line + 1,
                });
            }
            out.push_str(line);
        }
        out
    }
}

/// Accumulates output text and the location of each line.
#[derive(Debug, Default)]
pub(crate) struct Emitter {
    text: String,
    lines: Vec<SourceLocation>,
    open: Option<SourceLocation>,
}

impl Emitter {
    /// Appends `text`, whose first line comes from `file:first_line`; each newline in `text`
    /// advances the source line by one.
    pub fn push(&mut self, text: &str, file: &Arc<str>, first_line: u32) {
        let mut line = first_line;
        for segment in text.split_inclusive('\n') {
            if self.open.is_none() {
                self.open = Some(SourceLocation {
                    file: file.clone(),
                    line,
                });
            }
            self.text.push_str(segment);
            if segment.ends_with('\n') {
                self.lines.extend(self.open.take());
                line += 1;
            }
        }
    }

    pub fn finish(mut self, warnings: Vec<PreprocessError>) -> Output {
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.lines.extend(self.open.take());
        }
        Output {
            text: self.text,
            source_map: SourceMap { lines: self.lines },
            warnings,
        }
    }
}
