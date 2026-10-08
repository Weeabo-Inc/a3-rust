//! SQF parser: tokens to [`CodeBlock`].
//!
//! Grammar (precedence of binary commands from [`binary_precedence`]):
//!
//! ```text
//! code       := [statement] { (";" | ",") [statement] }
//! statement  := ["private"] IDENT "=" expression | expression
//! expression := operand { BINARY expression }      (precedence climbing,
//!                                                   left-associative)
//! operand    := NUMBER | STRING | "[" [expression {"," expression}] "]"
//!             | "{" code "}" | "(" expression ")"
//!             | UNARY operand | NULAR | VARIABLE
//! ```
//!
//! An identifier is resolved through the [`CommandTable`]: a unary command
//! takes the following operand; a nular command is a value; anything else is
//! a variable. A name that is both nular and unary is unary when an operand
//! follows. Identifiers starting with `_` are always local variables.

use std::rc::Rc;

use crate::ast::{CodeBlock, Expr, ExprKind, Statement};
use crate::error::CompileError;
use crate::lexer::{Token, TokenKind, tokenize};
use crate::source::Span;
use crate::symbol::Sym;
use crate::table::{CommandId, CommandTable, Form};

/// Parses a whole script.
pub fn parse(text: &str, table: &CommandTable) -> Result<CodeBlock, CompileError> {
    let tokens = tokenize(text)?;
    let mut p = Parser {
        text,
        tokens,
        pos: 0,
        table,
    };
    let start = 0;
    let statements = p.statements(&TokenKind::Eof)?;
    Ok(CodeBlock {
        statements,
        span: Span::new(start, text.len()),
    })
}

/// Parses a single expression (the whole text must be one expression).
pub fn parse_expression(text: &str, table: &CommandTable) -> Result<Expr, CompileError> {
    let tokens = tokenize(text)?;
    let mut p = Parser {
        text,
        tokens,
        pos: 0,
        table,
    };
    let e = p.expression(0)?;
    if p.peek() != &TokenKind::Eof {
        return Err(CompileError::new("Missing ;", p.span()));
    }
    Ok(e)
}

struct Parser<'a> {
    text: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    table: &'a CommandTable,
}

