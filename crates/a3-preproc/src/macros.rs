//! Macro definitions: parsing `#define` lines into a body template.

/// A piece of a macro body after `#` / `##` processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Piece {
    /// Literal text.
    Text(String),
    /// The (pre-expanded) value of parameter N.
    Param(usize),
    /// The (pre-expanded) value of parameter N wrapped in double quotes (`#param`).
    Stringify(usize),
}

/// A defined macro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Macro {
    /// `None` for object-like macros, the parameter names for function-like ones.
    pub params: Option<Vec<String>>,
    /// The body template.
    pub body: Vec<Piece>,
}

pub(crate) fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

pub(crate) fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Length of the identifier at the start of `s` (0 if none).
pub(crate) fn ident_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    if bytes.first().is_none_or(|&c| !is_ident_start(c)) {
        return 0;
    }
    bytes.iter().take_while(|&&c| is_ident_char(c)).count()
}

/// Parses the text after `#define` (continuations already joined). Returns the macro name and
/// definition, or `None` if no valid name follows.
pub(crate) fn parse_define(text: &str) -> Option<(String, Macro)> {
    let text = text.trim_start_matches([' ', '\t']);
    let name_len = ident_len(text);
    if name_len == 0 {
        return None;
    }
    let name = text[..name_len].to_owned();
    let mut rest = &text[name_len..];
    let mut params = None;
    if let Some(after_paren) = rest.strip_prefix('(') {
        let close = after_paren.find(')')?;
        let list = &after_paren[..close];
        params = Some(if list.trim().is_empty() {
            Vec::new()
        } else {
            list.split(',').map(|p| p.trim().to_owned()).collect()
        });
        rest = &after_paren[close + 1..];
    }
    // Spaces (not tabs) between the name and the body are swallowed.
    let body_text = rest.trim_start_matches(' ');
    let body = parse_body(body_text, params.as_deref().unwrap_or(&[]));
    Some((name, Macro { params, body }))
}

/// Builds the body template: `##` disappears, `#word` becomes `"word"` (or the stringified
/// argument if `word` is a parameter), a `#` before anything else disappears, and parameter names
/// become [`Piece::Param`]. Double-quoted strings are copied verbatim.
pub(crate) fn parse_body(body: &str, params: &[String]) -> Vec<Piece> {
    let bytes = body.as_bytes();
    let mut pieces = Vec::new();
    let mut text = String::new();
    let param_index = |word: &str| params.iter().position(|p| p == word);
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'"' => {
                let end = body[i + 1..]
                    .find('"')
                    .map_or(body.len(), |e| i + 1 + e + 1);
                text.push_str(&body[i..end]);
                i = end;
            }
            b'#' if bytes.get(i + 1) == Some(&b'#') => i += 2,
            b'#' => {
                i += 1;
                let len = ident_len(&body[i..]);
                if len > 0 {
                    let word = &body[i..i + len];
                    match param_index(word) {
                        Some(index) => {
                            flush(&mut pieces, &mut text);
                            pieces.push(Piece::Stringify(index));
                        }
                        None => {
                            text.push('"');
                            text.push_str(word);
                            text.push('"');
                        }
                    }
                    i += len;
                }
            }
            c if is_ident_start(c) => {
                let len = ident_len(&body[i..]);
                let word = &body[i..i + len];
                match param_index(word) {
                    Some(index) => {
                        flush(&mut pieces, &mut text);
                        pieces.push(Piece::Param(index));
                    }
                    None => text.push_str(word),
                }
                i += len;
            }
            c if c.is_ascii_digit() => {
                let len = bytes[i..].iter().take_while(|&&c| is_ident_char(c)).count();
                text.push_str(&body[i..i + len]);
                i += len;
            }
            _ => {
                let len = utf8_len(c);
                text.push_str(&body[i..i + len]);
                i += len;
            }
        }
    }
    flush(&mut pieces, &mut text);
    pieces
}

fn flush(pieces: &mut Vec<Piece>, text: &mut String) {
    if !text.is_empty() {
        pieces.push(Piece::Text(std::mem::take(text)));
    }
}

/// Byte length of the UTF-8 sequence starting with `lead`.
pub(crate) fn utf8_len(lead: u8) -> usize {
    match lead {
        0xF0..=0xFF => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
}
