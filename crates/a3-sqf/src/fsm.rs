//! Scripted FSMs (`execFSM`): `.fsm` state machines whose code is SQF, run by the scheduler.
//!
//! Every scheduler frame steps each running FSM once ([`a3_fsm::Machine::step`]); the code of
//! a state or link runs unscheduled, to completion. An FSM keeps its own private variables
//! across states — what one state's `init` sets, the next link's condition reads — and its
//! code sees `_this` (the `execFSM` argument) and `_thisFSM` (its handle).
//! `getFSMVariable`/`setFSMVariable` read and write those variables from outside.
//! Semantics: `docs/re/ai-fsm.md` §3.

use std::collections::HashMap;
use std::rc::Rc;

use a3_fsm::{Action, Condition, Driver, Fsm, Machine};

use crate::code::{Code, compile_source};
use crate::host::Host;
use crate::registry::Registry;
use crate::source::SourceFile;
use crate::symbol::Sym;
use crate::table::CommandTable;
use crate::value::{Namespace, Value};
use crate::vm::VmState;
use crate::vm::exec::{self, Outcome, ScriptState};

/// An FSM definition with its SQF compiled once, shared by every FSM started from it.
#[derive(Debug)]
pub struct CompiledFsm {
    fsm: Fsm,
    /// Each distinct piece of code of the FSM; `None` when it does not compile (it then does
    /// nothing, and a condition that does not compile is false).
    code: HashMap<String, Option<Code>>,
    /// Compile errors, for the caller to report.
    errors: Vec<String>,
}

impl CompiledFsm {
    /// Compiles every piece of SQF of `fsm`. `name` labels the code in error messages (the
    /// file path).
    pub fn new(fsm: Fsm, name: &str, table: &CommandTable) -> Self {
        let mut code = HashMap::new();
        let mut errors = Vec::new();
        let texts = fsm.states.iter().flat_map(|s| {
            let state = [script(&s.init), Some(s.precondition.as_str())];
            let links = s.links.iter().flat_map(|l| {
                let condition = match &l.condition {
                    Condition::Script(text) => Some(text.as_str()),
                    Condition::Native(_) => None,
                };
                [Some(l.precondition.as_str()), condition, script(&l.action)]
            });
            state.into_iter().chain(links).flatten()
        });
        for text in texts {
            if text.trim().is_empty() || code.contains_key(text) {
                continue;
            }
            let src = SourceFile::new(name, text);
            let compiled = match compile_source(&src, table) {
                Ok(c) => Some(c),
                Err(e) => {
                    errors.push(format!("{name}: {}", e.message));
                    None
                }
            };
            code.insert(text.to_owned(), compiled);
        }
        CompiledFsm { fsm, code, errors }
    }

    /// The FSM definition.
    pub fn fsm(&self) -> &Fsm {
        &self.fsm
    }

    /// What did not compile.
    pub fn errors(&self) -> &[String] {
        &self.errors
    }
}

fn script(action: &Action) -> Option<&str> {
    match action {
        Action::Script(text) => Some(text),
        Action::Native(_) => None,
    }
}

/// One running scripted FSM.
#[derive(Debug)]
pub(crate) struct ScriptedFsm {
    handle: u32,
    name: Rc<str>,
    compiled: Rc<CompiledFsm>,
    machine: Machine,
    this: Value,
    /// The FSM's private variables (without `_this`/`_thisFSM`).
    locals: Vec<(Sym, Value)>,
}

/// The scripted FSMs of a VM.
#[derive(Debug, Default)]
pub(crate) struct FsmList {
    running: Vec<ScriptedFsm>,
    next_handle: u32,
    /// Parsed and compiled FSM files, by lower-case path.
    cache: HashMap<String, Rc<CompiledFsm>>,
}

impl FsmList {
    /// Starts an FSM; it takes its first step at the next scheduler frame.
    pub(crate) fn start(&mut self, compiled: Rc<CompiledFsm>, this: Value, name: &str) -> u32 {
        self.next_handle += 1;
        let handle = self.next_handle;
        self.running.push(ScriptedFsm {
            handle,
            name: name.into(),
            machine: Machine::new(compiled.fsm()),
            compiled,
            this,
            locals: Vec::new(),
        });
        handle
    }

    pub(crate) fn cached(&self, path: &str) -> Option<Rc<CompiledFsm>> {
        self.cache.get(&path.to_ascii_lowercase()).cloned()
    }

    pub(crate) fn cache(&mut self, path: &str, compiled: Rc<CompiledFsm>) {
        self.cache.insert(path.to_ascii_lowercase(), compiled);
    }

    /// Whether the FSM has ended (`completedFSM`); an unknown handle counts as ended.
    pub(crate) fn is_completed(&self, handle: u32) -> bool {
        !self.running.iter().any(|f| f.handle == handle)
    }

    fn get(&self, handle: u32) -> Option<&ScriptedFsm> {
        self.running.iter().find(|f| f.handle == handle)
    }