impl Parser<'_> {
    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn peek_at(&self, ahead: usize) -> &TokenKind {
        let i = (self.pos + ahead).min(self.tokens.len() - 1);
        &self.tokens[i].kind
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn text_of(&self, i: usize) -> &str {
        let s = self.tokens[i].span;
        &self.text[s.start as usize..s.end as usize]
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn statements(&mut self, end: &TokenKind) -> Result<Vec<Statement>, CompileError> {
        let mut out = Vec::new();
        loop {
            while matches!(self.peek(), TokenKind::Semicolon | TokenKind::Comma) {
                self.advance();
            }
            if self.peek() == end {
                return Ok(out);
            }
            if self.peek() == &TokenKind::Eof {
                let msg = match end {
                    TokenKind::RBrace => "Missing }",
                    _ => "Missing ;",
                };
                return Err(CompileError::new(msg, self.span()));
            }
            out.push(self.statement()?);
            match self.peek() {
                TokenKind::Semicolon | TokenKind::Comma => {
                    self.advance();
                }
                k if k == end => return Ok(out),
                TokenKind::Eof => {
                    return Err(CompileError::new("Missing }", self.span()));
                }
                _ => return Err(CompileError::new("Missing ;", self.span())),
            }
        }
    }

    fn is_ident(&self, ahead: usize) -> bool {
        self.peek_at(ahead) == &TokenKind::Ident
    }

    fn statement(&mut self) -> Result<Statement, CompileError> {
        let start = self.span();
        // private _x = value
        if self.is_ident(0)
            && self.text_of(self.pos).eq_ignore_ascii_case("private")
            && self.is_ident(1)
        {
            let name_idx = self.pos + 1;
            let name = self.text_of(name_idx).to_string();
            if self.peek_at(2) == &TokenKind::Assign {
                if !name.starts_with('_') {
                    return Err(CompileError::new(
                        "Local variable in global space",
                        self.tokens[name_idx].span,
                    ));
                }
                self.advance();
                self.advance();
                self.advance();
                let value = self.expression(0)?;
                let span = start.to(value.span);
                return Ok(Statement::Assign {
                    private: true,
                    name: Sym::new(&name),
                    name_text: name.into(),
                    value,
                    span,
                });
            }
            if name.starts_with('_') {
                // `private _x;` declares the variable: same as `private "_x"`.
                let op = self.table.lookup("private").expect("private is a command");
                let kw = self.advance();
                let var = self.advance();
                return Ok(Statement::Expr(Expr {
                    span: kw.span.to(var.span),
                    kind: ExprKind::Unary {
                        op,
                        op_span: kw.span,
                        arg: Box::new(Expr {
                            kind: ExprKind::String(name.into()),
                            span: var.span,
                        }),
                    },
                }));
            }
        }
        // `0 = value`: shipped scripts assign to a number literal to discard
        // a result (`0 = [] spawn {...}`); the engine accepts it.
        if matches!(self.peek(), TokenKind::Number(_)) && self.peek_at(1) == &TokenKind::Assign {
            self.advance();
            self.advance();
            return Ok(Statement::Expr(self.expression(0)?));
        }
        // name = value. A command name on the left compiles too: the shipped
        // main menu script assigns `pixelGrid = 16` as a fallback for old
        // executables.
        if self.is_ident(0) && self.peek_at(1) == &TokenKind::Assign {
            let name = self.text_of(self.pos).to_string();
            self.advance();
            self.advance();
            let value = self.expression(0)?;
            let span = start.to(value.span);
            return Ok(Statement::Assign {
                private: false,
                name: Sym::new(&name),
                name_text: name.into(),
                value,
                span,
            });
        }
        Ok(Statement::Expr(self.expression(0)?))
    }

    /// The binary command at the current token, if any.
    fn peek_binary(&self) -> Option<(CommandId, u8)> {
        match self.peek() {
            TokenKind::Operator | TokenKind::Ident => {
                let name = self.text_of(self.pos);
                if name.starts_with('_') {
                    return None;
                }
                let id = self.table.lookup(name)?;
                let info = self.table.get(id);
                info.has_form(Form::Binary).then_some((id, info.precedence))
            }
            _ => None,
        }
    }

    fn expression(&mut self, min_prec: u8) -> Result<Expr, CompileError> {
        let mut left = self.operand()?;
        while let Some((op, prec)) = self.peek_binary() {
            if prec < min_prec {
                break;
            }
            let op_tok = self.advance();
            let right = self.expression(prec + 1)?;
            left = Expr {
                span: left.span.to(right.span),
                kind: ExprKind::Binary {
                    op,
                    op_span: op_tok.span,
                    left: Box::new(left),
                    right: Box::new(right),
                },
            };
        }
        Ok(left)
    }

    /// Whether the current token can begin an operand (used to decide
    /// between the nular and unary reading of a name that is both).
    fn operand_follows(&self) -> bool {
        match self.peek() {
            TokenKind::Number(_)
            | TokenKind::String(_)
            | TokenKind::LBracket
            | TokenKind::LBrace
            | TokenKind::LParen => true,
            TokenKind::Ident => {
                let name = self.text_of(self.pos);
                if name.starts_with('_') {
                    return true;
                }
                match self.table.lookup(name) {
                    Some(id) => {
                        let info = self.table.get(id);
                        !(info.has_form(Form::Binary)
                            && !info.has_form(Form::Unary)
                            && !info.has_form(Form::Nular))
                    }
                    None => true,
                }
            }
            TokenKind::Operator => {
                let name = self.text_of(self.pos);
                self.table.has(name, Form::Unary) && !self.table.has(name, Form::Binary)
            }
            _ => false,
        }
    }

    fn operand(&mut self) -> Result<Expr, CompileError> {
        let tok = self.advance();
        let span = tok.span;
        let kind = match tok.kind {
            TokenKind::Number(n) => ExprKind::Number(n),
            TokenKind::String(s) => ExprKind::String(s),
            TokenKind::LParen => {
                let inner = self.expression(0)?;
                if self.peek() != &TokenKind::RParen {
                    return Err(CompileError::new("Missing )", self.span()));
                }
                let close = self.advance();
                return Ok(Expr {
                    kind: inner.kind,
                    span: span.to(close.span),
                });
            }
            TokenKind::LBracket => {
                let mut items = Vec::new();
                if self.peek() != &TokenKind::RBracket {
                    loop {
                        items.push(self.expression(0)?);
                        match self.peek() {
                            TokenKind::Comma => {
                                self.advance();
                            }
                            TokenKind::RBracket => break,
                            _ => return Err(CompileError::new("Missing ]", self.span())),
                        }
                    }
                }
                let close = self.advance();
                return Ok(Expr {
                    kind: ExprKind::Array(items),
                    span: span.to(close.span),
                });
            }
            TokenKind::LBrace => {
                let statements = self.statements(&TokenKind::RBrace)?;
                let close = self.advance();
                let block = CodeBlock {
                    statements,
                    span: Span {
                        start: span.end,
                        end: close.span.start,
                    },
                };
                return Ok(Expr {
                    kind: ExprKind::Code(Rc::new(block)),
                    span: span.to(close.span),
                });
            }
            TokenKind::Operator => {
                let name = &self.text[span.start as usize..span.end as usize];
                match self.table.lookup(name) {
                    Some(op) if self.table.get(op).has_form(Form::Unary) => {
                        let arg = self.operand()?;
                        return Ok(Expr {
                            span: span.to(arg.span),
                            kind: ExprKind::Unary {
                                op,
                                op_span: span,
                                arg: Box::new(arg),
                            },
                        });
                    }
                    _ => return Err(CompileError::new("Invalid number in expression", span)),
                }
            }
            TokenKind::Ident => {
                let name = &self.text[span.start as usize..span.end as usize];
                if name.starts_with('_') {
                    ExprKind::Variable {
                        name: Sym::new(name),
                        text: name.into(),
                    }
                } else if let Some(id) = self.table.lookup(name) {
                    let info = self.table.get(id);
                    let unary = info.has_form(Form::Unary);
                    let nular = info.has_form(Form::Nular);
                    if unary && (!nular || self.operand_follows()) {
                        let arg = self.operand()?;
                        return Ok(Expr {
                            span: span.to(arg.span),
                            kind: ExprKind::Unary {
                                op: id,
                                op_span: span,
                                arg: Box::new(arg),
                            },
                        });
                    } else if nular {
                        ExprKind::Nular(id)
                    } else if info.has_form(Form::Binary) {
                        return Err(CompileError::new("Invalid number in expression", span));
                    } else {
                        // Known name without forms: treat as a variable.
                        ExprKind::Variable {
                            name: Sym::new(name),
                            text: name.into(),
                        }
                    }
                } else {
                    ExprKind::Variable {
                        name: Sym::new(name),
                        text: name.into(),
                    }
                }
            }
            TokenKind::Eof => return Err(CompileError::new("Missing ;", span)),
            TokenKind::RBracket => return Err(CompileError::new("Missing [", span)),
            TokenKind::RParen => return Err(CompileError::new("Missing (", span)),
            TokenKind::RBrace => return Err(CompileError::new("Missing {", span)),
            TokenKind::Assign | TokenKind::Semicolon | TokenKind::Comma => {
                return Err(CompileError::new("Invalid number in expression", span));
            }
        };
        Ok(Expr { kind, span })
    }
}
