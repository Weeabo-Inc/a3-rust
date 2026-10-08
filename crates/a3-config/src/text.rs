//! config.cpp syntax: a parser for already-preprocessed text and a pretty printer.
//!
//! Parsing rules (see `docs/re/config.md` for what is confirmed vs. assumed):
//! - `class Name: Base { ... };`, `class Name;`, `delete Name;`, `name = value;`,
//!   `name[] = {...};`, `name[] += {...};`, `enum { A, B = 2 };`.
//! - A value is a quoted string (`"..."` or `'...'`, the quote doubled to escape it) or raw text up
//!   to the next `;` (or `,`/`}` inside arrays, or end of line). Raw text that is a number literal
//!   becomes a number; anything else becomes a string, as the engine tolerates.
//! - Number literals: decimal integers become `Int` when they fit in i32 and `Int64` when they fit
//!   in i64; `0x` hex integers become `Int`; anything with a `.` or exponent becomes `Float`.
//! - Lines starting with `#` (preprocessor line markers) are skipped. The semicolon after a class
//!   body is optional.

use std::fmt::Write as _;

use crate::{Config, ConfigClass, Entry, EntryKind, EnumEntry, Value};

/// A syntax error with a 1-based line and column.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{line}:{column}: {message}")]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// Parses preprocessed config.cpp text.
pub fn parse_text(src: &str) -> Result<Config, ParseError> {
    let mut p = Parser {
        chars: src.chars().collect(),
        pos: 0,
        line: 1,
        column: 1,
    };
    let mut config = Config::default();
    p.body(&mut config.root, &mut config.enums, true)?;
    Ok(config)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
}

