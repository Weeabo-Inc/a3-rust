//! Control flow: conditionals, loops, `switch`, exceptions, scope exits,
//! `call`, and scheduling (`spawn`, `execVM`, `sleep`, `waitUntil`).
//!
//! Loops are [`Continuation`]s, so every loop can be suspended and resumed.
//!
//! Behaviour notes:
//! - `exitWith` leaves the scope it runs in; inside a loop body it ends the
//!   loop and the loop's result is the `exitWith` value.
//! - `while` stops silently after 10000 iterations in unscheduled code.
//! - `for "_i" from a to b` runs while `_i <= b` (or `>= b` for a negative
//!   step); the loop's counter is private and assignments to `_i` in the
//!   body do not change it _(uncertain)_.
//! - Loops return the value of the last body evaluation.

use std::rc::Rc;

use super::*;
use crate::code::compile_source;
use crate::source::SourceFile;
use crate::symbol::Sym;
use crate::value::{ForSpec, SwitchState};
use crate::vm::{Continuation, ContinuationKind, Ctx, Flow, Invoke, Suspend, Unwind};

/// Maximum `while` iterations in the unscheduled environment.
pub const UNSCHEDULED_WHILE_LIMIT: u32 = 10_000;

/// Leaves the current scope with the result of the code it ran
/// (`exitWith`).
pub struct ExitScope;

impl<H: Host> Continuation<H> for ExitScope {
    fn resume(&mut self, _: &mut Ctx<'_, H>, result: Value) -> Result<Flow<H>, SqfError> {
        Ok(Flow::Unwind(Unwind::ExitScope(result)))
    }
}

/// What an array loop does with each body result.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LoopMode {
    ForEach,
    Count,
    Select,
    Apply,
    FindIf,
}

/// A loop over an array (`forEach`, `count`, `select`, `apply`,
/// `findIf`) or a hash map (`forEach`).
pub struct Loop {
    mode: LoopMode,
    body: Code,
    items: LoopItems,
    index: usize,
    started: bool,
    last: Value,
    count: usize,
    out: Vec<Value>,
}

enum LoopItems {
    /// A live array: elements added during the loop are visited.
    Array(Array),
    /// A snapshot of hash map entries.
    Map(Vec<(Value, Value)>),
}

impl Loop {
    fn new(mode: LoopMode, body: Code, items: LoopItems) -> Loop {
        Loop {
            mode,
            body,
            items,
            index: 0,
            started: false,
            last: Value::Nothing,
            count: 0,
            out: Vec::new(),
        }
    }

    fn current(&self) -> Option<Value> {
        match &self.items {
            LoopItems::Array(a) => a.borrow().get(self.index).cloned(),
            LoopItems::Map(m) => m.get(self.index).map(|(k, _)| k.clone()),
        }
    }

    fn finish(&mut self) -> Value {
        match self.mode {
            LoopMode::ForEach => std::mem::replace(&mut self.last, Value::Nothing),
            LoopMode::Count => Value::Number(self.count as f32),
            LoopMode::Select | LoopMode::Apply => {
                Value::Array(Array::from_vec(std::mem::take(&mut self.out)))
            }
            LoopMode::FindIf => Value::Number(-1.0),
        }
    }

    fn step<H: Host>(&mut self) -> Result<Flow<H>, SqfError> {
        let Some(x) = self.current() else {
            return Ok(Flow::Value(self.finish()));
        };
        let mut inv = Invoke::new(self.body.clone()).local(Sym::X, x);
        if let LoopItems::Map(m) = &self.items {
            inv = inv.local(Sym::Y, m[self.index].1.clone());
        }
        if self.mode == LoopMode::ForEach {
            inv = inv.local(Sym::FOR_EACH_INDEX, Value::Number(self.index as f32));
        }
        Ok(Flow::Call(inv))
    }
}

