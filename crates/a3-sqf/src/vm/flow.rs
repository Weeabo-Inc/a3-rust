//! What a command asks the VM to do next.
//!
//! Most commands return a value. Control-flow commands instead return a
//! [`Flow`]: run some code and hand its result to a [`Continuation`], suspend
//! the script, or unwind frames (`exitWith`, `breakOut`, `throw`, ...).
//! Because loops are continuations on the VM's frame stack rather than Rust
//! loops, a scheduled script can suspend anywhere, even inside `forEach`
//! inside `call`.

use std::rc::Rc;

use crate::code::Code;
use crate::error::SqfError;
use crate::host::Host;
use crate::symbol::Sym;
use crate::value::{Namespace, SwitchState, Value};
use crate::vm::Ctx;

/// The outcome of a command.
pub enum Flow<H: Host> {
    /// The command's result.
    Value(Value),
    /// Run code in a new scope; its result is the command's result (or,
    /// from a continuation, is passed back to the same continuation).
    Call(Invoke),
    /// Run code in a new scope and pass its result to a continuation.
    CallThen(Invoke, Box<dyn Continuation<H>>),
    /// Suspend the script (scheduled environment only). When it wakes, the
    /// command's result is `Nothing` (or the continuation is resumed with
    /// `Nothing`).
    Suspend(Suspend),
    /// Leave frames.
    Unwind(Unwind),
}

impl<H: Host> From<Value> for Flow<H> {
    fn from(v: Value) -> Self {
        Flow::Value(v)
    }
}

/// The private variables of one scope. Most scopes have a handful
/// (`_this`, `_x`, `_forEachIndex`), kept inline without a heap allocation.
pub type Locals = smallvec::SmallVec<[(Sym, Value); 2]>;

/// A request to run code in a new scope.
#[derive(Clone, Debug)]
pub struct Invoke {
    pub code: Code,
    /// The new `_this`, or `None` to keep the caller's.
    pub this: Option<Value>,
    /// Further private variables of the new scope (`_x`, `_forEachIndex`,
    /// ...).
    pub locals: Locals,
    /// Namespace for global variables in the new scope (`with ... do`);
    /// `None` inherits the caller's.
    pub namespace: Option<Namespace>,
    /// Keep the scope's locals when it ends (read back with
    /// [`Ctx::take_captured`]); `for` loops use this to carry their loop
    /// scope from one block to the next.
    pub capture: bool,
    /// Reading a `nil` variable in this scope (and scopes it calls) is not
    /// an error (`isNil {...}`).
    pub nil_ok: bool,
    /// The scope and everything it calls runs in the unscheduled
    /// environment (`canSuspend` is false in it; `isNil {...}`).
    pub unscheduled: bool,
    pub(crate) switch: Option<Rc<SwitchState>>,
}

impl Invoke {
    pub fn new(code: Code) -> Invoke {
        Invoke {
            code,
            this: None,
            locals: Locals::new(),
            namespace: None,
            capture: false,
            nil_ok: false,
            unscheduled: false,
            switch: None,
        }
    }

    pub fn with_this(code: Code, this: Value) -> Invoke {
        Invoke {
            this: Some(this),
            ..Invoke::new(code)
        }
    }

    pub fn local(mut self, name: Sym, value: Value) -> Invoke {
        self.locals.push((name, value));
        self
    }
}

/// Why a scheduled script suspends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Suspend {
    /// `sleep`: until mission time advances by the given seconds.
    Sleep(f32),
    /// `uiSleep`: until real time advances by the given seconds.
    UiSleep(f32),
    /// Until the next frame (`waitUntil` with a false condition).
    NextFrame,
}

/// A non-local exit.
#[derive(Clone, Debug)]
pub enum Unwind {
    /// Leave the innermost code scope with a value (`exitWith`).
    ExitScope(Value),
    /// Leave the scope named by `scopeName` with a value (`breakOut`).
    BreakOut(Rc<str>, Value),
    /// Return to the scope named by `scopeName`, which continues (`breakTo`).
    BreakTo(Rc<str>),
    /// Raise an exception for the nearest `try`/`catch` (`throw`).
    Throw(Value),
    /// Leave the innermost loop (`break`).
    Break,
    /// Leave the innermost loop, which returns the value (`breakWith`).
    BreakWith(Value),
    /// Skip to the innermost loop's next iteration with a value for the
    /// current one (`continue`, `continueWith`).
    Continue(Value),
    /// End the script (`terminate _thisScript`).
    Terminate,
}

/// What a continuation frame is, for unwinding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContinuationKind {
    Other,
    /// A loop: target of `break` and `continue`.
    Loop,
    /// A `try` waiting for exceptions.
    Try,
}

/// A native frame on the VM stack: the rest of a command that runs code.
pub trait Continuation<H: Host> {
    /// The code this continuation invoked returned `result`.
    fn resume(&mut self, ctx: &mut Ctx<'_, H>, result: Value) -> Result<Flow<H>, SqfError>;

    /// The code this continuation invoked left its scope through `exitWith`
    /// or `breakOut` with `value`. Loops stop; the default returns `value`
    /// as the command's result.
    fn exited(&mut self, _ctx: &mut Ctx<'_, H>, value: Value) -> Result<Flow<H>, SqfError> {
        Ok(Flow::Value(value))
    }

    fn kind(&self) -> ContinuationKind {
        ContinuationKind::Other
    }

    /// A `throw` reached this continuation (only called when
    /// [`kind`](Self::kind) is [`ContinuationKind::Try`]).
    fn catch(&mut self, _ctx: &mut Ctx<'_, H>, exception: Value) -> Result<Flow<H>, SqfError> {
        Ok(Flow::Unwind(Unwind::Throw(exception)))
    }
}
