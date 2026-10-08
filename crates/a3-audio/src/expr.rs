//! "Simple expressions": the small arithmetic language of sound controllers in config
//! (`volume = "forest * (windy factor [0.1, 0.5])"`).
//!
//! The engine compiles these with its SQF parser into a float program (`docs/re/audio.md`
//! section 1), so syntax and priorities are SQF's. An expression is parsed once and evaluated
//! many times against named variables (`distance`, `speed`, `rain`, `forest`, ...).
//!
//! Operators that work on variables, from loosest to tightest:
//! comparisons `< > <= >=` (1 or 0); `factor [a, b]`, `interpolate [a, b, c, d]`,
//! `envelope [a, b, c, d]` (constant arguments); `+ - min max`; `* /`; `pow`; and the unary
//! `-x`, `+x`, `abs`, `sqr`, `sqrt`, `randomGen`.
//!
//! Other SQF operators (`^`, `%`, `mod`, `==`, `!=`, `&&`, `||`, `sin`, `cos`, `exp`, `ln`,
//! `floor`, `ceil`, `round`, ...) only work on constants: the engine folds them at compile time
//! and fails the whole expression when one of their operands depends on a variable. So do we.
//! An identifier that is not one of the context's variables also fails the expression
//! ([`Expr::check`]).

use std::cell::Cell;
use std::cmp::Ordering::{Greater, Less};
use std::fmt;

/// A parse or compile error with the byte offset in the source text.
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
    Sqr,
    Sqrt,
    RandomGen,
    // Constant only:
    Sin,
    Cos,
    Exp,
    Ln,
    Floor,
    Ceil,
    Round,
    Not,
}

