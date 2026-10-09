//! SQF, the Real Virtuality scripting language: values, lexer, parser,
//! compiler, command registry, VM and scheduler.
//!
//! Pipeline: text → [`lexer`] tokens → [`parser`] syntax tree ([`ast`]) →
//! [`code`] instruction stream → [`Vm`]. The parser resolves identifiers
//! through a [`CommandTable`], which knows every engine command's forms
//! (nular, unary, binary) and binary precedence (`data/commands.tsv`, from
//! the game binary). Commands are implemented in a [`Registry`]; the core,
//! world-independent ones live in [`commands`], and other crates add theirs
//! against the same registry with their own requirements on the [`Host`].
//! See `docs/adr/0005-sqf-vm-architecture.md`.
//!
//! ```
//! use a3_sqf::{NullHost, Vm};
//!
//! let mut vm = Vm::new(NullHost);
//! let v = vm.eval("_a = [1, 2, 3]; _a pushBack 4; (_a select 1 + 1) * count _a").unwrap();
//! assert_eq!(v.to_sqf_string(), "12");
//! ```

pub mod ast;
pub mod code;
pub mod commands;
pub mod error;
pub mod fsm;
pub mod host;
pub mod lexer;
pub mod number;
pub mod parser;
pub mod preprocess;
pub mod registry;
pub mod scheduler;
pub mod source;
pub mod symbol;
pub mod table;
pub mod types;
pub mod value;
pub mod vm;

pub use code::{Code, Instr, compile_block, compile_source, compile_str};
pub use error::{CompileError, ScriptError, SqfError};
pub use host::{Host, NullHost};
pub use preprocess::{HostResolver, SqfEvaluator, preprocess_with_host};
pub use registry::{Coverage, Registry};
pub use scheduler::{DEFAULT_FRAME_BUDGET, FrameReport};
pub use source::{Location, SourceFile, Span};
pub use symbol::Sym;
pub use table::{CommandId, CommandInfo, CommandTable, Form, Signature};
pub use types::{Type, TypeSet};
pub use value::{
    Array, ForSpec, Handle, HandleKind, HashKey, HashMap, Namespace, ScriptHandle, Side, Value,
};
pub use vm::{Ctx, Flow, Invoke, Vm};
