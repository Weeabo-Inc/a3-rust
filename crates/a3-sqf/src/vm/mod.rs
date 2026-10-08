//! The SQF virtual machine.
//!
//! [`Vm<H>`] owns the global variables, the command [`Registry`] and the
//! host. [`Vm::call`] runs code in the unscheduled environment: to
//! completion, without suspension. Scheduled scripts (`spawn`, `execVM`)
//! are added by the scheduler.
//!
//! # Scopes
//!
//! Every block (`call`, `then`, loop bodies, ...) runs in a new scope.
//! Assigning `_x = 1` overwrites the innermost existing `_x` in any
//! enclosing scope, including the caller's (SQF scoping is dynamic), and
//! otherwise creates `_x` in the current scope. `private _x = 1`, `private
//! "_x"` and `params` always create the variable in the current scope.
//!
//! # Errors
//!
//! A runtime error (type error, undefined variable in an expression, zero
//! divisor, unimplemented command, ...) is reported to
//! [`Host::report_error`] in the engine's format and aborts the whole
//! script, including every calling scope, as the engine does.

pub mod exec;
pub mod flow;
pub mod vars;

use std::rc::Rc;

pub use flow::{Continuation, ContinuationKind, Flow, Invoke, Suspend, Unwind};
pub use vars::{Namespaces, Variables};

use crate::code::{Code, compile_source};
use crate::error::{CompileError, ScriptError, SqfError};
use crate::host::Host;
use crate::registry::Registry;
use crate::source::SourceFile;
use crate::symbol::Sym;
use crate::table::CommandTable;
use crate::value::{Namespace, ScriptHandle, SwitchState, Value};
use exec::{Outcome, ScriptState};

/// VM state shared by all scripts.
pub struct VmState<H: Host> {
    pub(crate) namespaces: Namespaces,
    pub(crate) rng: Rng,
    pub(crate) scheduler: crate::scheduler::Scheduler<H>,
}

impl<H: Host> Default for VmState<H> {
    fn default() -> Self {
        VmState {
            namespaces: Namespaces::default(),
            rng: Rng::new(0x5EED_1234_ABCD_0001),
            scheduler: crate::scheduler::Scheduler::default(),
        }
    }
}

/// An SQF virtual machine with host `H`.
pub struct Vm<H: Host> {
    /// The embedding engine's services.
    pub host: H,
    pub(crate) registry: Rc<Registry<H>>,
    pub(crate) state: VmState<H>,
}

impl<H: Host> Vm<H> {
    /// A VM with every core command registered.
    pub fn new(host: H) -> Vm<H> {
        Vm::with_registry(host, Rc::new(Registry::with_core()))
    }

    /// A VM over a prepared registry (core plus host commands).
    pub fn with_registry(host: H, registry: Rc<Registry<H>>) -> Vm<H> {
        Vm {
            host,
            registry,
            state: VmState::default(),
        }
    }

    pub fn registry(&self) -> &Registry<H> {
        &self.registry
    }

    pub fn table(&self) -> &CommandTable {
        self.registry.table()
    }

    /// Compiles anonymous source text.
    pub fn compile(&self, text: &str) -> Result<Code, CompileError> {
        self.compile_file("", text)
    }

    /// Compiles source text attributed to `name` in error messages.
    pub fn compile_file(&self, name: &str, text: &str) -> Result<Code, CompileError> {
        compile_source(&SourceFile::new(name, text), self.table())
    }

    /// Runs `code` unscheduled with `_this` set to `this` (nil if `None`)
    /// and returns its value. Errors are also sent to
    /// [`Host::report_error`].
    pub fn call(&mut self, code: &Code, this: Option<Value>) -> Result<Value, ScriptError> {
        let Vm {
            host,
            registry,
            state,
        } = self;
        call_unscheduled(host, registry, state, code, this, Namespace::Mission)
    }

    /// Compiles and runs `text` unscheduled. A compile error is reported
    /// like a runtime error.
    pub fn eval(&mut self, text: &str) -> Result<Value, ScriptError> {
        let src = SourceFile::new("", text);
        let code = match compile_source(&src, self.registry.table()) {
            Ok(c) => c,
            Err(e) => {
                let err = ScriptError::new(
                    SqfError::Generic(e.message.clone()),
                    None,
                    Some((&src, e.span.start)),
                );
                self.host.report_error(&err);
                return Err(err);
            }
        };
        self.call(&code, None)
    }

    /// The variables of a namespace.
    pub fn namespace(&self, ns: Namespace) -> &Variables {
        self.state.namespaces.get(ns)
    }

    pub fn namespace_mut(&mut self, ns: Namespace) -> &mut Variables {
        self.state.namespaces.get_mut(ns)
    }

    /// Reads a `missionNamespace` variable.
    pub fn get_global(&self, name: &str) -> Value {
        self.namespace(Namespace::Mission)
            .get(Sym::new(name))
            .cloned()
            .unwrap_or(Value::Nil)
    }

    /// Sets a `missionNamespace` variable.
    pub fn set_global(&mut self, name: &str, value: Value) {
        self.namespace_mut(Namespace::Mission)
            .set(Sym::new(name), value);
    }
}

