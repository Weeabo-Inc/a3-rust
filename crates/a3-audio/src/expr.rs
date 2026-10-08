//! "Simple expressions": the small arithmetic language of sound controllers in config
//! (`volume = "forest * (windy factor [0.1, 0.5])"`).
//!
//! An expression is parsed once into a tree and evaluated many times against a set of named
//! variables (`distance`, `speed`, `rain`, `forest`, ...). Syntax and precedence follow SQF,
//! which the engine's evaluator mirrors; see `docs/re/audio.md`.
//!
//! - Numbers, variables (case-insensitive; unknown names are 0), parentheses, `[a, b, ...]`
//!   argument lists.
//! - Unary: `-x`, `+x`, `abs`, `sqrt`, `sin`, `cos`, `exp`, `ln`/`log`, `floor`, `ceil`,
//!   `round`, `!`/`not`.
//! - Binary, from loosest to tightest: `||`/`or`; `&&`/`and`; comparisons `== != < > <= >=`
//!   (1 or 0); `factor [lo, hi]`, `interpolate [x0, x1, y0, y1]`, `envelope [a, b, c, d]`;
//!   `+ - max min`;
//!   `* / % mod`; `^`.

use std::fmt;

/// A parse error with the byte offset in the source text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("expression {text:?}: {message} at byte {at}")]
pub struct ExprError {
    /// The expression text.
    pub text: String,
    /// What is wrong.
    pub message: String,
    /// Byte offset of the problem.
    pub at: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unary {
    Neg,
    Abs,
    Sqrt,
    Sin,
    Cos,
    Exp,
    Ln,
    Floor,
    Ceil,
    Round,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Binary {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Add,
    Sub,
    Max,
    Min,
    Mul,
    Div,
    Mod,
    Pow,
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Const(f32),
    Var(usize),
    Unary(Unary, Box<Node>),
    Binary(Binary, Box<Node>, Box<Node>),
    /// `x factor [lo, hi]`.
    Factor(Box<Node>, Box<Node>, Box<Node>),
    /// `x interpolate [x0, x1, y0, y1]`.
    Interpolate(Box<Node>, [Box<Node>; 4]),
    /// `x envelope [a, b, c, d]`.
    Envelope(Box<Node>, [Box<Node>; 4]),
}

/// A parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    text: String,
    root: Node,
    /// Lower-case variable names referenced, indexed by `Node::Var`.
    vars: Vec<String>,
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl Expr {
    /// Parses `text`.
    pub fn parse(text: &str) -> Result<Self, ExprError> {
        let tokens = tokenize(text)?;
        let mut p = Parser {
            text,
            tokens,
            pos: 0,
            vars: Vec::new(),
        };
        let root = p.expr(0)?;
        // Shipped configs have stray closing parentheses at the end of a few expressions;
        // they are ignored _(engine behaviour: medium confidence)_.
        while p.peek() == Some(&Tok::Sym(")")) {
            p.pos += 1;
        }
        if let Some(tok) = p.tokens.get(p.pos) {
            return Err(p.error("unexpected input", tok.at));
        }
        Ok(Self {
            text: text.to_string(),
            root,
            vars: p.vars,
        })
    }

    /// A constant.
    pub fn constant(value: f32) -> Self {
        Self {
            text: value.to_string(),
            root: Node::Const(value),
            vars: Vec::new(),
        }
    }

    /// The source text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The variables the expression reads, in lower case.
    pub fn variables(&self) -> &[String] {
        &self.vars
    }

    /// The value if the expression reads no variables.
    pub fn as_constant(&self) -> Option<f32> {
        match self.root {
            Node::Const(v) => Some(v),
            _ => None,
        }
    }

    /// Evaluates with `lookup` giving each variable's value (`None` counts as 0). Names are
    /// passed in lower case.
    pub fn eval(&self, lookup: impl Fn(&str) -> Option<f32>) -> f32 {
        let values: Vec<f32> = self
            .vars
            .iter()
            .map(|name| lookup(name).unwrap_or(0.0))
            .collect();
        eval(&self.root, &values)
    }
}

fn boolean(b: bool) -> f32 {
    if b { 1.0 } else { 0.0 }
}

/// `x factor [lo, hi]`: 0 at `lo`, 1 at `hi`, linear between and clamped (also when `lo > hi`).
pub fn factor(x: f32, lo: f32, hi: f32) -> f32 {
    if hi == lo {
        return boolean(x >= hi);
    }
    ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
}

fn eval(node: &Node, vars: &[f32]) -> f32 {
    match node {
        Node::Const(v) => *v,
        Node::Var(i) => vars[*i],
        Node::Unary(op, a) => {
            let a = eval(a, vars);
            match op {
                Unary::Neg => -a,
                Unary::Abs => a.abs(),
                Unary::Sqrt => a.max(0.0).sqrt(),
                Unary::Sin => a.to_radians().sin(),
                Unary::Cos => a.to_radians().cos(),
                Unary::Exp => a.exp(),
                Unary::Ln => a.ln(),
                Unary::Floor => a.floor(),
                Unary::Ceil => a.ceil(),
                Unary::Round => a.round(),
                Unary::Not => boolean(a == 0.0),
            }
        }
        Node::Binary(op, a, b) => {
            let (a, b) = (eval(a, vars), eval(b, vars));
            match op {
                Binary::Or => boolean(a != 0.0 || b != 0.0),
                Binary::And => boolean(a != 0.0 && b != 0.0),
                Binary::Eq => boolean(a == b),
                Binary::Ne => boolean(a != b),
                Binary::Lt => boolean(a < b),
                Binary::Gt => boolean(a > b),
                Binary::Le => boolean(a <= b),
                Binary::Ge => boolean(a >= b),
                Binary::Add => a + b,
                Binary::Sub => a - b,
                Binary::Max => a.max(b),
                Binary::Min => a.min(b),
                Binary::Mul => a * b,
                Binary::Div => {
                    if b == 0.0 {
                        0.0
                    } else {
                        a / b
                    }
                }
                Binary::Mod => {
                    if b == 0.0 {
                        0.0
                    } else {
                        a % b
                    }
                }
                Binary::Pow => a.powf(b),
            }
        }
        Node::Factor(x, lo, hi) => factor(eval(x, vars), eval(lo, vars), eval(hi, vars)),
        Node::Envelope(x, [a, b, c, d]) => {
            let x = eval(x, vars);
            let (a, b, c, d) = (eval(a, vars), eval(b, vars), eval(c, vars), eval(d, vars));
            factor(x, a, b).min(factor(x, d, c))
        }
        Node::Interpolate(x, [x0, x1, y0, y1]) => {
            let t = factor(eval(x, vars), eval(x0, vars), eval(x1, vars));
            let (y0, y1) = (eval(y0, vars), eval(y1, vars));
            y0 + (y1 - y0) * t
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f32),
    Word(String),
    Sym(&'static str),
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    at: usize,
}

const SYMBOLS: [&str; 18] = [
    "==", "!=", "<=", ">=", "&&", "||", "<", ">", "+", "-", "*", "/", "%", "^", "(", ")", "[", "]",
];

fn tokenize(text: &str) -> Result<Vec<Token>, ExprError> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            // Exponent: 1e-5, 2.5E3.
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                let mut j = i + 1;
                if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
                    j += 1;
                }
                if j < bytes.len() && bytes[j].is_ascii_digit() {
                    i = j;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let value: f32 = text[start..i].parse().map_err(|_| ExprError {
                text: text.to_string(),
                message: "bad number".into(),
                at: start,
            })?;
            out.push(Token {
                tok: Tok::Num(value),
                at: start,
            });
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Word(text[start..i].to_ascii_lowercase()),
                at: start,
            });
            continue;
        }
        if c == b',' {
            out.push(Token {
                tok: Tok::Sym(","),
                at: i,
            });
            i += 1;
            continue;
        }
        if c == b'!' && bytes.get(i + 1) != Some(&b'=') {
            out.push(Token {
                tok: Tok::Sym("!"),
                at: i,
            });
            i += 1;
            continue;
        }
        let Some(sym) = SYMBOLS.iter().find(|s| text[i..].starts_with(**s)) else {
            return Err(ExprError {
                text: text.to_string(),
                message: format!("unexpected character {:?}", c as char),
                at: i,
            });
        };
        out.push(Token {
            tok: Tok::Sym(sym),
            at: i,
        });
        i += sym.len();
    }
    Ok(out)
}

