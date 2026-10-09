//! The interpreter loop.
//!
//! A script is a stack of frames. A code frame runs compiled instructions
//! and owns one scope of private variables; a native frame is a
//! [`Continuation`] waiting for the result of code it invoked. Nothing here
//! recurses in Rust per SQF call, so a script can stop between any two
//! instructions (suspension, time budget) and resume later.

use std::rc::Rc;
use std::time::Instant;

use crate::code::{Code, Instr};
use crate::error::{ScriptError, SqfError};
use crate::host::Host;
use crate::registry::{BinaryImpl, NularImpl, Registry, UnaryImpl};
use crate::symbol::Sym;
use crate::value::{Array, Namespace, ScriptHandle, SwitchState, Value};
use crate::vm::flow::{Continuation, ContinuationKind, Flow, Invoke, Locals, Suspend, Unwind};
use crate::vm::{Ctx, VmState};

pub(crate) struct CodeFrame {
    pub code: Code,
    pub ip: usize,
    /// Value stack height when the frame started.
    pub base: usize,
    /// Private variables of this scope.
    pub locals: Locals,
    pub scope_name: Option<Rc<str>>,
    /// Namespace for global variables (`with ... do`).
    pub namespace: Namespace,
    /// The `switch` this block is the body of.
    pub switch: Option<Rc<SwitchState>>,
    /// When the frame ends, its locals are kept in
    /// [`ScriptState::captured`] (loop scopes of `for`).
    pub capture: bool,
    /// Reading a `nil` variable is not an error here (inside `isNil {...}`).
    pub nil_ok: bool,
    /// `privateAll`: locals of enclosing scopes are invisible from here.
    pub private_all: bool,
    /// First scope of a new evaluation context (`isNil {...}`), where
    /// `diag_scope` counts from.
    pub context_root: bool,
    /// An error in this scope ends only the scope, not the script (the code
    /// of `isNil {...}`).
    pub error_barrier: bool,
    /// The scope and everything it calls runs unscheduled (`isNil {...}`).
    pub unscheduled: bool,
}

pub(crate) enum Frame<H: Host> {
    Code(CodeFrame),
    Native(Box<dyn Continuation<H>>),
}

/// One running script: its frames and value stack.
pub(crate) struct ScriptState<H: Host> {
    pub frames: Vec<Frame<H>>,
    pub stack: Vec<Value>,
    /// Runs in the scheduled environment (may suspend).
    pub scheduled: bool,
    pub handle: ScriptHandle,
    pub name: Option<Rc<str>>,
    /// Locals of the last ended frame that had `capture` set.
    pub captured: Option<Locals>,
    /// `forceUnicode` mode: -1 off, 0 on until the script ends, 1 on for
    /// the next string command.
    pub unicode_mode: i8,
    /// Value to hand to the top frame when the script next runs (after a
    /// suspension).
    pub resume_with: Option<Value>,
    /// The private variables of the outermost scope when it ended (kept for
    /// `__EXEC`/`__EVAL`, whose locals persist between calls).
    pub final_locals: Vec<(Sym, Value)>,
    /// `toFixed n`: numbers print with `n` decimals for the rest of the
    /// script (a setting of the script's context, not of a scope).
    pub fixed: Option<u8>,
    /// A script error was already reported: the engine logs the first one
    /// only and goes on (server oracle: `1/0; 5%0; "END"` logs one error).
    pub error_reported: bool,
}

/// How a run of a script ended.
pub(crate) enum Outcome {
    Done(Value),
    Suspended(Suspend),
    /// The time budget ran out; the script continues next time.
    OutOfTime,
    Failed(ScriptError),
    Terminated,
}

enum Next {
    Continue,
    Deliver(Value, bool),
    Unwind(Unwind),
    Suspend(Suspend),
    Finish(Value),
    Fail(SqfError, Option<String>),
    OutOfTime,
    Terminated,
}