    /// `getFSMVariable`.
    pub(crate) fn variable(&self, handle: u32, name: &str) -> Option<Value> {
        let fsm = self.get(handle)?;
        let sym = Sym::new(&name.to_ascii_lowercase());
        if sym == Sym::THIS {
            return Some(fsm.this.clone());
        }
        fsm.locals
            .iter()
            .rev()
            .find(|(n, _)| *n == sym)
            .map(|(_, v)| v.clone())
    }

    /// `setFSMVariable`: sets (or creates) the variable.
    pub(crate) fn set_variable(&mut self, handle: u32, name: &str, value: Value) {
        let Some(fsm) = self.running.iter_mut().find(|f| f.handle == handle) else {
            return;
        };
        let sym = Sym::new(&name.to_ascii_lowercase());
        if sym == Sym::THIS {
            fsm.this = value;
            return;
        }
        match fsm.locals.iter_mut().rev().find(|(n, _)| *n == sym) {
            Some(slot) => slot.1 = value,
            None => fsm.locals.push((sym, value)),
        }
    }

    /// `diag_activeMissionFSMs`: name, state name and (unused) timeout of each running FSM.
    pub(crate) fn active(&self) -> Vec<(Rc<str>, String)> {
        self.running
            .iter()
            .map(|f| {
                let state = f
                    .machine
                    .state()
                    .map(|s| f.compiled.fsm().state(s).name.clone())
                    .unwrap_or_default();
                (f.name.clone(), state)
            })
            .collect()
    }
}

/// Steps every running FSM once; FSMs that end are removed. Runs inside a scheduler frame.
///
/// Each FSM is taken out of the list only while it steps, so the code it runs sees every other
/// FSM (`getFSMVariable`, `completedFSM`); FSMs it starts wait for the next frame.
pub(crate) fn step_all<H: Host>(host: &mut H, reg: &Registry<H>, state: &mut VmState<H>) {
    let handles: Vec<u32> = state.fsms.running.iter().map(|f| f.handle).collect();
    for handle in handles {
        let Some(index) = state.fsms.running.iter().position(|f| f.handle == handle) else {
            continue;
        };
        let mut fsm = state.fsms.running.remove(index);
        let compiled = fsm.compiled.clone();
        let mut driver = ScriptDriver {
            host: &mut *host,
            reg,
            state: &mut *state,
            compiled: &compiled,
            this: fsm.this.clone(),
            handle: fsm.handle,
            locals: std::mem::take(&mut fsm.locals),
        };
        let running = fsm.machine.step(compiled.fsm(), &mut driver);
        fsm.locals = driver.locals;
        if running {
            let index = index.min(state.fsms.running.len());
            state.fsms.running.insert(index, fsm);
        }
    }
}

/// Runs an FSM's code with the FSM's variables.
struct ScriptDriver<'a, H: Host> {
    host: &'a mut H,
    reg: &'a Registry<H>,
    state: &'a mut VmState<H>,
    compiled: &'a CompiledFsm,
    this: Value,
    handle: u32,
    locals: Vec<(Sym, Value)>,
}

impl<H: Host> ScriptDriver<'_, H> {
    fn run(&mut self, text: &str) -> Value {
        if text.trim().is_empty() {
            return Value::Nothing;
        }
        let Some(Some(code)) = self.compiled.code.get(text) else {
            return Value::Nothing;
        };
        let this_fsm = Sym::new("_thisFSM");
        let mut locals = self.locals.clone();
        locals.push((Sym::THIS, self.this.clone()));
        locals.push((this_fsm, Value::Number(self.handle as f32)));
        let mut script = ScriptState::new_with_locals(code.clone(), Namespace::Mission, locals);
        let value = match exec::run(self.host, self.reg, self.state, &mut script, None) {
            Outcome::Done(v) => v,
            Outcome::Failed(e) => {
                // The error ends this piece of code; the FSM keeps the variables it had.
                self.host.report_error(&e);
                return Value::Nothing;
            }
            Outcome::Terminated | Outcome::Suspended(_) | Outcome::OutOfTime => {
                return Value::Nothing;
            }
        };
        self.locals = std::mem::take(&mut script.final_locals)
            .into_iter()
            .filter(|(name, _)| *name != Sym::THIS && *name != this_fsm)
            .collect();
        value
    }
}

impl<H: Host> Driver for ScriptDriver<'_, H> {
    fn action(&mut self, action: &Action) {
        if let Action::Script(text) = action {
            self.run(text);
        }
    }

    fn precondition(&mut self, code: &str) {
        self.run(code);
    }

    fn condition(&mut self, condition: &Condition) -> f32 {
        match condition {
            Condition::Script(text) => match self.run(text) {
                Value::Bool(true) => 1.0,
                _ => 0.0,
            },
            Condition::Native(_) => 0.0,
        }
    }

    fn random(&mut self) -> f32 {
        self.state.rng.next_f32()
    }
}