impl Unary {
    fn works_on_variables(self) -> bool {
        matches!(
            self,
            Self::Neg | Self::Abs | Self::Sqr | Self::Sqrt | Self::RandomGen
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Binary {
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
    Pow,
    // Constant only:
    Or,
    And,
    Eq,
    Ne,
    Mod,
    Caret,
}

impl Binary {
    fn works_on_variables(self) -> bool {
        !matches!(
            self,
            Self::Or | Self::And | Self::Eq | Self::Ne | Self::Mod | Self::Caret
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Const(f32),
    Var(usize),
    Unary(Unary, Box<Node>),
    Binary(Binary, Box<Node>, Box<Node>),
    /// `x factor [a, b]`.
    Factor(Box<Node>, f32, f32),
    /// `x interpolate [a, b, c, d]`.
    Interpolate(Box<Node>, [f32; 4]),
    /// `x envelope [a, b, c, d]`.
    Envelope(Box<Node>, [f32; 4]),
}

/// A parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    text: String,
    root: Node,
    /// Lower-case variable names referenced, indexed by `Node::Var`, with the byte offset of
    /// their first use.
    vars: Vec<(String, usize)>,
    names: Vec<String>,
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl Expr {
    /// Parses `text`. Any identifier is accepted as a variable; see [`Expr::check`].
    pub fn parse(text: &str) -> Result<Self, ExprError> {
        let tokens = tokenize(text)?;
        let mut p = Parser {
            text,
            tokens,
            pos: 0,
            vars: Vec::new(),
        };
        let root = p.expr(0)?;
        // Shipped configs have stray closing parentheses at the end of a few expressions; the
        // engine accepts them _(medium)_.
        while p.peek() == Some(&Tok::Sym(")")) {
            p.pos += 1;
        }
        if let Some(tok) = p.tokens.get(p.pos) {
            return Err(p.error("unexpected input", tok.at));
        }
        let names = p.vars.iter().map(|(n, _)| n.clone()).collect();
        Ok(Self {
            text: text.to_string(),
            root,
            vars: p.vars,
            names,
        })
    }

    /// Parses `text` and checks it against the variables of a context ([`Expr::check`]).
    pub fn compile(text: &str, variables: &[&str]) -> Result<Self, ExprError> {
        let expr = Self::parse(text)?;
        expr.check(variables)?;
        Ok(expr)
    }

    /// A constant.
    pub fn constant(value: f32) -> Self {
        Self {
            text: value.to_string(),
            root: Node::Const(value),
            vars: Vec::new(),
            names: Vec::new(),
        }
    }

    /// Fails, as the engine's compiler does, when the expression names a variable that is not
    /// in `variables` (compared ignoring ASCII case).
    pub fn check(&self, variables: &[&str]) -> Result<(), ExprError> {
        for (name, at) in &self.vars {
            if !variables.iter().any(|v| v.eq_ignore_ascii_case(name)) {
                return Err(ExprError {
                    text: self.text.clone(),
                    message: format!("unknown variable {name:?}"),
                    at: *at,
                });
            }
        }
        Ok(())
    }

    /// The source text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The variables the expression reads, in lower case.
    pub fn variables(&self) -> &[String] {
        &self.names
    }

    /// The value if the expression reads no variables.
    pub fn as_constant(&self) -> Option<f32> {
        match self.root {
            Node::Const(v) => Some(v),
            _ => None,
        }
    }

    /// Evaluates with `lookup` giving each variable's value (`None` counts as 0). Names are
    /// passed in lower case. Like the engine, division by zero gives an infinite or NaN result.
    pub fn eval(&self, lookup: impl Fn(&str) -> Option<f32>) -> f32 {
        let values: Vec<f32> = self
            .names
            .iter()
            .map(|name| lookup(name).unwrap_or(0.0))
            .collect();
        eval(&self.root, &values)
    }
}

fn boolean(b: bool) -> f32 {
    if b { 1.0 } else { 0.0 }
}

/// `x factor [a, b]`: maps `a` to 0 and `b` to 1, linear and clamped; also for `a > b`.
pub fn factor(x: f32, a: f32, b: f32) -> f32 {
    if b > a {
        if x < a {
            0.0
        } else if x > b {
            1.0
        } else {
            (x - a) / (b - a)
        }
    } else if x < b {
        1.0
    } else if x > a {
        0.0
    } else {
        1.0 - (x - b) / (a - b) // NaN for x == a == b, as in the engine
    }
}

/// `x interpolate [a, b, c, d]`: maps `a` to `c` and `b` to `d`, linear and clamped.
pub fn interpolate(x: f32, [a, b, c, d]: [f32; 4]) -> f32 {
    if b > a {
        if x < a {
            c
        } else if x > b {
            d
        } else {
            c + (x - a) / (b - a) * (d - c)
        }
    } else if x < b {
        d
    } else if x > a {
        c
    } else {
        d + (x - b) / (a - b) * (c - d)
    }
}

/// `x envelope [a, b, c, d]`: a trapezoid rising over `a..b`, 1 over `b..c`, falling over
/// `c..d`, 0 outside the open interval `(a, d)`.
pub fn envelope(x: f32, [a, b, c, d]: [f32; 4]) -> f32 {
    if !(a < x && x < d) {
        return 0.0;
    }
    let rise = if x <= b { (x - a) / (b - a) } else { 1.0 };
    if rise <= 0.0 {
        return 0.0;
    }
    let fall = if x >= c { 1.0 - (x - c) / (d - c) } else { 1.0 };
    rise * fall
}

thread_local! {
    static RANDOM_STATE: Cell<u32> = const { Cell::new(0x1234_5678) };
}

/// The engine's random generator (an LCG): a value in `0..1`.
pub fn rand01() -> f32 {
    RANDOM_STATE.with(|s| {
        let next = s.get().wrapping_mul(1_103_515_245).wrapping_add(12_345) & 0x7fff_ffff;
        s.set(next);
        next as f32 * 4.656_613e-10
    })
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
                Unary::Sqr => a * a,
                Unary::Sqrt => a.sqrt(),
                Unary::RandomGen => a * rand01(),
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
                // NaN compares true, as in the engine (each test is a negated opposite).
                Binary::Lt => boolean(matches!(a.partial_cmp(&b), Some(Less) | None)),
                Binary::Gt => boolean(matches!(a.partial_cmp(&b), Some(Greater) | None)),
                Binary::Le => boolean(!matches!(a.partial_cmp(&b), Some(Greater))),
                Binary::Ge => boolean(!matches!(a.partial_cmp(&b), Some(Less))),
                Binary::Add => a + b,
                Binary::Sub => a - b,
                Binary::Max => {
                    if a <= b {
                        b
                    } else {
                        a
                    }
                }
                Binary::Min => {
                    if b <= a {
                        b
                    } else {
                        a
                    }
                }
                Binary::Mul => a * b,
                Binary::Div => a / b,
                Binary::Pow | Binary::Caret => a.powf(b),
                Binary::Or => boolean(a != 0.0 || b != 0.0),
                Binary::And => boolean(a != 0.0 && b != 0.0),
                Binary::Eq => boolean(a == b),
                Binary::Ne => boolean(a != b),
                Binary::Mod => a % b,
            }
        }
        Node::Factor(x, a, b) => factor(eval(x, vars), *a, *b),
        Node::Interpolate(x, args) => interpolate(eval(x, vars), *args),
        Node::Envelope(x, args) => envelope(eval(x, vars), *args),
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

const SYMBOLS: [&str; 19] = [
    "==", "!=", "<=", ">=", "&&", "||", "<", ">", "+", "-", "*", "/", "%", "^", "(", ")", "[", "]",
    ",",
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
    vars: Vec<(String, usize)>,
}

enum Op {
    Binary(Binary),
    /// `factor`, `interpolate`, `envelope`: an argument list follows.
    List(&'static str),
}

/// SQF priority of a binary operator token (higher binds tighter).
fn binary_op(tok: &Tok) -> Option<(u8, Op)> {
    let (power, op) = match tok {
        Tok::Sym(s) => match *s {
            "||" => (1, Binary::Or),
            "&&" => (2, Binary::And),
            "==" => (3, Binary::Eq),
            "!=" => (3, Binary::Ne),
            "<" => (3, Binary::Lt),
            ">" => (3, Binary::Gt),
            "<=" => (3, Binary::Le),
            ">=" => (3, Binary::Ge),
            "+" => (6, Binary::Add),
            "-" => (6, Binary::Sub),
            "*" => (7, Binary::Mul),
            "/" => (7, Binary::Div),
            "%" => (7, Binary::Mod),
            "^" => (8, Binary::Caret),
            _ => return None,
        },
        Tok::Word(w) => match w.as_str() {
            "or" => (1, Binary::Or),
            "and" => (2, Binary::And),
            "max" => (6, Binary::Max),
            "min" => (6, Binary::Min),
            "mod" => (7, Binary::Mod),
            "pow" => (9, Binary::Pow),
            "factor" => return Some((4, Op::List("factor"))),
            "interpolate" => return Some((4, Op::List("interpolate"))),
            "envelope" => return Some((4, Op::List("envelope"))),
            _ => return None,
        },
        Tok::Num(_) => return None,
    };
    Some((power, Op::Binary(op)))
}

fn unary_op(word: &str) -> Option<Unary> {
    Some(match word {
        "abs" => Unary::Abs,
        "sqr" => Unary::Sqr,
        "sqrt" => Unary::Sqrt,
        "randomgen" => Unary::RandomGen,
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

    /// Precedence climbing over operators binding tighter than `min`; all left-associative.
    fn expr(&mut self, min: u8) -> Result<Node, ExprError> {
        let mut left = self.unary()?;
        while let Some(tok) = self.peek().cloned() {
            let Some((power, op)) = binary_op(&tok) else {
                break;
            };
            if power <= min {
                break;
            }
            let op_at = self.at();
            self.pos += 1;
            left = match op {
                Op::Binary(op) => {
                    let right = self.expr(power)?;
                    fold_binary(op, left, right).map_err(|()| self.variable_error(op_at))?
                }
                Op::List(name) => {
                    let at = self.at();
                    let args = self.constant_list()?;
                    match (name, args.as_slice()) {
                        ("factor", &[a, b]) => fold(Node::Factor(Box::new(left), a, b)),
                        ("interpolate", &[a, b, c, d]) => {
                            fold(Node::Interpolate(Box::new(left), [a, b, c, d]))
                        }
                        ("envelope", &[a, b, c, d]) => {
                            fold(Node::Envelope(Box::new(left), [a, b, c, d]))
                        }
                        (name, args) => {
                            return Err(self.error(
                                &format!(
                                    "{name} takes a list of {} numbers, got {}",
                                    if name == "factor" { 2 } else { 4 },
                                    args.len()
                                ),
                                at,
                            ));
                        }
                    }
                }
            };
        }
        Ok(left)
    }

    fn variable_error(&self, at: usize) -> ExprError {
        self.error("this operator only works on constants", at)
    }

    /// `[a, b, ...]` of constant expressions.
    fn constant_list(&mut self) -> Result<Vec<f32>, ExprError> {
        self.expect("[")?;
        let mut items = Vec::new();
        if self.peek() != Some(&Tok::Sym("]")) {
            loop {
                let at = self.at();
                match self.expr(0)? {
                    Node::Const(v) => items.push(v),
                    _ => return Err(self.error("list items must be constants", at)),
                }
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
        let (op, operand) = match tok {
            Tok::Num(v) => return Ok(Node::Const(v)),
            Tok::Sym("(") => {
                let inner = self.expr(0)?;
                self.expect(")")?;
                return Ok(inner);
            }
            Tok::Sym("-") => (Unary::Neg, self.unary()?),
            Tok::Sym("+") => return self.unary(),
            Tok::Sym("!") => (Unary::Not, self.unary()?),
            Tok::Word(w) => match unary_op(&w) {
                Some(op) => (op, self.unary()?),
                None => {
                    if binary_op(&Tok::Word(w.clone())).is_some() {
                        return Err(self.error(&format!("operator {w:?} needs a left operand"), at));
                    }
                    if w == "pi" {
                        return Ok(Node::Const(std::f32::consts::PI));
                    }
                    let index = match self.vars.iter().position(|(v, _)| *v == w) {
                        Some(i) => i,
                        None => {
                            self.vars.push((w, at));
                            self.vars.len() - 1
                        }
                    };
                    return Ok(Node::Var(index));
                }
            },
            _ => return Err(self.error("expected a value", at)),
        };
        if !op.works_on_variables() && !matches!(operand, Node::Const(_)) {
            return Err(self.variable_error(at));
        }
        Ok(fold(Node::Unary(op, Box::new(operand))))
    }
}

/// Folds a node whose operands are all constants (except `randomGen`, which draws anew at every
/// evaluation).
fn fold(node: Node) -> Node {
    let constant = match &node {
        Node::Unary(Unary::RandomGen, _) => false,
        Node::Unary(_, a) => matches!(**a, Node::Const(_)),
        Node::Binary(_, a, b) => matches!(**a, Node::Const(_)) && matches!(**b, Node::Const(_)),
        Node::Factor(x, ..) | Node::Interpolate(x, _) | Node::Envelope(x, _) => {
            matches!(**x, Node::Const(_))
        }
        Node::Const(_) | Node::Var(_) => false,
    };
    if constant {
        Node::Const(eval(&node, &[]))
    } else {
        node
    }
}

fn fold_binary(op: Binary, a: Node, b: Node) -> Result<Node, ()> {
    let constants = matches!(a, Node::Const(_)) && matches!(b, Node::Const(_));
    if !op.works_on_variables() && !constants {
        return Err(());
    }
    Ok(fold(Node::Binary(op, Box::new(a), Box::new(b))))
}