const MAX_DEPTH: usize = 256;

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            line: self.line,
            column: self.column,
            message: message.into(),
        }
    }

    fn at_line_start(&self) -> bool {
        self.chars[..self.pos]
            .iter()
            .rev()
            .take_while(|&&c| c != '\n')
            .all(|c| c.is_whitespace())
    }

    /// Skips whitespace, `//` and `/* */` comments, and `#` lines at the start of a line.
    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    self.bump();
                    self.bump();
                    while self.peek().is_some()
                        && !(self.peek() == Some('*') && self.peek_at(1) == Some('/'))
                    {
                        self.bump();
                    }
                    self.bump();
                    self.bump();
                }
                Some('#') if self.at_line_start() => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                _ => return,
            }
        }
    }

    fn expect(&mut self, c: char, what: &str) -> Result<(), ParseError> {
        self.skip_trivia();
        if self.peek() == Some(c) {
            self.bump();
            Ok(())
        } else {
            Err(self.error(format!("expected {what}, found {}", self.describe())))
        }
    }

    fn describe(&self) -> String {
        match self.peek() {
            None => "end of input".into(),
            Some(c) => format!("`{c}`"),
        }
    }

    fn ident(&mut self, what: &str) -> Result<String, ParseError> {
        self.skip_trivia();
        let start = self.pos;
        while self.peek().is_some_and(is_ident_char) {
            self.bump();
        }
        if start == self.pos {
            return Err(self.error(format!("expected {what}, found {}", self.describe())));
        }
        Ok(self.chars[start..self.pos].iter().collect())
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_trivia();
        if self.peek() == Some(c) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn body(
        &mut self,
        class: &mut ConfigClass,
        enums: &mut Vec<EnumEntry>,
        top: bool,
    ) -> Result<(), ParseError> {
        self.body_at(class, enums, top, 0)
    }

    fn body_at(
        &mut self,
        class: &mut ConfigClass,
        enums: &mut Vec<EnumEntry>,
        top: bool,
        depth: usize,
    ) -> Result<(), ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.error("classes nested too deeply"));
        }
        loop {
            self.skip_trivia();
            match self.peek() {
                None if top => return Ok(()),
                None => return Err(self.error("expected `}` to close class, found end of input")),
                Some('}') if !top => {
                    self.bump();
                    return Ok(());
                }
                Some(';') => {
                    self.bump();
                }
                _ => {
                    let entry = self.statement(enums, depth)?;
                    if let Some(entry) = entry {
                        class.entries.push(entry);
                    }
                }
            }
        }
    }

    fn statement(
        &mut self,
        enums: &mut Vec<EnumEntry>,
        depth: usize,
    ) -> Result<Option<Entry>, ParseError> {
        let name = self.ident("an entry name, `class`, `delete` or `enum`")?;
        if name == "class" {
            let name = self.ident("class name")?;
            let base = if self.eat(':') {
                Some(self.ident("base class name")?)
            } else {
                None
            };
            if self.eat('{') {
                let mut class = ConfigClass {
                    base,
                    entries: Vec::new(),
                };
                self.body_at(&mut class, enums, false, depth + 1)?;
                self.eat(';');
                return Ok(Some(Entry::class(name, class)));
            }
            if base.is_some() {
                return Err(self.error(format!(
                    "expected `{{` to open class body, found {}",
                    self.describe()
                )));
            }
            self.expect(';', "`;` or `{`")?;
            return Ok(Some(Entry::new(name, EntryKind::External)));
        }
        if name == "delete" {
            let name = self.ident("class name to delete")?;
            self.expect(';', "`;`")?;
            return Ok(Some(Entry::new(name, EntryKind::Delete)));
        }
        if name == "enum" && {
            self.skip_trivia();
            self.peek() == Some('{')
        } {
            self.enum_block(enums)?;
            return Ok(None);
        }

        self.skip_trivia();
        if self.eat('[') {
            self.expect(']', "`]`")?;
            self.skip_trivia();
            let append = if self.peek() == Some('+') && self.peek_at(1) == Some('=') {
                self.bump();
                self.bump();
                true
            } else {
                self.expect('=', "`=` or `+=`")?;
                false
            };
            self.skip_trivia();
            let items = if self.peek() == Some('{') {
                self.array(depth)?
            } else {
                return Err(self.error(format!("expected `{{`, found {}", self.describe())));
            };
            self.end_statement()?;
            let kind = if append {
                EntryKind::ArrayAppend(items)
            } else {
                EntryKind::Value(Value::Array(items))
            };
            return Ok(Some(Entry::new(name, kind)));
        }
        self.expect('=', "`=`, `[]` or `;`")?;
        self.skip_trivia();
        let value = if self.peek() == Some('{') {
            // Tolerated: `name = {...};` without `[]`.
            Value::Array(self.array(depth)?)
        } else {
            self.scalar(&[';', '}'])?
        };
        self.end_statement()?;
        Ok(Some(Entry::value(name, value)))
    }

    /// A value statement ends at `;`; a missing `;` before `}` or a new line is tolerated.
    fn end_statement(&mut self) -> Result<(), ParseError> {
        self.skip_trivia();
        match self.peek() {
            Some(';') => {
                self.bump();
                Ok(())
            }
            Some('}') | None => Ok(()),
            _ => Err(self.error(format!("expected `;`, found {}", self.describe()))),
        }
    }

    fn enum_block(&mut self, enums: &mut Vec<EnumEntry>) -> Result<(), ParseError> {
        self.expect('{', "`{`")?;
        let mut next = 0i32;
        loop {
            if self.eat('}') {
                break;
            }
            let name = self.ident("enum constant name")?;
            if self.eat('=') {
                self.skip_trivia();
                let (line, column) = (self.line, self.column);
                next = match self.scalar(&[',', '}'])? {
                    Value::Int(v) => v,
                    other => {
                        return Err(ParseError {
                            line,
                            column,
                            message: format!("enum value must be an integer, found {other:?}"),
                        });
                    }
                };
            }
            enums.push(EnumEntry { name, value: next });
            next = next.wrapping_add(1);
            if !self.eat(',') {
                self.expect('}', "`,` or `}`")?;
                break;
            }
        }
        self.eat(';');
        Ok(())
    }

    fn array(&mut self, depth: usize) -> Result<Vec<Value>, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.error("arrays nested too deeply"));
        }
        self.expect('{', "`{`")?;
        let mut items = Vec::new();
        loop {
            self.skip_trivia();
            if self.eat('}') {
                return Ok(items);
            }
            let item = if self.peek() == Some('{') {
                Value::Array(self.array(depth + 1)?)
            } else {
                self.scalar(&[',', '}', ';'])?
            };
            items.push(item);
            if !self.eat(',') {
                self.expect('}', "`,` or `}`")?;
                return Ok(items);
            }
        }
    }

    /// A quoted string, or raw text up to one of `stops` or end of line.
    fn scalar(&mut self, stops: &[char]) -> Result<Value, ParseError> {
        self.skip_trivia();
        if let Some(q @ ('"' | '\'')) = self.peek() {
            return Ok(Value::String(self.quoted(q)?));
        }
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c != '\n' && !stops.contains(&c))
        {
            self.bump();
        }
        let raw: String = self.chars[start..self.pos].iter().collect();
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(self.error(format!("expected a value, found {}", self.describe())));
        }
        Ok(parse_number(raw).unwrap_or_else(|| Value::String(raw.to_owned())))
    }

    fn quoted(&mut self, quote: char) -> Result<String, ParseError> {
        let (line, column) = (self.line, self.column);
        self.bump();
        let mut s = String::new();
        loop {
            match self.bump() {
                None => {
                    return Err(ParseError {
                        line,
                        column,
                        message: "unterminated string".into(),
                    });
                }
                Some(c) if c == quote => {
                    if self.peek() == Some(quote) {
                        self.bump();
                        s.push(quote);
                    } else {
                        return Ok(s);
                    }
                }
                Some(c) => s.push(c),
            }
        }
    }
}