impl<H: Host> ScriptState<H> {
    pub fn new(
        code: Code,
        this: Option<Value>,
        scheduled: bool,
        handle: ScriptHandle,
        namespace: Namespace,
    ) -> ScriptState<H> {
        let mut s = ScriptState {
            frames: Vec::new(),
            stack: Vec::new(),
            scheduled,
            handle,
            name: None,
            captured: None,
            unicode_mode: -1,
            resume_with: None,
            final_locals: Vec::new(),
            fixed: None,
            error_reported: false,
        };
        let mut inv = Invoke::new(code);
        inv.this = Some(this.unwrap_or(Value::Nil));
        if scheduled {
            inv.locals.push((Sym::THIS_SCRIPT, Value::Script(handle)));
        }
        s.push_code(inv, namespace);
        s
    }

    /// An unscheduled script whose outermost scope starts with `locals`.
    pub fn new_with_locals(
        code: Code,
        namespace: Namespace,
        locals: Vec<(Sym, Value)>,
    ) -> ScriptState<H> {
        let mut s = ScriptState::new(code, None, false, ScriptHandle::default(), namespace);
        if let Some(Frame::Code(cf)) = s.frames.last_mut() {
            cf.locals.extend(locals);
        }
        s
    }

    pub fn push_code(&mut self, inv: Invoke, namespace: Namespace) {
        let mut locals = inv.locals;
        if let Some(this) = inv.this {
            locals.push((Sym::THIS, this));
        }
        let parent_nil_ok = self.top_code().is_some_and(|cf| cf.nil_ok);
        let nil_ok = inv.nil_ok || parent_nil_ok;
        let parent_unscheduled = self.top_code().is_some_and(|cf| cf.unscheduled);
        self.frames.push(Frame::Code(CodeFrame {
            code: inv.code,
            ip: 0,
            base: self.stack.len(),
            locals,
            scope_name: None,
            namespace: inv.namespace.unwrap_or(namespace),
            switch: inv.switch,
            capture: inv.capture,
            nil_ok,
            private_all: false,
            context_root: inv.nil_ok,
            error_barrier: inv.nil_ok,
            unscheduled: inv.unscheduled || parent_unscheduled,
        }));
    }

    /// Pushes a code frame inheriting the namespace of the current one.
    fn push_invoke(&mut self, inv: Invoke) {
        let ns = self.current_namespace();
        self.push_code(inv, ns);
    }

    fn pop_frame(&mut self) -> Option<Frame<H>> {
        let mut f = self.frames.pop()?;
        if let Frame::Code(cf) = &mut f {
            self.stack.truncate(cf.base);
            if cf.capture {
                self.captured = Some(std::mem::take(&mut cf.locals));
            } else if self.frames.is_empty() {
                self.final_locals = std::mem::take(&mut cf.locals).into_vec();
            }
        }
        Some(f)
    }

    pub fn top_code(&self) -> Option<&CodeFrame> {
        self.frames.iter().rev().find_map(|f| match f {
            Frame::Code(cf) => Some(cf),
            Frame::Native(_) => None,
        })
    }

    pub fn top_code_mut(&mut self) -> Option<&mut CodeFrame> {
        self.frames.iter_mut().rev().find_map(|f| match f {
            Frame::Code(cf) => Some(cf),
            Frame::Native(_) => None,
        })
    }

    pub fn current_namespace(&self) -> Namespace {
        self.top_code()
            .map(|cf| cf.namespace)
            .unwrap_or(Namespace::Mission)
    }

    /// Looks a local variable up through all scopes, innermost first.
    pub fn get_local(&self, name: Sym) -> Option<&Value> {
        for f in self.frames.iter().rev() {
            if let Frame::Code(cf) = f {
                if let Some((_, v)) = cf.locals.iter().rev().find(|(s, _)| *s == name) {
                    return Some(v);
                }
                if cf.private_all {
                    break;
                }
            }
        }
        None
    }

