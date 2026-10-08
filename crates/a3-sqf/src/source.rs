//! Source text and position mapping.
//!
//! `preprocessFileLineNumbers` emits `#line <n> "<file>"` directives so that
//! errors in preprocessed text point back to the original file and line.
//! A [`SourceFile`] keeps the text and the directives the lexer found, and
//! maps a byte offset to a [`Location`].

use std::fmt;
use std::rc::Rc;

/// A half-open byte range in a [`SourceFile`]'s text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span {
            start: start as u32,
            end: end as u32,
        }
    }

    /// The smallest span covering both.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// A position as reported in error messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    /// The file name from the latest `#line` directive, or the name the
    /// source was compiled under.
    pub file: Rc<str>,
    /// 1-based line, adjusted by `#line` directives.
    pub line: u32,
    /// 1-based column in bytes.
    pub column: u32,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.file.is_empty() {
            write!(f, "line {}", self.line)
        } else {
            write!(f, "File {}..., line {}", self.file, self.line)
        }
    }
}

#[derive(Clone, Debug)]
struct LineDirective {
    /// Offset of the first byte of the line after the directive.
    offset: u32,
    /// The line number that line has.
    line: u32,
    file: Rc<str>,
}

/// Source text being compiled, with its `#line` mapping.
#[derive(Debug)]
pub struct SourceFile {
    name: Rc<str>,
    text: Rc<str>,
    line_starts: Vec<u32>,
    directives: Vec<LineDirective>,
}

impl SourceFile {
    /// Wraps `text` compiled under `name` (empty for an anonymous string).
    pub fn new(name: impl Into<Rc<str>>, text: impl Into<Rc<str>>) -> Rc<SourceFile> {
        let text: Rc<str> = text.into();
        let mut line_starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        let name: Rc<str> = name.into();
        let directives = scan_line_directives(&text, &line_starts, &name);
        Rc::new(SourceFile {
            name,
            text,
            line_starts,
            directives,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The text covered by `span`.
    pub fn slice(&self, span: Span) -> &str {
        &self.text[span.start as usize..span.end as usize]
    }

    /// Maps a byte offset to a file, line and column.
    pub fn locate(&self, offset: u32) -> Location {
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let column = offset - self.line_starts[line_idx] + 1;
        let physical_line = line_idx as u32 + 1;
        let dirs = &self.directives;
        let idx = dirs.partition_point(|d| d.offset <= offset);
        if idx == 0 {
            return Location {
                file: self.name.clone(),
                line: physical_line,
                column,
            };
        }
        let d = &dirs[idx - 1];
        let dir_line_idx = match self.line_starts.binary_search(&d.offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        Location {
            file: d.file.clone(),
            line: d.line + (line_idx - dir_line_idx) as u32,
            column,
        }
    }
}

/// Parses a `#line <n> ["file"]` directive line. Returns the line number
/// and optional file name.
pub(crate) fn parse_line_directive(line: &str) -> Option<(u32, Option<&str>)> {
    let rest = line.trim_start().strip_prefix("#line")?;
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let rest = rest.trim_start();
    let digits_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let line_no: u32 = rest[..digits_end].parse().ok()?;
    let rest = rest[digits_end..].trim();
    let file = rest
        .strip_prefix('"')
        .and_then(|r| r.split_once('"'))
        .map(|(f, _)| f);
    Some((line_no, file))
}

fn scan_line_directives(text: &str, line_starts: &[u32], name: &Rc<str>) -> Vec<LineDirective> {
    let mut out: Vec<LineDirective> = Vec::new();
    if !text.contains("#line") {
        return out;
    }
    for (i, &start) in line_starts.iter().enumerate() {
        let end = line_starts
            .get(i + 1)
            .map(|&e| e as usize)
            .unwrap_or(text.len());
        let line = &text[start as usize..end];
        if let Some((line_no, file)) = parse_line_directive(line) {
            let file: Rc<str> = match file {
                Some(f) => f.into(),
                None => out
                    .last()
                    .map(|d| d.file.clone())
                    .unwrap_or_else(|| name.clone()),
            };
            out.push(LineDirective {
                offset: end as u32,
                line: line_no,
                file,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locate_maps_offsets_to_lines() {
        let src = SourceFile::new("a.sqf", "a;\nbb;\nccc");
        assert_eq!(src.locate(0).line, 1);
        assert_eq!(src.locate(3).line, 2);
        let loc = src.locate(8);
        assert_eq!((loc.line, loc.column), (3, 2));
    }

    #[test]
    fn line_directives_renumber_following_lines() {
        let text = "#line 10 \"x.sqf\"\na;\nb;";
        let src = SourceFile::new("", text);
        let loc = src.locate(20);
        assert_eq!(&*loc.file, "x.sqf");
        assert_eq!(loc.line, 11);
    }
}
