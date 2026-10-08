//! `__EVAL` / `__EXEC` support.
//!
//! In the engine these two are handled by the config parser, which runs their contents as SQF in
//! `parsingNamespace`. The SQF VM does not exist yet, so the preprocessor talks to an
//! [`Evaluator`]; [`SimpleEvaluator`] covers what real configs use (arithmetic, string
//! concatenation, variables assigned by `__EXEC`). The SQF VM will implement [`Evaluator`] later.

use std::collections::HashMap;

/// The result of `__EVAL`. The engine only produces numbers and strings; any other SQF type
/// (Boolean included) is converted to its string form by the evaluator.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalValue {
    /// A number.
    Number(f64),
    /// A string.
    String(String),
}

/// Runs the SQF in `__EXEC(...)` and `__EVAL(...)`.
pub trait Evaluator {
    /// Runs the statements of an `__EXEC`. Variables it assigns stay visible to later calls.
    fn exec(&mut self, code: &str) -> Result<(), String>;
    /// Evaluates the expression of an `__EVAL`.
    fn eval(&mut self, expression: &str) -> Result<EvalValue, String>;
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Number(f64),
    String(String),
    Bool(bool),
}

impl Value {
    fn type_name(&self) -> &'static str {
        match self {
            Value::Number(_) => "Number",
            Value::String(_) => "String",
            Value::Bool(_) => "Boolean",
        }
    }

    fn number(&self) -> Result<f64, String> {
        match self {
            Value::Number(n) => Ok(*n),
            other => Err(format!("expected Number, got {}", other.type_name())),
        }
    }

    fn bool(&self) -> Result<bool, String> {
        match self {
            Value::Bool(b) => Ok(*b),
            other => Err(format!("expected Boolean, got {}", other.type_name())),
        }
    }

    fn to_sqf_string(&self) -> String {
        match self {
            Value::Number(n) => format_number(*n),
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
        }
    }
}

/// Formats a number the way it appears in preprocessed text: integers without a fraction, other
/// values in the shortest form that round-trips through a 32-bit float (configs store `float`).
pub(crate) fn format_number(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", n as f32)
    }
}

/// A small SQF subset evaluator for `__EVAL` / `__EXEC`.
///
/// Supports numbers (decimal, `0x`/`$` hex, exponent), strings (`"..."` / `'...'` with doubled
/// quote escapes), `true`/`false`, `pi`, variables (global and `_local`, case-insensitive, shared
/// like `parsingNamespace`), assignments (`a = expr`, optional `private`), `;`-separated
/// statements, parentheses, unary `+ - ! not abs ceil floor round sqrt str`, and the binary
/// operators `^`, `* / % mod`, `+ - min max`, `== != < > <= >=`, `&& and`, `|| or` with SQF
/// precedence (left-to-right within a level). Anything else is an error.
#[derive(Debug, Clone, Default)]
pub struct SimpleEvaluator {
    variables: HashMap<String, Value>,
}

impl SimpleEvaluator {
    /// An evaluator with no variables.
    pub fn new() -> Self {
        Self::default()
    }

    /// The value of a variable set by `__EXEC`, as `parsingNamespace getVariable` would see it.
    pub fn variable(&self, name: &str) -> Option<EvalValue> {
        self.variables
            .get(&name.to_ascii_lowercase())
            .map(|v| match v {
                Value::Number(n) => EvalValue::Number(*n),
                Value::String(s) => EvalValue::String(s.clone()),
                Value::Bool(b) => EvalValue::String(b.to_string()),
            })
    }

    fn run(&mut self, code: &str) -> Result<Option<Value>, String> {
        let tokens = tokenize(code)?;
        let mut parser = Parser {
            tokens: &tokens,
            pos: 0,
            vars: &mut self.variables,
        };
        let mut last = None;
        loop {
            while parser.eat(&Token::Semicolon) {}
            if parser.at_end() {
                break;
            }
            last = Some(parser.statement()?);
            if !parser.at_end() && !parser.eat(&Token::Semicolon) {
                return Err(format!("unexpected `{}`", parser.peek_text()));
            }
        }
        Ok(last)
    }
}

impl Evaluator for SimpleEvaluator {
    fn exec(&mut self, code: &str) -> Result<(), String> {
        self.run(code).map(|_| ())
    }