    /// Looks a local variable up through all scopes, ignoring `privateAll`
    /// barriers (`import`). The current scope is skipped.
    pub fn get_local_through_barrier(&self, name: Sym) -> Option<&Value> {
        let mut seen_current = false;
        for f in self.frames.iter().rev() {
            if let Frame::Code(cf) = f {
                if !seen_current {
                    seen_current = true;
                    continue;
                }
                if let Some((_, v)) = cf.locals.iter().rev().find(|(s, _)| *s == name) {
                    return Some(v);
                }
            }
        }
        None
    }

    /// Assigns a local: overwrites the innermost existing variable of that
    /// name, else creates it in the current scope.
    pub fn set_local(&mut self, name: Sym, value: Value) {
        for f in self.frames.iter_mut().rev() {
            if let Frame::Code(cf) = f {
                if let Some(slot) = cf.locals.iter_mut().rev().find(|(s, _)| *s == name) {
                    slot.1 = value;
                    return;
                }
                if cf.private_all {
                    break;
                }
            }
        }
        let target = self.frames.iter_mut().rev().find_map(|f| match f {
            Frame::Code(cf) => Some(cf),
            _ => None,
        });
        match target {
            Some(cf) => cf.locals.push((name, value)),
            None => self.set_private(name, value),
        }
    }

    /// Creates (or overwrites) a variable in the current scope.
    pub fn set_private(&mut self, name: Sym, value: Value) {
        if let Some(cf) = self.top_code_mut() {
            if let Some(slot) = cf.locals.iter_mut().find(|(s, _)| *s == name) {
                slot.1 = value;
            } else {
                cf.locals.push((name, value));
            }
        }
    }

    /// `private "name"`: declares the variable in the current scope holding
    /// nil. A variable that is already there keeps its value, so
    /// `private ["_this"]` inside a call does not hide `_this` (server
    /// oracle).
    pub fn declare_private(&mut self, name: Sym) {
        if let Some(cf) = self.top_code_mut() {
            if !cf.locals.iter().any(|(s, _)| *s == name) {
                cf.locals.push((name, Value::Nil));
            }
        }
    }

    /// The error position: the instruction the innermost code frame is at.
    pub(crate) fn error_position(&self) -> Option<(Code, u32)> {
        self.top_code()
            .map(|cf| (cf.code.clone(), cf.code.offset_of(cf.ip.saturating_sub(1))))
    }
}

/// Reads a variable as `GetVar` does.
pub(crate) fn read_var<H: Host>(vm: &VmState<H>, script: &ScriptState<H>, name: Sym) -> Value {
    if name.is_local() {
        script.get_local(name).cloned().unwrap_or(Value::Nil)
    } else {
        vm.namespaces
            .get(script.current_namespace())
            .get(name)
            .cloned()
            .unwrap_or(Value::Nil)
    }
}

/// Assigns a variable as `name = value` does.
///
/// A namespace variable that holds a final value cannot be overwritten: the
/// engine logs `Attempt to override final function - <name>` and leaves the
/// variable alone, and the script goes on (server oracle:
/// `finA = compileFinal "1"; finA = "2"; "after"` is `"after"`, `str finA` stays
/// `"{1}"`). Assigning `nil` — how a variable is deleted — over a final
/// **function** is refused *silently* instead (`nilF = compileFinal "1";
/// nilF = nil; isNil nilF` is `false` with no error line), while over a final
/// **hash map** it still reports an override attempt (`nilJ = compileFinal
/// createHashMap; nilJ = nil` logs `- nilj` and stays a HASHMAP). Locals are not
/// protected at all: `private _f = compileFinal "1"; _f = "2"` is allowed.
pub(crate) fn assign_var<H: Host>(
    vm: &mut VmState<H>,
    script: &mut ScriptState<H>,
    name: Sym,
    value: Value,
) -> Result<(), SqfError> {
    if name.is_local() {
        script.set_local(name, value);
        return Ok(());
    }
    let vars = vm.namespaces.get_mut(script.current_namespace());
    if let Some(held) = vars.get(name) {
        if is_final(held) {
            if value.is_nil() && matches!(held, Value::Code(_)) {
                return Ok(());
            }
            return Err(SqfError::generic(format!(
                "Attempt to override final function - {name}"
            )));
        }
    }
    vars.set(name, value);
    Ok(())
}

