//! First pass: split a file into physical lines and strip comments.
//!
//! Only double quotes delimit strings for the preprocessor. Single-quoted SQF strings are plain
//! text to it: macros expand inside them and `//` inside them starts a comment, as in the engine.

/// One physical source line with comments removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Line {
    /// Line text without comments and without the line terminator.
    pub text: String,
    /// The line begins inside a double-quoted string that started on an earlier line.
    pub starts_in_string: bool,
    /// The line begins inside a `/* */` comment that started on an earlier line.
    pub starts_in_comment: bool,
    /// The line ends inside a `/* */` comment.
    pub ends_in_comment: bool,
    /// The line was terminated by a newline (false only for a final unterminated line).
    pub has_newline: bool,
}

impl Line {
    /// Whether this line is a preprocessor directive (`#` as first non-blank character, not
    /// inside a multi-line string or comment).
    pub fn is_directive(&self) -> bool {
        !self.starts_in_string
            && !self.starts_in_comment
            && self.text.trim_start_matches([' ', '\t']).starts_with('#')
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Code,
    String,
    BlockComment,
}

/// Splits `source` into [`Line`]s. A leading UTF-8 BOM is dropped and `\r\n` counts as one line
/// terminator.
pub(crate) fn split_lines(source: &str) -> Vec<Line> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let bytes = source.as_bytes();
    let mut lines = Vec::new();
    let mut state = State::Code;
    let mut text = String::new();
    let mut starts_in = state;
    let mut i = 0;
    // Start of the not-yet-copied run of code/string text.
    let mut run = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' || (c == b'\r' && bytes.get(i + 1) == Some(&b'\n')) {
            if state != State::BlockComment {
                text.push_str(&source[run..i]);
            }
            lines.push(Line {
                text: std::mem::take(&mut text),
                starts_in_string: starts_in == State::String,
                starts_in_comment: starts_in == State::BlockComment,
                ends_in_comment: state == State::BlockComment,
                has_newline: true,
            });
            i += if c == b'\r' { 2 } else { 1 };
            run = i;
            starts_in = state;
            continue;
        }
        match state {
            State::Code => match c {
                b'"' => state = State::String,
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    text.push_str(&source[run..i]);
                    // Skip to the line terminator, which the loop handles next.
                    while i < bytes.len()
                        && bytes[i] != b'\n'
                        && !(bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n'))
                    {
                        i += 1;
                    }
                    run = i;
                    continue;
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    text.push_str(&source[run..i]);
                    state = State::BlockComment;
                    i += 2;
                    continue;
                }
                _ => {}
            },
            State::String => {
                if c == b'"' {
                    state = State::Code;
                }
            }
            State::BlockComment => {
                if c == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    state = State::Code;
                    i += 2;
                    run = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    if state != State::BlockComment {
        text.push_str(&source[run..]);
    }
    if !text.is_empty() || run < bytes.len() || starts_in != State::Code {
        lines.push(Line {
            text,
            starts_in_string: starts_in == State::String,
            starts_in_comment: starts_in == State::BlockComment,
            ends_in_comment: state == State::BlockComment,
            has_newline: false,
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(source: &str) -> Vec<String> {
        split_lines(source).into_iter().map(|l| l.text).collect()
    }

    #[test]
    fn line_comment_is_removed_up_to_newline() {
        assert_eq!(texts("a // c\nb"), ["a ", "b"]);
    }

    #[test]
    fn block_comment_keeps_its_lines_empty() {
        assert_eq!(texts("a /* x\ny\nz */ b\nc"), ["a ", "", " b", "c"]);
    }

    #[test]
    fn comment_markers_inside_double_quotes_are_kept() {
        assert_eq!(texts("\"//x\" /*y*/"), ["\"//x\" "]);
    }

    #[test]
    fn comment_markers_inside_single_quotes_start_comments() {
        assert_eq!(texts("'http://x'"), ["'http:"]);
    }

    #[test]
    fn crlf_is_one_terminator() {
        let lines = split_lines("a\r\nb\r\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "a");
        assert!(lines[1].has_newline);
    }

    #[test]
    fn multi_line_string_marks_next_line() {
        let lines = split_lines("x = \"a\n  #b\";");
        assert!(lines[1].starts_in_string);
        assert!(!lines[1].is_directive());
    }
}