struct Parser<'a> {
    text: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    vars: Vec<String>,
}

/// Binding power of a binary operator token (higher binds tighter), as in SQF.
fn binary_op(tok: &Tok) -> Option<(u8, Option<Binary>)> {
    let op = match tok {
        Tok::Sym(s) => match *s {
            "||" => (1, Binary::Or),
            "&&" => (2, Binary::And),
            "==" => (3, Binary::Eq),
            "!=" => (3, Binary::Ne),
            "<" => (3, Binary::Lt),
            ">" => (3, Binary::Gt),
            "<=" => (3, Binary::Le),
            ">=" => (3, Binary::Ge),
            "+" => (5, Binary::Add),
            "-" => (5, Binary::Sub),
            "*" => (6, Binary::Mul),
            "/" => (6, Binary::Div),
            "%" => (6, Binary::Mod),
            "^" => (7, Binary::Pow),
            _ => return None,
        },
        Tok::Word(w) => match w.as_str() {
            "or" => (1, Binary::Or),
            "and" => (2, Binary::And),
            "max" => (5, Binary::Max),
            "min" => (5, Binary::Min),
            "mod" => (6, Binary::Mod),
            // Argument-list operators, handled by the caller.
            "factor" | "interpolate" | "envelope" => return Some((4, None)),
            _ => return None,
        },
        Tok::Num(_) => return None,
    };
    Some((op.0, Some(op.1)))
}