/// Whether `value` is final, so a variable holding it cannot be overwritten:
/// the code of `compileFinal`, or the hash map of `compileFinal createHashMap`
/// (both give "Attempt to override final function", server oracle:
/// `finN = compileFinal createHashMap; finN = 1` leaves a HASHMAP).
///
/// The name in the message is the stored variable name, which the engine — and
/// [`Sym`] — lower-cases, not the name as spelled at the assignment (server
/// oracle: `finC_KeEpS_CaSe = compileFinal "1"; finC_KeEpS_CaSe = 2` logs
/// `- finc_keeps_case`).
pub(crate) fn is_final(value: &Value) -> bool {
    match value {
        Value::Code(c) => c.is_final(),
        Value::HashMap(m) => m.is_read_only(),
        _ => false,
    }
}

/// Reports a script error, once per script: the engine writes the first
/// error of a script to the RPT and swallows the rest (server oracle:
/// `1/0; 5%0; "END"` logs one line).
pub(crate) fn report_error<H: Host>(host: &mut H, script: &mut ScriptState<H>, error: ScriptError) {
    if !script.error_reported {
        script.error_reported = true;
        host.report_error(&error);
    }
}

/// A script error a command raised at the current position.
fn command_error<H: Host>(
    script: &ScriptState<H>,
    error: SqfError,
    command: Option<&str>,
) -> ScriptError {
    let pos = script.error_position();
    ScriptError::new(
        error,
        command,
        pos.as_ref()
            .map(|(code, off)| (code.source_file().as_ref(), *off)),
    )
}

const BUDGET_CHECK_INTERVAL: u32 = 256;

/// Runs `script` until it finishes, fails, suspends or (with a deadline)
/// runs out of time.
pub(crate) fn run<H: Host>(
    host: &mut H,
    reg: &Registry<H>,
    vm: &mut VmState<H>,
    script: &mut ScriptState<H>,
    deadline: Option<Instant>,
) -> Outcome {
    let mut next = match script.resume_with.take() {
        Some(v) => Next::Deliver(v, false),
        None => Next::Continue,
    };
    let mut counter = 0u32;
    loop {
        next = match next {
            Next::Continue => exec_code(host, reg, vm, script, deadline, &mut counter),
            Next::Deliver(v, exited) => deliver(host, reg, vm, script, v, exited),
            Next::Unwind(u) => unwind(host, reg, vm, script, u),
            Next::Suspend(s) => {
                if script.scheduled && !script.top_code().is_some_and(|cf| cf.unscheduled) {
                    script.resume_with = Some(Value::Nothing);
                    return Outcome::Suspended(s);
                }
                // The VM itself cannot suspend here: that ends the script
                // (a command error would only be logged).
                Next::Fail(SqfError::SuspendNotAllowed, None)
            }
            Next::Finish(v) => {
                script.stack.clear();
                return Outcome::Done(v);
            }
            Next::OutOfTime => return Outcome::OutOfTime,
            Next::Terminated => {
                script.frames.clear();
                script.stack.clear();
                return Outcome::Terminated;
            }
            Next::Fail(e, cmd) => {
                let err = command_error(script, e, cmd.as_deref());
                report_error(host, script, err.clone());
                // Inside `isNil {...}` an error ends only that block; the
                // block's value is the empty value and `isNil` is true.
                if let Some(i) = barrier_frame(script) {
                    while script.frames.len() > i {
                        script.pop_frame();
                    }
                    Next::Deliver(Value::Nothing, false)
                } else {
                    script.frames.clear();
                    script.stack.clear();
                    return Outcome::Failed(err);
                }
            }
        };
    }
}

/// The innermost frame whose errors are contained (`isNil {...}`).
fn barrier_frame<H: Host>(script: &ScriptState<H>) -> Option<usize> {
    script
        .frames
        .iter()
        .rposition(|f| matches!(f, Frame::Code(cf) if cf.error_barrier))
}

