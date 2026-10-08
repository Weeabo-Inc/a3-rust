//! SQF, the Real Virtuality scripting language: values, lexer, parser,
//! compiler and command signature table.
//!
//! Pipeline: text → [`lexer`] tokens → [`parser`] syntax tree ([`ast`]) →
//! [`code`] instruction stream. The parser resolves identifiers through a
//! [`CommandTable`], which knows every engine command's forms (nular, unary,
//! binary) and binary precedence.
//!
//! ```
//! use a3_sqf::{CommandTable, compile_str};
//!
//! let table = CommandTable::builtin();
//! let code = compile_str("", "_a = [1, 2] select 0 + 1", &table).unwrap();
//! assert_eq!(code.source(), "_a = [1, 2] select 0 + 1");
//! ```

pub mod ast;
pub mod code;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod source;
pub mod symbol;
pub mod table;
pub mod types;
pub mod value;

pub use code::{Code, Instr, compile_block, compile_source, compile_str};
pub use error::CompileError;
pub use source::{Location, SourceFile, Span};
pub use symbol::Sym;
pub use table::{CommandId, CommandInfo, CommandTable, Form, Signature};
pub use types::{Type, TypeSet};
pub use value::{
    Array, ForSpec, Handle, HandleKind, HashKey, HashMap, Namespace, ScriptHandle, Side, Value,
};