impl<H: Host> Continuation<H> for Loop {
    fn resume(&mut self, _: &mut Ctx<'_, H>, result: Value) -> Result<Flow<H>, SqfError> {
        if !self.started {
            self.started = true;
            return self.step();
        }
        match self.mode {
            LoopMode::ForEach => self.last = result,
            LoopMode::Count => {
                if let Value::Bool(b) = result {
                    self.count += usize::from(b);
                } else if !result.is_nil() && !matches!(result, Value::Nothing) {
                    return Err(SqfError::type_error(&result, BOOL));
                }
            }
            LoopMode::Select => match result {
                Value::Bool(true) => {
                    if let Some(x) = self.current() {
                        self.out.push(x);
                    }
                }
                Value::Bool(false) => {}
                other => return Err(SqfError::type_error(&other, BOOL)),
            },
            LoopMode::Apply => self.out.push(result),
            LoopMode::FindIf => match result {
                Value::Bool(true) => return Ok(Flow::Value(Value::Number(self.index as f32))),
                Value::Bool(false) => {}
                other => return Err(SqfError::type_error(&other, BOOL)),
            },
        }
        self.index += 1;
        self.step()
    }

    fn kind(&self) -> ContinuationKind {
        ContinuationKind::Loop
    }
}

fn start_loop<H: Host>(mode: LoopMode, body: Code, items: LoopItems) -> Result<Flow<H>, SqfError> {
    let mut l = Loop::new(mode, body, items);
    l.started = true;
    match l.step::<H>()? {
        Flow::Call(inv) => Ok(Flow::CallThen(inv, Box::new(l))),
        other => Ok(other),
    }
}

/// `while {cond} do {body}`.
struct WhileLoop {
    cond: Code,
    body: Code,
    in_body: bool,
    iterations: u32,
    last: Value,
}

impl<H: Host> Continuation<H> for WhileLoop {
    fn resume(&mut self, ctx: &mut Ctx<'_, H>, result: Value) -> Result<Flow<H>, SqfError> {
        if self.in_body {
            self.last = result;
            self.in_body = false;
            self.iterations += 1;
            return Ok(Flow::Call(Invoke::new(self.cond.clone())));
        }
        let go = match result {
            Value::Bool(b) => b,
            other => return Err(SqfError::type_error(&other, BOOL)),
        };
        if !go || (!ctx.is_scheduled() && self.iterations >= UNSCHEDULED_WHILE_LIMIT) {
            return Ok(Flow::Value(std::mem::replace(
                &mut self.last,
                Value::Nothing,
            )));
        }
        self.in_body = true;
        Ok(Flow::Call(Invoke::new(self.body.clone())))
    }

    fn kind(&self) -> ContinuationKind {
        ContinuationKind::Loop
    }
}

/// `for "_i" from a to b step s do {body}`.
struct ForRange {
    var: Sym,
    i: f32,
    to: f32,
    step: f32,
    body: Code,
    last: Value,
}

impl ForRange {
    fn next<H: Host>(&mut self) -> Flow<H> {
        let more = if self.step >= 0.0 {
            self.i <= self.to
        } else {
            self.i >= self.to
        };
        if !more {
            return Flow::Value(std::mem::replace(&mut self.last, Value::Nothing));
        }
        Flow::Call(Invoke::new(self.body.clone()).local(self.var, Value::Number(self.i)))
    }
}

impl<H: Host> Continuation<H> for ForRange {
    fn resume(&mut self, _: &mut Ctx<'_, H>, result: Value) -> Result<Flow<H>, SqfError> {
        self.last = result;
        self.i += self.step;
        Ok(self.next())
    }