/// Reports a command error and continues with the empty value: a handler
/// error (a bad index, a type check inside a command, a regexp that does not
/// compile, ...) is logged and the script goes on (server oracle).
///
/// A suspension the VM cannot honour (`sleep`/`waitUntil` in the unscheduled
/// environment) is not a command error but a VM-level failure: it ends the
/// script.
fn report_and_continue<H: Host>(
    host: &mut H,
    script: &mut ScriptState<H>,
    e: SqfError,
    cmd: Option<&str>,
) -> Next {
    if matches!(e, SqfError::SuspendNotAllowed) {
        return Next::Fail(e, cmd.map(str::to_string));
    }
    let err = command_error(script, e, cmd);
    report_error(host, script, err);
    script.stack.push(Value::Nothing);
    Next::Continue
}

fn handle_flow<H: Host>(script: &mut ScriptState<H>, flow: Flow<H>) -> Next {
    match flow {
        Flow::Value(v) => {
            script.stack.push(v);
            Next::Continue
        }
        Flow::Call(inv) => {
            script.push_invoke(inv);
            Next::Continue
        }
        Flow::CallThen(inv, cont) => {
            script.frames.push(Frame::Native(cont));
            script.push_invoke(inv);
            Next::Continue
        }
        Flow::Suspend(s) => Next::Suspend(s),
        Flow::Unwind(u) => Next::Unwind(u),
    }
}

