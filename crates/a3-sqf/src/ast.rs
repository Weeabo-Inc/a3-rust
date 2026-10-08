//! SQF syntax tree and its pretty-printer.

use std::fmt::Write as _;
use std::rc::Rc;

use crate::source::Span;
use crate::symbol::Sym;
use crate::table::{CommandId, CommandTable, binary_precedence};

/// A sequence of statements: a whole script or the inside of `{ ... }`.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeBlock {
    pub statements: Vec<Statement>,
    /// The text of the block, without the braces.
    pub span: Span,
}

/// One statement.
#[derive(Clone, Debug, PartialEq)]
pub enum Statement {
    /// An expression whose value becomes the block's value if it is last.
    Expr(Expr),
    /// `name = value` or `private _name = value`.
    Assign {
        private: bool,
        name: Sym,
        /// The name as written.
        name_text: Rc<str>,
        value: Expr,
        span: Span,
    },
}

/// An expression node.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Number(f32),
    String(Rc<str>),
    Array(Vec<Expr>),
    Code(Rc<CodeBlock>),
    /// A local (`_x`) or global variable read.
    Variable {
        name: Sym,
        text: Rc<str>,
    },
    Nular(CommandId),
    Unary {
        op: CommandId,
        op_span: Span,
        arg: Box<Expr>,
    },
    Binary {
        op: CommandId,
        op_span: Span,
        left: Box<Expr>,
        right: Box<Expr>,
    },
}

/// Prints a block as SQF text with minimal parentheses, statements joined
/// by `; `. Parsing the output gives back an equivalent tree.
pub fn print_block(block: &CodeBlock, table: &CommandTable) -> String {
    let mut out = String::new();
    write_statements(&mut out, block, table);
    out
}

/// Prints one expression as SQF text.
pub fn print_expr(expr: &Expr, table: &CommandTable) -> String {
    let mut out = String::new();
    write_expr(&mut out, expr, table);
    out
}

fn write_statements(out: &mut String, block: &CodeBlock, table: &CommandTable) {
    for (i, st) in block.statements.iter().enumerate() {
        if i > 0 {
            out.push_str("; ");
        }
        match st {
            Statement::Expr(e) => write_expr(out, e, table),
            Statement::Assign {
                private,
                name_text,
                value,
                ..
            } => {
                if *private {
                    out.push_str("private ");
                }
                out.push_str(name_text);
                out.push_str(" = ");
                write_expr(out, value, table);
            }
        }
    }
}

fn is_word(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
}

fn prec_of(e: &Expr, table: &CommandTable) -> Option<u8> {
    match &e.kind {
        ExprKind::Binary { op, .. } => Some(binary_precedence(&table.get(*op).name)),
        _ => None,
    }
}

fn write_expr(out: &mut String, e: &Expr, table: &CommandTable) {
    match &e.kind {
        ExprKind::Number(n) => {
            let _ = write!(out, "{n}");
        }
        ExprKind::String(s) => {
            out.push('"');
            for c in s.chars() {
                if c == '"' {
                    out.push('"');
                }
                out.push(c);
            }
            out.push('"');
        }
        ExprKind::Array(items) => {
            out.push('[');
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_expr(out, it, table);
            }
            out.push(']');
        }
        ExprKind::Code(block) => {
            out.push('{');
            write_statements(out, block, table);
            out.push('}');
        }
        ExprKind::Variable { text, .. } => out.push_str(text),
        ExprKind::Nular(id) => out.push_str(&table.get(*id).name),
        ExprKind::Unary { op, arg, .. } => {
            let name = &table.get(*op).name;
            out.push_str(name);
            if is_word(name) {
                out.push(' ');
            }
            if prec_of(arg, table).is_some() {
                out.push('(');
                write_expr(out, arg, table);
                out.push(')');
            } else {
                write_expr(out, arg, table);
            }
        }
        ExprKind::Binary {
            op, left, right, ..
        } => {
            let name = &table.get(*op).name;
            let p = binary_precedence(name);
            let lp = prec_of(left, table).is_some_and(|lp| lp < p);
            let rp = prec_of(right, table).is_some_and(|rp| rp <= p);
            write_paren(out, left, table, lp);
            out.push(' ');
            out.push_str(name);
            out.push(' ');
            write_paren(out, right, table, rp);
        }
    }
}

fn write_paren(out: &mut String, e: &Expr, table: &CommandTable, paren: bool) {
    if paren {
        out.push('(');
    }
    write_expr(out, e, table);
    if paren {
        out.push(')');
    }
}
