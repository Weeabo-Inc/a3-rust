//! Compiled code: the instruction stream the VM runs.
//!
//! Like the engine, the compiler turns each statement into a postfix list of
//! instructions (push a constant, read a variable, call a nular/unary/binary
//! command, build an array, assign) followed by an end-of-statement marker.
//! Nested `{ ... }` blocks compile into their own [`Code`] value, pushed as a
//! constant.

use std::fmt;
use std::rc::Rc;

use crate::ast::{CodeBlock, Expr, ExprKind, Statement};
use crate::error::CompileError;
use crate::parser;
use crate::source::{SourceFile, Span};
use crate::symbol::Sym;
use crate::table::{CommandId, CommandTable};
use crate::value::Value;

/// One VM instruction.
#[derive(Clone, Debug)]
pub enum Instr {
    /// Push a constant (number, string, code).
    Push(Value),
    /// Push the value of a variable (local or global), `nil` if undefined.
    GetVar(Sym),
    /// Call a nular command and push its result.
    Nular(CommandId),
    /// Pop one argument, call a unary command, push the result.
    Unary(CommandId),
    /// Pop right then left argument, call a binary command, push the result.
    Binary(CommandId),
    /// Pop a value and assign it to a variable (an existing local in an
    /// enclosing scope, else a new local in the current scope, or a global).
    Assign(Sym),
    /// Pop a value and assign it to a new local in the current scope.
    AssignPrivate(Sym),
    /// Pop `n` values and push a new array of them.
    MakeArray(u32),
    /// End of a statement: the statement's value (if any) becomes the
    /// block's current result and the stack is cleared.
    EndStatement,
}

/// A compiled block of SQF.
pub struct CompiledCode {
    source: Rc<SourceFile>,
    span: Span,
    instrs: Box<[Instr]>,
    /// Source offset of each instruction, for error positions.
    offsets: Box<[u32]>,
    is_final: bool,
}

impl CompiledCode {
    pub fn instructions(&self) -> &[Instr] {
        &self.instrs
    }

    /// The source offset of instruction `ip`.
    pub fn offset_of(&self, ip: usize) -> u32 {
        self.offsets.get(ip).copied().unwrap_or(self.span.start)
    }
}

/// A reference to compiled code, the payload of a `CODE` value.
#[derive(Clone)]
pub struct Code(Rc<CompiledCode>);

impl Code {
    /// The source text of the block, without braces (what `toString`
    /// returns; `str` adds the braces).
    pub fn source(&self) -> &str {
        self.0.source.slice(self.0.span)
    }

    /// The file the code was compiled from.
    pub fn source_file(&self) -> &Rc<SourceFile> {
        &self.0.source
    }

    pub fn instructions(&self) -> &[Instr] {
        &self.0.instrs
    }

    pub fn offset_of(&self, ip: usize) -> u32 {
        self.0.offset_of(ip)
    }

    /// Whether the code came from `compileFinal` (its variable can't be
    /// overwritten).
    pub fn is_final(&self) -> bool {
        self.0.is_final
    }

    /// A copy of this code marked final.
    pub fn to_final(&self) -> Code {
        if self.0.is_final {
            return self.clone();
        }
        Code(Rc::new(CompiledCode {
            source: self.0.source.clone(),
            span: self.0.span,
            instrs: self.0.instrs.clone(),
            offsets: self.0.offsets.clone(),
            is_final: true,
        }))
    }

    pub fn ptr_eq(&self, other: &Code) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// Whether the code has no instructions.
    pub fn is_empty(&self) -> bool {
        self.0.instrs.is_empty()
    }
}

impl fmt::Debug for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{}}}", self.source())
    }
}

/// Parses and compiles a whole source file.
pub fn compile_source(source: &Rc<SourceFile>, table: &CommandTable) -> Result<Code, CompileError> {
    let block = parser::parse(source.text(), table)?;
    Ok(compile_block(source, &block))
}

/// Parses and compiles a string, as the `compile` command does. `name` is
/// used in error locations (empty for an anonymous string).
pub fn compile_str(name: &str, text: &str, table: &CommandTable) -> Result<Code, CompileError> {
    compile_source(&SourceFile::new(name, text), table)
}

/// Compiles an already parsed block whose spans refer to `source`.
pub fn compile_block(source: &Rc<SourceFile>, block: &CodeBlock) -> Code {
    let mut c = Compiler {
        source,
        instrs: Vec::new(),
        offsets: Vec::new(),
    };
    c.block(block);
    Code(Rc::new(CompiledCode {
        source: source.clone(),
        span: block.span,
        instrs: c.instrs.into_boxed_slice(),
        offsets: c.offsets.into_boxed_slice(),
        is_final: false,
    }))
}

struct Compiler<'a> {
    source: &'a Rc<SourceFile>,
    instrs: Vec<Instr>,
    offsets: Vec<u32>,
}

impl Compiler<'_> {
    fn emit(&mut self, i: Instr, at: Span) {
        self.instrs.push(i);
        self.offsets.push(at.start);
    }

    fn block(&mut self, block: &CodeBlock) {
        for (i, st) in block.statements.iter().enumerate() {
            if i > 0 {
                let at = self.offsets.last().copied().unwrap_or(block.span.start);
                self.instrs.push(Instr::EndStatement);
                self.offsets.push(at);
            }
            match st {
                Statement::Expr(e) => self.expr(e),
                Statement::Assign {
                    private,
                    name,
                    value,
                    span,
                    ..
                } => {
                    self.expr(value);
                    let instr = if *private {
                        Instr::AssignPrivate(*name)
                    } else {
                        Instr::Assign(*name)
                    };
                    self.emit(instr, *span);
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Number(n) => self.emit(Instr::Push(Value::Number(*n)), e.span),
            ExprKind::String(s) => self.emit(Instr::Push(Value::String(s.clone())), e.span),
            ExprKind::Array(items) => {
                for it in items {
                    self.expr(it);
                }
                self.emit(Instr::MakeArray(items.len() as u32), e.span);
            }
            ExprKind::Code(block) => {
                let code = compile_block(self.source, block);
                self.emit(Instr::Push(Value::Code(code)), e.span);
            }
            ExprKind::Variable { name, .. } => self.emit(Instr::GetVar(*name), e.span),
            ExprKind::Nular(id) => self.emit(Instr::Nular(*id), e.span),
            ExprKind::Unary { op, op_span, arg } => {
                self.expr(arg);
                self.emit(Instr::Unary(*op), *op_span);
            }
            ExprKind::Binary {
                op,
                op_span,
                left,
                right,
            } => {
                self.expr(left);
                self.expr(right);
                self.emit(Instr::Binary(*op), *op_span);
            }
        }
    }
}