fn unary_op(word: &str) -> Option<Unary> {
    Some(match word {
        "abs" => Unary::Abs,
        "sqrt" => Unary::Sqrt,
        "sin" => Unary::Sin,
        "cos" => Unary::Cos,
        "exp" => Unary::Exp,
        "ln" | "log" => Unary::Ln,
        "floor" => Unary::Floor,
        "ceil" => Unary::Ceil,
        "round" => Unary::Round,
        "not" => Unary::Not,
        _ => return None,
    })
}

impl Parser<'_> {
    fn error(&self, message: &str, at: usize) -> ExprError {
        ExprError {
            text: self.text.to_string(),
            message: message.to_string(),
            at,
        }
    }

    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos).map(|t| &t.tok)
    }

    fn at(&self) -> usize {
        self.tokens.get(self.pos).map_or(self.text.len(), |t| t.at)
    }

    fn expect(&mut self, sym: &str) -> Result<(), ExprError> {
        match self.peek() {
            Some(Tok::Sym(s)) if *s == sym => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(self.error(&format!("expected {sym:?}"), self.at())),
        }
    }

    /// Precedence climbing over binary operators binding tighter than `min`.
    fn expr(&mut self, min: u8) -> Result<Node, ExprError> {
        let mut left = self.unary()?;
        while let Some(tok) = self.peek().cloned() {
            let Some((power, op)) = binary_op(&tok) else {
                break;
            };
            if power <= min {
                break;
            }
            self.pos += 1;
            left = match op {
                Some(op) => {
                    // Every operator is left-associative, as in SQF.
                    let right = self.expr(power)?;
                    Node::Binary(op, Box::new(left), Box::new(right))
                }
                None => {
                    let name = match tok {
                        Tok::Word(w) => w,
                        _ => unreachable!("argument operators are words"),
                    };
                    let at = self.at();
                    let mut args = self.list()?;
                    match (name.as_str(), args.len()) {
                        ("factor", 2) => {
                            let hi = args.pop().expect("2");
                            let lo = args.pop().expect("2");
                            Node::Factor(Box::new(left), Box::new(lo), Box::new(hi))
                        }
                        ("interpolate" | "envelope", 4) => {
                            let mut it = args.into_iter().map(Box::new);
                            let four = [(); 4].map(|_| it.next().expect("4"));
                            if name == "envelope" {
                                Node::Envelope(Box::new(left), four)
                            } else {
                                Node::Interpolate(Box::new(left), four)
                            }
                        }
                        (name, n) => {
                            return Err(
                                self.error(&format!("{name} takes a list, got {n} items"), at)
                            );
                        }
                    }
                }
            };
        }
        Ok(left)
    }

    /// `[a, b, ...]`.
    fn list(&mut self) -> Result<Vec<Node>, ExprError> {
        self.expect("[")?;
        let mut items = Vec::new();
        if self.peek() != Some(&Tok::Sym("]")) {
            loop {
                items.push(self.expr(0)?);
                if self.peek() == Some(&Tok::Sym(",")) {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }
        self.expect("]")?;
        Ok(items)
    }

    fn unary(&mut self) -> Result<Node, ExprError> {
        let at = self.at();
        let Some(tok) = self.peek().cloned() else {
            return Err(self.error("expression ends early", at));
        };
        self.pos += 1;
        Ok(match tok {
            Tok::Num(v) => Node::Const(v),
            Tok::Sym("(") => {
                let inner = self.expr(0)?;
                self.expect(")")?;
                inner
            }
            Tok::Sym("-") => fold_unary(Unary::Neg, self.unary()?),
            Tok::Sym("+") => self.unary()?,
            Tok::Sym("!") => fold_unary(Unary::Not, self.unary()?),
            Tok::Word(w) => match unary_op(&w) {
                Some(op) => fold_unary(op, self.unary()?),
                None => {
                    if binary_op(&Tok::Word(w.clone())).is_some() {
                        return Err(self.error(&format!("operator {w:?} needs a left operand"), at));
                    }
                    let index = match self.vars.iter().position(|v| *v == w) {
                        Some(i) => i,
                        None => {
                            self.vars.push(w);
                            self.vars.len() - 1
                        }
                    };
                    Node::Var(index)
                }
            },
            _ => return Err(self.error("expected a value", at)),
        })
    }
}

fn fold_unary(op: Unary, node: Node) -> Node {
    match node {
        Node::Const(v) => Node::Const(eval(&Node::Unary(op, Box::new(Node::Const(v))), &[])),
        node => Node::Unary(op, Box::new(node)),
    }
}