/// Parses a number literal per the rules in the module docs; `None` if `raw` is not a number.
pub(crate) fn parse_number(raw: &str) -> Option<Value> {
    let (negative, digits) = match raw.as_bytes().first()? {
        b'-' => (true, &raw[1..]),
        b'+' => (false, &raw[1..]),
        _ => (false, raw),
    };
    if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        let v = i64::from_str_radix(hex, 16).ok()?;
        let v = if negative { -v } else { v };
        return Some(match i32::try_from(v) {
            Ok(i) => Value::Int(i),
            Err(_) if (0..=i64::from(u32::MAX)).contains(&v) => Value::Int(v as u32 as i32),
            Err(_) => Value::Int64(v),
        });
    }
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(v) = raw.parse::<i64>() {
            return Some(match i32::try_from(v) {
                Ok(i) => Value::Int(i),
                Err(_) => Value::Int64(v),
            });
        }
        return raw.parse::<f32>().ok().map(Value::Float);
    }
    is_float_literal(digits)
        .then(|| raw.parse::<f32>().ok().map(Value::Float))
        .flatten()
}

/// `digits* ['.' digits*] [('e'|'E') ['+'|'-'] digits+]` with at least one mantissa digit.
fn is_float_literal(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    let int_digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    i += int_digits;
    let mut frac_digits = 0;
    if b.get(i) == Some(&b'.') {
        i += 1;
        frac_digits = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        i += frac_digits;
    }
    if int_digits + frac_digits == 0 {
        return false;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exp_digits = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        if exp_digits == 0 {
            return false;
        }
        i += exp_digits;
    }
    i == b.len()
}

/// Pretty-prints a config as config.cpp text that [`parse_text`] reads back to the same
/// [`Config`] (except [`Value::Expression`], which prints as a plain string, and non-finite
/// floats, which have no literal syntax).
pub fn write_text(config: &Config) -> String {
    let mut out = String::new();
    if !config.enums.is_empty() {
        out.push_str("enum\n{\n");
        for (i, e) in config.enums.iter().enumerate() {
            let sep = if i + 1 < config.enums.len() { "," } else { "" };
            let _ = writeln!(out, "    {} = {}{sep}", e.name, e.value);
        }
        out.push_str("};\n");
    }
    write_entries(&mut out, &config.root.entries, 0);
    out
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("    ");
    }
}

fn write_entries(out: &mut String, entries: &[Entry], level: usize) {
    for entry in entries {
        indent(out, level);
        let name = &entry.name;
        match &entry.kind {
            EntryKind::Class(class) => {
                let _ = write!(out, "class {name}");
                if let Some(base) = &class.base {
                    let _ = write!(out, ": {base}");
                }
                if class.entries.is_empty() {
                    out.push_str(" {};\n");
                } else {
                    out.push('\n');
                    indent(out, level);
                    out.push_str("{\n");
                    write_entries(out, &class.entries, level + 1);
                    indent(out, level);
                    out.push_str("};\n");
                }
            }
            EntryKind::External => {
                let _ = writeln!(out, "class {name};");
            }
            EntryKind::Delete => {
                let _ = writeln!(out, "delete {name};");
            }
            EntryKind::Value(Value::Array(items)) => {
                let _ = write!(out, "{name}[] = ");
                write_array(out, items);
                out.push_str(";\n");
            }
            EntryKind::Value(value) => {
                let _ = write!(out, "{name} = ");
                write_scalar(out, value);
                out.push_str(";\n");
            }
            EntryKind::ArrayAppend(items) => {
                let _ = write!(out, "{name}[] += ");
                write_array(out, items);
                out.push_str(";\n");
            }
        }
    }
}

fn write_array(out: &mut String, items: &[Value]) {
    out.push('{');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write_scalar(out, item);
    }
    out.push('}');
}

fn write_scalar(out: &mut String, value: &Value) {
    match value {
        Value::String(s) | Value::Expression(s) => {
            out.push('"');
            out.push_str(&s.replace('"', "\"\""));
            out.push('"');
        }
        // Debug keeps a `.0` on integral floats so they read back as floats, and uses the
        // shortest digits that round-trip.
        Value::Float(f) => {
            let _ = write!(out, "{f:?}");
        }
        Value::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Value::Int64(i) => {
            let _ = write!(out, "{i}");
        }
        Value::Array(items) => write_array(out, items),
    }
}