    fn eval(&mut self, expression: &str) -> Result<EvalValue, String> {
        match self.run(expression)? {
            Some(Value::Number(n)) => Ok(EvalValue::Number(n)),
            Some(other) => Ok(EvalValue::String(other.to_sqf_string())),
            None => Err("empty expression".to_owned()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    String(String),
    Word(String),
    Op(&'static str),
    LParen,
    RParen,
    Semicolon,
}

const OPERATORS: [&str; 15] = [
    "==", "!=", "<=", ">=", "&&", "||", "<", ">", "+", "-", "*", "/", "%", "^", "!",
];

fn tokenize(code: &str) -> Result<Vec<Token>, String> {
    let bytes = code.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            b')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            b';' => {
                tokens.push(Token::Semicolon);
                i += 1;
            }
            b'=' if bytes.get(i + 1) != Some(&b'=') => {
                tokens.push(Token::Op("="));
                i += 1;
            }
            b'"' | b'\'' => {
                let mut s = String::new();
                i += 1;
                let start = i;
                let mut seg = start;
                loop {
                    match bytes.get(i) {
                        None => return Err("unterminated string".to_owned()),
                        Some(&q) if q == c => {
                            s.push_str(&code[seg..i]);
                            if bytes.get(i + 1) == Some(&c) {
                                s.push(c as char);
                                i += 2;
                                seg = i;
                            } else {
                                i += 1;
                                break;
                            }
                        }
                        Some(_) => i += 1,
                    }
                }
                tokens.push(Token::String(s));
            }
            b'$' => {
                let start = i + 1;
                i = start;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let n = u64::from_str_radix(&code[start..i], 16)
                    .map_err(|_| "invalid hex number".to_owned())?;
                tokens.push(Token::Number(n as f64));
            }
            b'0' if matches!(bytes.get(i + 1), Some(b'x' | b'X')) => {
                let start = i + 2;
                i = start;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let n = u64::from_str_radix(&code[start..i], 16)
                    .map_err(|_| "invalid hex number".to_owned())?;
                tokens.push(Token::Number(n as f64));
            }
            b'0'..=b'9' | b'.' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
                    let mut j = i + 1;
                    if j < bytes.len() && matches!(bytes[j], b'+' | b'-') {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j].is_ascii_digit() {
                        i = j;
                        while i < bytes.len() && bytes[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let n = code[start..i]
                    .parse::<f64>()
                    .map_err(|_| format!("invalid number `{}`", &code[start..i]))?;
                tokens.push(Token::Number(n));
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                tokens.push(Token::Word(code[start..i].to_ascii_lowercase()));
            }
            _ => {
                let op = OPERATORS
                    .iter()
                    .find(|op| code[i..].starts_with(**op))
                    .ok_or_else(|| {
                        format!(
                            "unexpected character `{}`",
                            code[i..].chars().next().unwrap_or(' ')
                        )
                    })?;
                tokens.push(Token::Op(op));
                i += op.len();
            }
        }
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    vars: &'a mut HashMap<String, Value>,
}

/// Binary operator precedence levels, loosest first (SQF order).
const LEVELS: [&[&str]; 6] = [
    &["||", "or"],
    &["&&", "and"],
    &["==", "!=", "<", ">", "<=", ">="],
    &["+", "-", "min", "max"],
    &["*", "/", "%", "mod"],
    &["^"],
];

impl Parser<'_> {
    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_text(&self) -> String {
        match self.peek() {
            Some(Token::Number(n)) => format_number(*n),
            Some(Token::String(s)) => format!("\"{s}\""),
            Some(Token::Word(w)) => w.clone(),
            Some(Token::Op(o)) => (*o).to_owned(),
            Some(Token::LParen) => "(".to_owned(),
            Some(Token::RParen) => ")".to_owned(),
            Some(Token::Semicolon) => ";".to_owned(),
            None => "end of input".to_owned(),
        }
    }

    fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn statement(&mut self) -> Result<Value, String> {
        if self.peek() == Some(&Token::Word("private".to_owned())) {
            self.pos += 1;
        }
        if let (Some(Token::Word(name)), Some(Token::Op("="))) =
            (self.tokens.get(self.pos), self.tokens.get(self.pos + 1))
        {
            let name = name.clone();
            self.pos += 2;
            let value = self.binary(0)?;
            self.vars.insert(name, value.clone());
            return Ok(value);
        }
        self.binary(0)
    }

    fn binary_op_at(&self, level: usize) -> Option<&'static str> {
        let text = match self.peek()? {
            Token::Op(o) => *o,
            Token::Word(w) => LEVELS[level].iter().copied().find(|op| op == w)?,
            _ => return None,
        };
        LEVELS[level].iter().copied().find(|op| *op == text)
    }

    fn binary(&mut self, level: usize) -> Result<Value, String> {
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(op) = self.binary_op_at(level) {
            self.pos += 1;
            let right = self.binary(level + 1)?;
            left = apply_binary(op, left, right)?;
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Value, String> {
        let token = self.peek().cloned().ok_or("unexpected end of input")?;
        self.pos += 1;
        match token {
            Token::Op("-") => Ok(Value::Number(-self.unary()?.number()?)),
            Token::Op("+") => Ok(Value::Number(self.unary()?.number()?)),
            Token::Op("!") => Ok(Value::Bool(!self.unary()?.bool()?)),
            Token::Number(n) => Ok(Value::Number(n)),
            Token::String(s) => Ok(Value::String(s)),
            Token::LParen => {
                let value = self.binary(0)?;
                if !self.eat(&Token::RParen) {
                    return Err("missing `)`".to_owned());
                }
                Ok(value)
            }
            Token::Word(word) => {
                match word.as_str() {
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    "pi" => Ok(Value::Number(std::f64::consts::PI)),
                    "not" => Ok(Value::Bool(!self.unary()?.bool()?)),
                    "str" => {
                        let v = self.unary()?;
                        Ok(Value::String(match v {
                            Value::String(s) => format!("\"{}\"", s.replace('"', "\"\"")),
                            other => other.to_sqf_string(),
                        }))
                    }
                    "abs" | "ceil" | "floor" | "round" | "sqrt" => {
                        let n = self.unary()?.number()?;
                        Ok(Value::Number(match word.as_str() {
                            "abs" => n.abs(),
                            "ceil" => n.ceil(),
                            "floor" => n.floor(),
                            "round" => n.round(),
                            _ => n.sqrt(),
                        }))
                    }
                    name => self.vars.get(name).cloned().ok_or_else(|| {
                        format!("unsupported command or undefined variable `{name}`")
                    }),
                }
            }
            other => Err(format!("unexpected `{other:?}`")),
        }
    }
}

fn apply_binary(op: &str, left: Value, right: Value) -> Result<Value, String> {
    use Value::{Bool, Number, String as Str};
    Ok(match (op, left, right) {
        ("+", Number(a), Number(b)) => Number(a + b),
        ("+", Str(a), Str(b)) => Str(a + &b),
        ("-", Number(a), Number(b)) => Number(a - b),
        ("*", Number(a), Number(b)) => Number(a * b),
        ("/", Number(a), Number(b)) => Number(a / b),
        ("%" | "mod", Number(a), Number(b)) => Number(a % b),
        ("^", Number(a), Number(b)) => Number(a.powf(b)),
        ("min", Number(a), Number(b)) => Number(a.min(b)),
        ("max", Number(a), Number(b)) => Number(a.max(b)),
        ("==", a, b) => Bool(match (&a, &b) {
            (Str(x), Str(y)) => x.eq_ignore_ascii_case(y),
            _ => a == b,
        }),
        ("!=", a, b) => Bool(match (&a, &b) {
            (Str(x), Str(y)) => !x.eq_ignore_ascii_case(y),
            _ => a != b,
        }),
        ("<", Number(a), Number(b)) => Bool(a < b),
        (">", Number(a), Number(b)) => Bool(a > b),
        ("<=", Number(a), Number(b)) => Bool(a <= b),
        (">=", Number(a), Number(b)) => Bool(a >= b),
        ("&&" | "and", Bool(a), Bool(b)) => Bool(a && b),
        ("||" | "or", Bool(a), Bool(b)) => Bool(a || b),
        (op, a, b) => {
            return Err(format!(
                "operator `{op}` does not accept {} and {}",
                a.type_name(),
                b.type_name()
            ));
        }
    })
}