pub(crate) fn call_unscheduled<H: Host>(
    host: &mut H,
    reg: &Registry<H>,
    state: &mut VmState<H>,
    code: &Code,
    this: Option<Value>,
    namespace: Namespace,
) -> Result<Value, ScriptError> {
    let mut script = ScriptState::new(
        code.clone(),
        this,
        false,
        ScriptHandle::default(),
        namespace,
    );
    match exec::run(host, reg, state, &mut script, None) {
        Outcome::Done(v) => Ok(v),
        Outcome::Terminated => Ok(Value::Nothing),
        Outcome::Failed(e) => {
            host.report_error(&e);
            Err(e)
        }
        Outcome::Suspended(_) | Outcome::OutOfTime => {
            unreachable!("unscheduled scripts neither suspend nor run out of time")
        }
    }
}

/// What a command implementation can reach: the host, the VM state and the
/// running script.
pub struct Ctx<'a, H: Host> {
    /// The embedding engine's services.
    pub host: &'a mut H,
    pub(crate) reg: &'a Registry<H>,
    pub(crate) vm: &'a mut VmState<H>,
    pub(crate) script: &'a mut ScriptState<H>,
}

impl<H: Host> Ctx<'_, H> {
    pub fn table(&self) -> &CommandTable {
        self.reg.table()
    }

    pub fn registry(&self) -> &Registry<H> {
        self.reg
    }

    /// Compiles source text (as `compile` does).
    pub fn compile(&self, name: &str, text: &str) -> Result<Code, CompileError> {
        compile_source(&SourceFile::new(name, text), self.reg.table())
    }

    /// Reads a variable as an expression would (local or global in the
    /// current namespace).
    pub fn get_var(&self, name: Sym) -> Value {
        exec::read_var(self.vm, self.script, name)
    }

    /// Assigns a variable as `name = value` would.
    pub fn set_var(&mut self, name: Sym, value: Value) -> Result<(), SqfError> {
        exec::assign_var(self.vm, self.script, name, value)
    }

    /// Creates or overwrites a private variable in the current scope.
    pub fn set_private(&mut self, name: Sym, value: Value) {
        self.script.set_private(name, value);
    }

    /// Whether a local variable is defined in any enclosing scope.
    pub fn local_exists(&self, name: Sym) -> bool {
        self.script.get_local(name).is_some()
    }

    /// The variables of a namespace.
    pub fn namespace(&self, ns: Namespace) -> &Variables {
        self.vm.namespaces.get(ns)
    }

    pub fn namespace_mut(&mut self, ns: Namespace) -> &mut Variables {
        self.vm.namespaces.get_mut(ns)
    }

    /// The namespace global variables resolve in (changed by `with`).
    pub fn current_namespace(&self) -> Namespace {
        self.script.current_namespace()
    }

    /// Whether the script runs in the scheduled environment
    /// (`canSuspend`).
    pub fn is_scheduled(&self) -> bool {
        self.script.scheduled
    }

    /// The running script's handle (`scriptNull` when unscheduled).
    pub fn script_handle(&self) -> ScriptHandle {
        self.script.handle
    }

    /// The script name set by `scriptName`.
    pub fn script_name(&self) -> Option<Rc<str>> {
        self.script.name.clone()
    }

    pub fn set_script_name(&mut self, name: &str) {
        self.script.name = Some(name.into());
    }

    /// Names the current scope (`scopeName`).
    pub fn set_scope_name(&mut self, name: &str) {
        if let Some(cf) = self.script.top_code_mut() {
            cf.scope_name = Some(name.into());
        }
    }

    /// The `switch` whose block is running, if any.
    pub(crate) fn current_switch(&self) -> Option<Rc<SwitchState>> {
        self.script.top_code().and_then(|cf| cf.switch.clone())
    }

    /// A uniform random number in `[0, 1)`.
    pub fn random(&mut self) -> f32 {
        self.vm.rng.next_f32()
    }

    /// Formats a value as `str` does, using the host for handles.
    pub fn to_sqf_string(&self, v: &Value) -> String {
        let host: &H = self.host;
        v.to_sqf_string_with(&|h| host.format_handle(h))
    }

    /// Formats a value as `format "%1"` does, using the host for handles.
    pub fn to_display_string(&self, v: &Value) -> String {
        let host: &H = self.host;
        v.to_display_string_with(&|h| host.format_handle(h))
    }

    /// Runs code unscheduled to completion right now, in a fresh script
    /// (for event handlers fired from inside a command). Global variables
    /// are shared; locals of the current script are not visible.
    pub fn call_unscheduled(
        &mut self,
        code: &Code,
        this: Option<Value>,
    ) -> Result<Value, ScriptError> {
        let ns = self.current_namespace();
        call_unscheduled(self.host, self.reg, self.vm, code, this, ns)
    }

    /// Reports a non-fatal error at the current position (the script
    /// continues).
    pub fn report(&mut self, error: SqfError) {
        let pos = self.script.error_position();
        let err = ScriptError::new(
            error,
            None,
            pos.as_ref()
                .map(|(code, off)| (code.source_file().as_ref(), *off)),
        );
        self.host.report_error(&err);
    }

    /// Starts a scheduled script (`spawn`).
    pub fn spawn(&mut self, code: Code, this: Value, name: Option<Rc<str>>) -> ScriptHandle {
        self.vm.scheduler.spawn(code, this, name)
    }

    pub(crate) fn scheduler(&mut self) -> &mut crate::scheduler::Scheduler<H> {
        &mut self.vm.scheduler
    }
}

/// The VM's random number generator (xorshift64*). The engine's generator
/// is not reproduced bit for bit.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}