    fn kind(&self) -> ContinuationKind {
        ContinuationKind::Loop
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ForPhase {
    Init,
    Cond,
    Body,
    Step,
}

/// `for [{init}, {cond}, {step}] do {body}`.
struct ForCode {
    cond: Code,
    step: Code,
    body: Code,
    phase: ForPhase,
    last: Value,
}

impl<H: Host> Continuation<H> for ForCode {
    fn resume(&mut self, _: &mut Ctx<'_, H>, result: Value) -> Result<Flow<H>, SqfError> {
        let transparent = |code: &Code| Invoke {
            transparent: true,
            ..Invoke::new(code.clone())
        };
        match self.phase {
            ForPhase::Init | ForPhase::Step => {
                self.phase = ForPhase::Cond;
                Ok(Flow::Call(transparent(&self.cond)))
            }
            ForPhase::Cond => match result {
                Value::Bool(true) => {
                    self.phase = ForPhase::Body;
                    Ok(Flow::Call(Invoke::new(self.body.clone())))
                }
                Value::Bool(false) => Ok(Flow::Value(std::mem::replace(
                    &mut self.last,
                    Value::Nothing,
                ))),
                other => Err(SqfError::type_error(&other, BOOL)),
            },
            ForPhase::Body => {
                self.last = result;
                self.phase = ForPhase::Step;
                Ok(Flow::Call(transparent(&self.step)))
            }
        }
    }

    fn kind(&self) -> ContinuationKind {
        if self.phase == ForPhase::Body {
            ContinuationKind::Loop
        } else {
            ContinuationKind::Other
        }
    }
}

/// The body of `switch ... do {...}` ran; now run the selected case.
struct SwitchBody {
    state: Rc<SwitchState>,
    ran_case: bool,
}

impl<H: Host> Continuation<H> for SwitchBody {
    fn resume(&mut self, _: &mut Ctx<'_, H>, result: Value) -> Result<Flow<H>, SqfError> {
        if self.ran_case {
            return Ok(Flow::Value(result));
        }
        self.ran_case = true;
        let selected = self.state.selected.borrow().clone();
        let default = self.state.default.borrow().clone();
        match selected.or(default) {
            Some(code) => Ok(Flow::Call(Invoke::new(code))),
            None => Ok(Flow::Value(Value::Bool(true))),
        }
    }
}

/// `try {...} catch {...}`.
struct TryCatch {
    catch: Code,
    catching: bool,
}

impl<H: Host> Continuation<H> for TryCatch {
    fn resume(&mut self, _: &mut Ctx<'_, H>, result: Value) -> Result<Flow<H>, SqfError> {
        Ok(Flow::Value(result))
    }

    fn kind(&self) -> ContinuationKind {
        if self.catching {
            ContinuationKind::Other
        } else {
            ContinuationKind::Try
        }
    }

    fn catch(&mut self, _: &mut Ctx<'_, H>, exception: Value) -> Result<Flow<H>, SqfError> {
        self.catching = true;
        Ok(Flow::Call(
            Invoke::new(self.catch.clone()).local(Sym::EXCEPTION, exception),
        ))
    }
}

/// `waitUntil {cond}`: re-evaluates the condition once per frame.
struct WaitUntil {
    cond: Code,
    waiting: bool,
}

impl<H: Host> Continuation<H> for WaitUntil {
    fn resume(&mut self, _: &mut Ctx<'_, H>, result: Value) -> Result<Flow<H>, SqfError> {
        if self.waiting {
            self.waiting = false;
            return Ok(Flow::Call(Invoke::new(self.cond.clone())));
        }
        match result {
            Value::Bool(true) => Ok(Flow::Value(Value::Nothing)),
            Value::Bool(false) => {
                self.waiting = true;
                Ok(Flow::Suspend(Suspend::NextFrame))
            }
            other => Err(SqfError::type_error(&other, BOOL)),
        }
    }
}

/// `isNil {code}`.
struct IsNilCode;

impl<H: Host> Continuation<H> for IsNilCode {
    fn resume(&mut self, _: &mut Ctx<'_, H>, result: Value) -> Result<Flow<H>, SqfError> {
        Ok(Flow::Value(Value::Bool(matches!(
            result,
            Value::Nil | Value::Nothing
        ))))
    }
}

fn for_spec(v: &Value) -> ForSpec {
    match v {
        Value::For(spec) => (**spec).clone(),
        _ => unreachable!("registry checked the FOR type"),
    }
}

fn for_range(v: &Value, f: impl FnOnce(&mut f32, &mut f32, &mut f32)) -> Value {
    let mut spec = for_spec(v);
    if let ForSpec::Range { from, to, step, .. } = &mut spec {
        f(from, to, step);
    }
    Value::For(Rc::new(spec))
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    // Conditionals.
    r.unary("if", BOOL, TypeSet::of(Type::If), |_, a| {
        Ok(Value::If(boolean(&a)))
    });
    let if_t = TypeSet::of(Type::If);
    r.binary_flow("then", if_t, CODE, ANY, |_, a, b| {
        if matches!(a, Value::If(true)) {
            Ok(Flow::Call(Invoke::new(expect_code(&b)?)))
        } else {
            Ok(Flow::Value(Value::Nothing))
        }
    });
    r.binary_flow("then", if_t, ARR, ANY, |_, a, b| {
        let arr = array(&b);
        let branches = arr.borrow();
        let pick = if matches!(a, Value::If(true)) { 0 } else { 1 };
        match branches.get(pick) {
            Some(v) => Ok(Flow::Call(Invoke::new(expect_code(v)?))),
            None => Ok(Flow::Value(Value::Nothing)),
        }
    });
    r.binary("else", CODE, CODE, ARR, |_, a, b| Ok(Value::array([a, b])));
    r.binary_flow("exitWith", if_t, CODE, ANY, |_, a, b| {
        if matches!(a, Value::If(true)) {
            Ok(Flow::CallThen(
                Invoke::new(expect_code(&b)?),
                Box::new(ExitScope),
            ))
        } else {
            Ok(Flow::Value(Value::Nothing))
        }
    });

    // Calls.
    r.unary_flow("call", CODE, ANY, |_, a| {
        Ok(Flow::Call(Invoke::new(expect_code(&a)?)))
    });
    r.binary_flow("call", ANY, CODE, ANY, |_, a, b| {
        Ok(Flow::Call(Invoke::with_this(expect_code(&b)?, a)))
    });
    r.binary("spawn", ANY, CODE, SCRIPT, |ctx, a, b| {
        Ok(Value::Script(ctx.spawn(expect_code(&b)?, a, None)))
    });
    r.unary("execVM", STR, SCRIPT, |ctx, a| exec_vm(ctx, Value::Nil, &a));
    r.binary("execVM", ANY, STR, SCRIPT, |ctx, a, b| exec_vm(ctx, a, &b));

    // Loops.
    r.unary("while", CODE, TypeSet::of(Type::While), |_, a| {
        Ok(Value::While(expect_code(&a)?))
    });
    r.binary_flow("do", TypeSet::of(Type::While), CODE, ANY, |_, a, b| {
        let Value::While(cond) = a else {
            unreachable!()
        };
        Ok(Flow::CallThen(
            Invoke::new(cond.clone()),
            Box::new(WhileLoop {
                cond,
                body: expect_code(&b)?,
                in_body: false,
                iterations: 0,
                last: Value::Nothing,
            }),
        ))
    });
    let for_t = TypeSet::of(Type::For);
    r.unary("for", STR, for_t, |_, a| {
        Ok(Value::For(Rc::new(ForSpec::Range {
            var: Sym::new(string(&a)),
            from: 0.0,
            to: 0.0,
            step: 1.0,
        })))
    });
    r.unary("for", ARR, for_t, |_, a| {
        let arr = array(&a);
        let parts = arr.borrow();
        if parts.len() != 3 {
            return Err(SqfError::generic(format!(
                "{} elements provided, 3 expected",
                parts.len()
            )));
        }
        Ok(Value::For(Rc::new(ForSpec::Code {
            init: expect_code(&parts[0])?,
            cond: expect_code(&parts[1])?,
            step: expect_code(&parts[2])?,
        })))
    });
    r.binary("from", for_t, NUM, for_t, |_, a, b| {
        Ok(for_range(&a, |from, _, _| *from = num(&b)))
    });
    r.binary("to", for_t, NUM, for_t, |_, a, b| {
        Ok(for_range(&a, |_, to, _| *to = num(&b)))
    });
    r.binary("step", for_t, NUM, for_t, |_, a, b| {
        Ok(for_range(&a, |_, _, step| *step = num(&b)))
    });
    r.binary_flow("do", for_t, CODE, ANY, |_, a, b| {
        let body = expect_code(&b)?;
        match for_spec(&a) {
            ForSpec::Range {
                var,
                from,
                to,
                step,
            } => {
                let mut l = ForRange {
                    var,
                    i: from,
                    to,
                    step,
                    body,
                    last: Value::Nothing,
                };
                match l.next::<H>() {
                    Flow::Call(inv) => Ok(Flow::CallThen(inv, Box::new(l))),
                    other => Ok(other),
                }
            }
            ForSpec::Code { init, cond, step } => Ok(Flow::CallThen(
                Invoke {
                    transparent: true,
                    ..Invoke::new(init)
                },
                Box::new(ForCode {
                    cond,
                    step,
                    body,
                    phase: ForPhase::Init,
                    last: Value::Nothing,
                }),
            )),
        }
    });
    r.binary_flow("forEach", CODE, ARR, ANY, |_, a, b| {
        start_loop(
            LoopMode::ForEach,
            expect_code(&a)?,
            LoopItems::Array(array(&b)),
        )
    });
    r.binary_flow("forEach", CODE, HASH, ANY, |_, a, b| {
        let Value::HashMap(m) = b else { unreachable!() };
        let entries = m
            .borrow()
            .iter()
            .map(|(k, v)| (k.to_value(), v.clone()))
            .collect();
        start_loop(LoopMode::ForEach, expect_code(&a)?, LoopItems::Map(entries))
    });
    r.binary_flow("count", CODE, ARR, NUM, |_, a, b| {
        start_loop(
            LoopMode::Count,
            expect_code(&a)?,
            LoopItems::Array(array(&b)),
        )
    });
    r.binary_flow("count", CODE, HASH, NUM, |_, a, b| {
        let Value::HashMap(m) = b else { unreachable!() };
        let entries = m
            .borrow()
            .iter()
            .map(|(k, v)| (k.to_value(), v.clone()))
            .collect();
        start_loop(LoopMode::Count, expect_code(&a)?, LoopItems::Map(entries))
    });
    r.binary_flow("select", ARR, CODE, ARR, |_, a, b| {
        start_loop(
            LoopMode::Select,
            expect_code(&b)?,
            LoopItems::Array(array(&a).shallow_copy()),
        )
    });
    r.binary_flow("apply", ARR, CODE, ARR, |_, a, b| {
        start_loop(
            LoopMode::Apply,
            expect_code(&b)?,
            LoopItems::Array(array(&a).shallow_copy()),
        )
    });
    r.binary_flow("apply", HASH, CODE, ARR, |_, a, b| {
        let Value::HashMap(m) = a else { unreachable!() };
        let entries = m
            .borrow()
            .iter()
            .map(|(k, v)| (k.to_value(), v.clone()))
            .collect();
        start_loop(LoopMode::Apply, expect_code(&b)?, LoopItems::Map(entries))
    });
    r.binary_flow("findIf", ARR, CODE, NUM, |_, a, b| {
        start_loop(
            LoopMode::FindIf,
            expect_code(&b)?,
            LoopItems::Array(array(&a)),
        )
    });

    // switch.
    let sw_t = TypeSet::of(Type::Switch);
    r.unary("switch", ANY, sw_t, |_, a| {
        Ok(Value::Switch(Rc::new(SwitchState {
            value: a,
            ..SwitchState::default()
        })))
    });
    r.binary_flow("do", sw_t, CODE, ANY, |_, a, b| {
        let Value::Switch(state) = a else {
            unreachable!()
        };
        let inv = Invoke {
            switch: Some(state.clone()),
            ..Invoke::new(expect_code(&b)?)
        };
        Ok(Flow::CallThen(
            inv,
            Box::new(SwitchBody {
                state,
                ran_case: false,
            }),
        ))
    });
    r.unary("case", ANY, sw_t, |ctx, a| {
        let Some(state) = ctx.current_switch() else {
            return Err(SqfError::generic("case outside of switch"));
        };
        let hit = *state.matched.borrow() || state.value.is_equal_to(&a);
        if hit {
            *state.matched.borrow_mut() = true;
        }
        Ok(Value::Switch(Rc::new(SwitchState {
            value: Value::Bool(hit),
            ..SwitchState::default()
        })))
    });
    r.binary(":", sw_t, CODE, NOTHING, |ctx, a, b| {
        let Value::Switch(case) = a else {
            unreachable!()
        };
        if matches!(case.value, Value::Bool(true)) {
            if let Some(state) = ctx.current_switch() {
                let mut sel = state.selected.borrow_mut();
                if sel.is_none() {
                    *sel = Some(expect_code(&b)?);
                }
            }
        }
        Ok(Value::Nothing)
    });
    r.unary("default", CODE, NOTHING, |ctx, a| {
        if let Some(state) = ctx.current_switch() {
            *state.default.borrow_mut() = Some(expect_code(&a)?);
        }
        Ok(Value::Nothing)
    });

    // Exceptions and scope exits.
    let exc_t = TypeSet::of(Type::Exception);
    r.unary("try", CODE, exc_t, |_, a| {
        Ok(Value::Exception(expect_code(&a)?))
    });
    r.binary_flow("catch", exc_t, CODE, ANY, |_, a, b| {
        let Value::Exception(body) = a else {
            unreachable!()
        };
        Ok(Flow::CallThen(
            Invoke::new(body),
            Box::new(TryCatch {
                catch: expect_code(&b)?,
                catching: false,
            }),
        ))
    });
    r.unary_flow("throw", ANY, NOTHING, |_, a| {
        Ok(Flow::Unwind(Unwind::Throw(a)))
    });
    r.binary_flow("throw", if_t, ANY, NOTHING, |_, a, b| {
        if matches!(a, Value::If(true)) {
            Ok(Flow::Unwind(Unwind::Throw(b)))
        } else {
            Ok(Flow::Value(Value::Nothing))
        }
    });
    r.unary("scopeName", STR, NOTHING, |ctx, a| {
        ctx.set_scope_name(string(&a));
        Ok(Value::Nothing)
    });
    r.unary_flow("breakOut", STR, NOTHING, |_, a| {
        Ok(Flow::Unwind(Unwind::BreakOut(
            string(&a).into(),
            Value::Nil,
        )))
    });
    r.binary_flow("breakOut", ANY, STR, NOTHING, |_, a, b| {
        Ok(Flow::Unwind(Unwind::BreakOut(string(&b).into(), a)))
    });
    r.unary_flow("breakTo", STR, NOTHING, |_, a| {
        Ok(Flow::Unwind(Unwind::BreakTo(string(&a).into())))
    });
    r.nular_flow("break", NOTHING, |_| Ok(Flow::Unwind(Unwind::Break)));
    r.nular_flow("continue", NOTHING, |_| {
        Ok(Flow::Unwind(Unwind::Continue(Value::Nothing)))
    });
    r.unary_flow("continueWith", ANY, NOTHING, |_, a| {
        Ok(Flow::Unwind(Unwind::Continue(a)))
    });

    // Scheduling.
    r.unary_flow("sleep", NUM, NOTHING, |_, a| {
        Ok(Flow::Suspend(Suspend::Sleep(num(&a))))
    });
    r.unary_flow("uiSleep", NUM, NOTHING, |_, a| {
        Ok(Flow::Suspend(Suspend::UiSleep(num(&a))))
    });
    r.unary_flow("waitUntil", CODE, NOTHING, |ctx, a| {
        if !ctx.is_scheduled() {
            return Err(SqfError::SuspendNotAllowed);
        }
        let cond = expect_code(&a)?;
        Ok(Flow::CallThen(
            Invoke::new(cond.clone()),
            Box::new(WaitUntil {
                cond,
                waiting: false,
            }),
        ))
    });
    r.nular("canSuspend", BOOL, |ctx| {
        Ok(Value::Bool(ctx.is_scheduled()))
    });
    r.unary("scriptDone", SCRIPT, BOOL, |ctx, a| {
        let Value::Script(h) = a else { unreachable!() };
        Ok(Value::Bool(ctx.scheduler().is_done(h)))
    });
    r.unary_flow("terminate", SCRIPT, NOTHING, |ctx, a| {
        let Value::Script(h) = a else { unreachable!() };
        if h.0 != 0 && h == ctx.script_handle() {
            return Ok(Flow::Unwind(Unwind::Terminate));
        }
        ctx.scheduler().terminate(h);
        Ok(Flow::Value(Value::Nothing))
    });
    r.nular("scriptNull", SCRIPT, |_| {
        Ok(Value::Script(Default::default()))
    });

    // isNil.
    r.unary("isNil", STR, BOOL, |ctx, a| {
        Ok(Value::Bool(ctx.get_var(Sym::new(string(&a))).is_nil()))
    });
    r.unary_flow("isNil", CODE, BOOL, |_, a| {
        Ok(Flow::CallThen(
            Invoke::new(expect_code(&a)?),
            Box::new(IsNilCode),
        ))
    });

    // with namespace do.
    let with_t = TypeSet::of(Type::With);
    r.unary("with", NS, with_t, |_, a| {
        let Value::Namespace(ns) = a else {
            unreachable!()
        };
        Ok(Value::With(ns))
    });
    r.binary_flow("do", with_t, CODE, ANY, |_, a, b| {
        let Value::With(ns) = a else { unreachable!() };
        Ok(Flow::Call(Invoke {
            namespace: Some(ns),
            ..Invoke::new(expect_code(&b)?)
        }))
    });
}

fn exec_vm<H: Host>(ctx: &mut Ctx<'_, H>, this: Value, path: &Value) -> Result<Value, SqfError> {
    let path = string(path);
    let text = ctx
        .host
        .preprocess_file(path, true)
        .map_err(SqfError::Generic)?;
    let src = SourceFile::new(path, text);
    let code = compile_source(&src, ctx.table()).map_err(|e| SqfError::Generic(e.message))?;
    Ok(Value::Script(ctx.spawn(code, this, Some(path.into()))))
}