fn exec_code<H: Host>(
    host: &mut H,
    reg: &Registry<H>,
    vm: &mut VmState<H>,
    script: &mut ScriptState<H>,
    deadline: Option<Instant>,
    counter: &mut u32,
) -> Next {
    let fi = script.frames.len() - 1;
    let (code, mut ip, base) = match &script.frames[fi] {
        Frame::Code(cf) => (cf.code.clone(), cf.ip, cf.base),
        Frame::Native(_) => unreachable!("exec_code on a native frame"),
    };
    let instrs = code.instructions();
    macro_rules! save_ip {
        () => {
            if let Frame::Code(cf) = &mut script.frames[fi] {
                cf.ip = ip;
            }
        };
    }
    loop {
        if ip >= instrs.len() {
            let v = if script.stack.len() > base {
                script.stack.pop().unwrap_or(Value::Nothing)
            } else {
                Value::Nothing
            };
            script.pop_frame();
            return Next::Deliver(v, false);
        }
        if let Some(deadline) = deadline {
            *counter += 1;
            if *counter >= BUDGET_CHECK_INTERVAL {
                *counter = 0;
                if Instant::now() >= deadline {
                    save_ip!();
                    return Next::OutOfTime;
                }
            }
        }
        let instr = &instrs[ip];
        ip += 1;
        match instr {
            Instr::Push(v) => script.stack.push(v.clone()),
            Instr::GetVar(name) => {
                let v = read_var(vm, script, *name);
                if v.is_nil() && !matches!(&script.frames[fi], Frame::Code(cf) if cf.nil_ok) {
                    // An undefined variable is an error the script survives:
                    // the read yields nil (server oracle).
                    save_ip!();
                    let err = command_error(
                        script,
                        SqfError::UndefinedVariable(name.as_str().to_string()),
                        None,
                    );
                    report_error(host, script, err);
                    script.stack.push(Value::Nil);
                    continue;
                }
                script.stack.push(v);
            }
            Instr::MakeArray(n) => {
                let at = script.stack.len() - *n as usize;
                let items = script.stack.split_off(at);
                script.stack.push(Value::Array(Array::from_vec(items)));
            }
            Instr::EndStatement => script.stack.truncate(base),
            Instr::Assign(name) => {
                let v = script.stack.pop().unwrap_or(Value::Nil);
                save_ip!();
                if let Err(e) = assign_var(vm, script, *name, v) {
                    return report_and_continue(host, script, e, None);
                }
            }
            Instr::AssignPrivate(name) => {
                let v = script.stack.pop().unwrap_or(Value::Nil);
                if let Frame::Code(cf) = &mut script.frames[fi] {
                    if let Some(slot) = cf.locals.iter_mut().find(|(s, _)| s == name) {
                        slot.1 = v;
                    } else {
                        cf.locals.push((*name, v));
                    }
                }
            }
            Instr::Nular(id) => {
                save_ip!();
                let name = reg.table().get(*id).name.clone();
                let Some(imp) = reg.nular_impl(*id) else {
                    return Next::Fail(SqfError::Unimplemented(name.clone()), Some(name));
                };
                let mut ctx = Ctx {
                    host,
                    reg,
                    vm,
                    script,
                };
                // Value commands push directly; only control flow goes
                // through the (larger) `Flow`.
                let flow = match imp {
                    NularImpl::Value(f) => match f(&mut ctx) {
                        Ok(v) => {
                            script.stack.push(v);
                            continue;
                        }
                        // A command's own error is logged and the script
                        // goes on with the empty value.
                        Err(e) => Err(e),
                    },
                    NularImpl::Flow(f) => f(&mut ctx),
                };
                match flow {
                    Ok(Flow::Value(v)) => script.stack.push(v),
                    Ok(flow) => return handle_flow(script, flow),
                    Err(e) => return report_and_continue(host, script, e, Some(&name)),
                }
            }
            Instr::Unary(id) => {
                save_ip!();
                let name = reg.table().get(*id).name.clone();
                let arg = script.stack.pop().unwrap_or(Value::Nil);
                let imp = match reg.unary_impl(*id, &arg) {
                    Ok(Some(imp)) => imp,
                    Ok(None) => {
                        script.stack.push(Value::Nil);
                        continue;
                    }
                    // No overload takes the argument: the engine stops the
                    // script here (server oracle: `1 + "x"`).
                    Err(e) => return Next::Fail(e, Some(name)),
                };
                let mut ctx = Ctx {
                    host,
                    reg,
                    vm,
                    script,
                };
                let flow = match imp {
                    UnaryImpl::Value(f) => match f(&mut ctx, arg) {
                        Ok(v) => {
                            script.stack.push(v);
                            continue;
                        }
                        Err(e) => Err(e),
                    },
                    UnaryImpl::Flow(f) => f(&mut ctx, arg),
                };
                match flow {
                    Ok(Flow::Value(v)) => script.stack.push(v),
                    Ok(flow) => return handle_flow(script, flow),
                    Err(e) => return report_and_continue(host, script, e, Some(&name)),
                }
            }
            Instr::Binary(id) => {
                save_ip!();
                let name = reg.table().get(*id).name.clone();
                let right = script.stack.pop().unwrap_or(Value::Nil);
                let left = script.stack.pop().unwrap_or(Value::Nil);
                let imp = match reg.binary_impl(*id, &left, &right) {
                    Ok(Some(imp)) => imp,
                    Ok(None) => {
                        script.stack.push(Value::Nil);
                        continue;
                    }
                    Err(e) => return Next::Fail(e, Some(name)),
                };
                let mut ctx = Ctx {
                    host,
                    reg,
                    vm,
                    script,
                };
                let flow = match imp {
                    BinaryImpl::Value(f) => match f(&mut ctx, left, right) {
                        Ok(v) => {
                            script.stack.push(v);
                            continue;
                        }
                        Err(e) => Err(e),
                    },
                    BinaryImpl::Flow(f) => f(&mut ctx, left, right),
                };
                match flow {
                    Ok(Flow::Value(v)) => script.stack.push(v),
                    Ok(flow) => return handle_flow(script, flow),
                    Err(e) => return report_and_continue(host, script, e, Some(&name)),
                }
            }
        }
    }
}

