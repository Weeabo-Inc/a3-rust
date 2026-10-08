//! SQF tokenizer.
//!
//! Recognises numbers (decimal with optional fraction and exponent, `0x` and
//! `$` hexadecimal), strings in `"` or `'` with the quote doubled to escape
//! it, identifiers, punctuation and operator symbols. Comments (normally
//! removed by the preprocessor) are skipped, as are `#line` directive lines.

use std::rc::Rc;

use crate::error::CompileError;
use crate::source::{Span, parse_line_directive};

/// One token.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Number(f32),
    String(Rc<str>),
    Ident,
    /// An operator symbol that names a command: `+ - * / % ^ # ! == != < >
    /// <= >= >> && || :`.
    Operator,
    Assign,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Semicolon,
    Comma,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Splits `text` into tokens, ending with [`TokenKind::Eof`].
pub fn tokenize(text: &str) -> Result<Vec<Token>, CompileError> {
    Lexer {
        src: text,
        bytes: text.as_bytes(),
        pos: 0,
        out: Vec::new(),
    }
    .run()
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    out: Vec<Token>,
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

impl Lexer<'_> {
    fn peek(&self, ahead: usize) -> u8 {
        self.bytes.get(self.pos + ahead).copied().unwrap_or(0)
    }

    fn push(&mut self, kind: TokenKind, start: usize) {
        self.out.push(Token {
            kind,
            span: Span::new(start, self.pos),
        });
    }

    fn at_line_start(&self) -> bool {
        let before = &self.bytes[..self.pos];
        before
            .iter()
            .rev()
            .take_while(|&&b| b != b'\n')
            .all(|b| b.is_ascii_whitespace())
    }

    fn run(mut self) -> Result<Vec<Token>, CompileError> {
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            let start = self.pos;
            match b {
                b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c => self.pos += 1,
                b'/' if self.peek(1) == b'/' => {
                    while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                b'/' if self.peek(1) == b'*' => {
                    self.pos += 2;
                    while self.pos < self.bytes.len()
                        && !(self.bytes[self.pos] == b'*' && self.peek(1) == b'/')
                    {
                        self.pos += 1;
                    }
                    self.pos = (self.pos + 2).min(self.bytes.len());
                }
                b'#' if self.at_line_start() && self.is_directive_line() => {
                    while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                b'"' | b'\'' => self.string(b)?,
                b'0'..=b'9' => self.number()?,
                b'.' if self.peek(1).is_ascii_digit() => self.number()?,
                b'$' if self.peek(1).is_ascii_hexdigit() => self.hex(1)?,
                _ if is_ident_start(b) => {
                    while self.pos < self.bytes.len() && is_ident_continue(self.bytes[self.pos]) {
                        self.pos += 1;
                    }
                    // Keep identifiers on char boundaries for non-ASCII text.
                    while !self.src.is_char_boundary(self.pos) {
                        self.pos += 1;
                    }
                    self.push(TokenKind::Ident, start);
                }
                b'{' => self.single(TokenKind::LBrace),
                b'}' => self.single(TokenKind::RBrace),
                b'[' => self.single(TokenKind::LBracket),
                b']' => self.single(TokenKind::RBracket),
                b'(' => self.single(TokenKind::LParen),
                b')' => self.single(TokenKind::RParen),
                b';' => self.single(TokenKind::Semicolon),
                b',' => self.single(TokenKind::Comma),
                b'=' if self.peek(1) == b'=' => self.operator(2),
                b'=' => self.single(TokenKind::Assign),
                b'!' | b'<' if self.peek(1) == b'=' => self.operator(2),
                b'>' if self.peek(1) == b'=' || self.peek(1) == b'>' => self.operator(2),
                b'&' if self.peek(1) == b'&' => self.operator(2),
                b'|' if self.peek(1) == b'|' => self.operator(2),
                b'+' | b'-' | b'*' | b'/' | b'%' | b'^' | b'#' | b'!' | b'<' | b'>' | b':' => {
                    self.operator(1)
                }
                _ => {
                    let ch = self.src[self.pos..].chars().next().unwrap_or('?');
                    return Err(CompileError::new(
                        format!("Invalid character '{ch}'"),
                        Span::new(start, start + ch.len_utf8()),
                    ));
                }
            }
        }
        let end = self.bytes.len();
        self.out.push(Token {
            kind: TokenKind::Eof,
            span: Span::new(end, end),
        });
        Ok(self.out)
    }

    fn is_directive_line(&self) -> bool {
        let rest = &self.src[self.pos..];
        let line = rest.split('\n').next().unwrap_or("");
        parse_line_directive(line).is_some()
    }

    fn single(&mut self, kind: TokenKind) {
        let start = self.pos;
        self.pos += 1;
        self.push(kind, start);
    }

    fn operator(&mut self, len: usize) {
        let start = self.pos;
        self.pos += len;
        self.push(TokenKind::Operator, start);
    }

    fn string(&mut self, quote: u8) -> Result<(), CompileError> {
        let start = self.pos;
        self.pos += 1;
        let mut value = String::new();
        let mut seg_start = self.pos;
        loop {
            if self.pos >= self.bytes.len() {
                return Err(CompileError::new(
                    "Missing \"\"",
                    Span::new(start, self.bytes.len()),
                ));
            }
            if self.bytes[self.pos] == quote {
                value.push_str(&self.src[seg_start..self.pos]);
                if self.peek(1) == quote {
                    value.push(quote as char);
                    self.pos += 2;
                    seg_start = self.pos;
                    continue;
                }
                self.pos += 1;
                break;
            }
            self.pos += 1;
        }
        self.push(TokenKind::String(value.into()), start);
        Ok(())
    }

    fn hex(&mut self, prefix: usize) -> Result<(), CompileError> {
        let start = self.pos;
        self.pos += prefix;
        let digits_start = self.pos;
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_hexdigit() {
            self.pos += 1;
        }
        let digits = &self.src[digits_start..self.pos];
        let value = u64::from_str_radix(digits, 16).map_err(|_| {
            CompileError::new("Invalid number in expression", Span::new(start, self.pos))
        })?;
        self.push(TokenKind::Number(value as f32), start);
        Ok(())
    }

    fn number(&mut self) -> Result<(), CompileError> {
        let start = self.pos;
        if self.bytes[self.pos] == b'0'
            && matches!(self.peek(1), b'x' | b'X')
            && self.peek(2).is_ascii_hexdigit()
        {
            return self.hex(2);
        }
        while self.peek(0).is_ascii_digit() {
            self.pos += 1;
        }
        if self.peek(0) == b'.' && self.peek(1).is_ascii_digit() {
            self.pos += 1;
            while self.peek(0).is_ascii_digit() {
                self.pos += 1;
            }
        } else if self.peek(0) == b'.' && !is_ident_start(self.peek(1)) {
            // "1." is a valid number.
            self.pos += 1;
        }
        if matches!(self.peek(0), b'e' | b'E') {
            let save = self.pos;
            self.pos += 1;
            if matches!(self.peek(0), b'+' | b'-') {
                self.pos += 1;
            }
            if self.peek(0).is_ascii_digit() {
                while self.peek(0).is_ascii_digit() {
                    self.pos += 1;
                }
            } else {
                self.pos = save;
            }
        }
        let text = &self.src[start..self.pos];
        let value: f64 = text.parse().map_err(|_| {
            CompileError::new("Invalid number in expression", Span::new(start, self.pos))
        })?;
        self.push(TokenKind::Number(value as f32), start);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        tokenize(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn numbers_in_all_notations() {
        assert_eq!(
            kinds("1 2.5 .5 1e3 1.5e-2 0x1F $ff"),
            vec![
                TokenKind::Number(1.0),
                TokenKind::Number(2.5),
                TokenKind::Number(0.5),
                TokenKind::Number(1000.0),
                TokenKind::Number(0.015),
                TokenKind::Number(31.0),
                TokenKind::Number(255.0),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn strings_with_doubled_quotes() {
        assert_eq!(
            kinds(r#""a""b" 'c''d' "it's""#),
            vec![
                TokenKind::String("a\"b".into()),
                TokenKind::String("c'd".into()),
                TokenKind::String("it's".into()),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn unterminated_string_is_an_error() {
        assert!(tokenize("\"abc").is_err());
    }

    #[test]
    fn operators_and_comments() {
        let toks = tokenize("a >= b // c\n/* d */ >> && ! # =").unwrap();
        let ops: Vec<_> = toks
            .iter()
            .filter(|t| t.kind == TokenKind::Operator)
            .map(|t| {
                &"a >= b // c\n/* d */ >> && ! # ="[t.span.start as usize..t.span.end as usize]
            })
            .collect();
        assert_eq!(ops, vec![">=", ">>", "&&", "!", "#"]);
        assert_eq!(toks[toks.len() - 2].kind, TokenKind::Assign);
    }

    #[test]
    fn line_directives_are_skipped() {
        assert_eq!(
            kinds("#line 1 \"a.sqf\"\nx"),
            vec![TokenKind::Ident, TokenKind::Eof]
        );
        // `#` elsewhere is the select operator.
        assert_eq!(
            kinds("a # 1"),
            vec![
                TokenKind::Ident,
                TokenKind::Operator,
                TokenKind::Number(1.0),
                TokenKind::Eof
            ]
        );
    }
}