/// Handles what a continuation returned. `cont` is the continuation that
/// produced `flow`, already removed from the frame stack.
fn continuation_flow<H: Host>(
    script: &mut ScriptState<H>,
    cont: Box<dyn Continuation<H>>,
    flow: Result<Flow<H>, SqfError>,
) -> Next {
    match flow {
        Ok(Flow::Value(v)) => Next::Deliver(v, false),
        Ok(Flow::Call(inv)) => {
            script.frames.push(Frame::Native(cont));
            script.push_invoke(inv);
            Next::Continue
        }
        Ok(Flow::CallThen(inv, next)) => {
            script.frames.push(Frame::Native(next));
            script.push_invoke(inv);
            Next::Continue
        }
        Ok(Flow::Suspend(s)) => {
            script.frames.push(Frame::Native(cont));
            Next::Suspend(s)
        }
        Ok(Flow::Unwind(u)) => Next::Unwind(u),
        Err(e) => Next::Fail(e, None),
    }
}

fn deliver<H: Host>(
    host: &mut H,
    reg: &Registry<H>,
    vm: &mut VmState<H>,
    script: &mut ScriptState<H>,
    value: Value,
    exited: bool,
) -> Next {
    match script.frames.last() {
        None => Next::Finish(value),
        Some(Frame::Code(_)) => {
            script.stack.push(value);
            Next::Continue
        }
        Some(Frame::Native(_)) => {
            let Some(Frame::Native(mut cont)) = script.frames.pop() else {
                unreachable!()
            };
            let mut ctx = Ctx {
                host,
                reg,
                vm,
                script,
            };
            let flow = if exited {
                cont.exited(&mut ctx, value)
            } else {
                cont.resume(&mut ctx, value)
            };
            continuation_flow(script, cont, flow)
        }
    }
}

fn unwind<H: Host>(
    host: &mut H,
    reg: &Registry<H>,
    vm: &mut VmState<H>,
    script: &mut ScriptState<H>,
    u: Unwind,
) -> Next {
    match u {
        Unwind::ExitScope(v) => {
            while let Some(f) = script.pop_frame() {
                if let Frame::Code(_) = f {
                    return Next::Deliver(v, true);
                }
            }
            Next::Finish(v)
        }
        Unwind::BreakOut(name, v) => {
            while let Some(f) = script.pop_frame() {
                if let Frame::Code(cf) = &f {
                    if cf
                        .scope_name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case(&name))
                    {
                        return Next::Deliver(v, true);
                    }
                }
            }
            Next::Finish(v)
        }
        Unwind::BreakTo(name) => loop {
            match script.frames.last() {
                None => return Next::Finish(Value::Nil),
                Some(Frame::Code(cf))
                    if cf
                        .scope_name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case(&name)) =>
                {
                    script.stack.push(Value::Nil);
                    return Next::Continue;
                }
                Some(_) => {
                    script.pop_frame();
                }
            }
        },
        Unwind::Throw(exception) => {
            while let Some(f) = script.pop_frame() {
                if let Frame::Native(mut cont) = f {
                    if cont.kind() == ContinuationKind::Try {
                        let mut ctx = Ctx {
                            host,
                            reg,
                            vm,
                            script,
                        };
                        let flow = cont.catch(&mut ctx, exception);
                        return continuation_flow(script, cont, flow);
                    }
                }
            }
            Next::Fail(
                SqfError::UnhandledException(exception.to_display_string()),
                None,
            )
        }
        Unwind::Break | Unwind::BreakWith(_) => {
            let value = match u {
                Unwind::BreakWith(v) => v,
                _ => Value::Nothing,
            };
            while let Some(f) = script.pop_frame() {
                if let Frame::Native(cont) = &f {
                    if cont.kind() == ContinuationKind::Loop {
                        return Next::Deliver(value, false);
                    }
                }
            }
            Next::Finish(value)
        }
        Unwind::Continue(v) => loop {
            match script.frames.last() {
                None => return Next::Finish(Value::Nothing),
                Some(Frame::Native(cont)) if cont.kind() == ContinuationKind::Loop => {
                    return Next::Deliver(v, false);
                }
                Some(_) => {
                    script.pop_frame();
                }
            }
        },
        Unwind::Terminate => Next::Terminated,
    }
}
