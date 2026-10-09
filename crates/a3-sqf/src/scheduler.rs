//! The scheduled environment: script instances started by `spawn` and
//! `execVM`, run cooperatively a slice per frame.
//!
//! Each frame the engine gives the scheduler a time budget (3 ms in the
//! original, see [`DEFAULT_FRAME_BUDGET`]). Scripts run in start order until
//! they suspend (`sleep`, `uiSleep`, `waitUntil`), finish, or the budget runs
//! out. A script cut off by the budget continues where it stopped and runs
//! first in the next frame. Scheduled code has no `while` iteration cap.

use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::code::Code;
use crate::error::ScriptError;
use crate::host::Host;
use crate::registry::Registry;
use crate::value::{Namespace, ScriptHandle, Value};
use crate::vm::exec::{self, Outcome, ScriptState};
use crate::vm::{Suspend, Vm, VmState};

/// The scheduler's per-frame budget in the original engine.
///
/// The community wiki's "Scheduler" page gives 3 ms in total per frame
/// (50 ms on a loading screen); not yet located in the binary.
pub const DEFAULT_FRAME_BUDGET: Duration = Duration::from_millis(3);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Wake {
    Ready,
    /// Mission time (`sleep`).
    AtTime(f32),
    /// Real time (`uiSleep`).
    AtTick(f32),
    /// Scheduler frame number (`waitUntil`).
    AtFrame(u64),
}

struct Scheduled<H: Host> {
    state: ScriptState<H>,
    wake: Wake,
}

/// The set of scheduled scripts.
pub struct Scheduler<H: Host> {
    scripts: Vec<Scheduled<H>>,
    /// Scripts spawned while the scheduler is running a frame.
    spawned: Vec<Scheduled<H>>,
    running: Option<ScriptHandle>,
    /// Every script not finished yet, wherever it sits during a frame (`scriptDone`).
    live: HashSet<ScriptHandle>,
    terminate: HashSet<ScriptHandle>,
    next_id: u32,
    frame: u64,
}

impl<H: Host> Default for Scheduler<H> {
    fn default() -> Self {
        Scheduler {
            scripts: Vec::new(),
            spawned: Vec::new(),
            running: None,
            live: HashSet::new(),
            terminate: HashSet::new(),
            next_id: 1,
            frame: 0,
        }
    }
}

/// What one scheduler frame did.
#[derive(Debug, Default)]
pub struct FrameReport {
    /// Scripts that got to run.
    pub ran: usize,
    /// Scripts that finished (normally, by error, or terminated).
    pub finished: usize,
    /// Errors raised by scheduled scripts (also sent to the host).
    pub errors: Vec<ScriptError>,
    /// Whether the budget ran out before every ready script ran.
    pub out_of_time: bool,
}

impl<H: Host> Scheduler<H> {
    /// Adds a script; it starts running at the next frame.
    pub fn spawn(&mut self, code: Code, this: Value, name: Option<Rc<str>>) -> ScriptHandle {
        let handle = ScriptHandle(self.next_id);
        self.next_id += 1;
        let mut state = ScriptState::new(code, Some(this), true, handle, Namespace::Mission);
        state.name = name;
        self.live.insert(handle);
        self.spawned.push(Scheduled {
            state,
            wake: Wake::Ready,
        });
        handle
    }

    /// Whether the script has finished (`scriptDone`). `scriptNull` counts
    /// as done.
    pub fn is_done(&self, handle: ScriptHandle) -> bool {
        handle.0 == 0 || !self.live.contains(&handle)
    }

    /// Requests that a script stop (`terminate`). It is removed before it
    /// runs again.
    pub fn terminate(&mut self, handle: ScriptHandle) {
        self.terminate.insert(handle);
    }

    /// Number of live scripts.
    pub fn len(&self) -> usize {
        self.scripts.len() + self.spawned.len() + usize::from(self.running.is_some())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Handles and names of the live scripts (not counting the running
    /// one).
    pub fn scripts_info(&self) -> Vec<(ScriptHandle, Option<Rc<str>>)> {
        self.scripts
            .iter()
            .chain(self.spawned.iter())
            .map(|s| (s.state.handle, s.state.name.clone()))
            .collect()
    }

    /// The handle of the script running now, if any.
    pub fn running(&self) -> Option<ScriptHandle> {
        self.running
    }

    fn apply_terminations(&mut self) -> usize {
        if self.terminate.is_empty() {
            return 0;
        }
        let before = self.scripts.len() + self.spawned.len();
        let term = std::mem::take(&mut self.terminate);
        self.scripts.retain(|s| !term.contains(&s.state.handle));
        self.spawned.retain(|s| !term.contains(&s.state.handle));
        self.live.retain(|h| !term.contains(h));
        before - self.scripts.len() - self.spawned.len()
    }
}

impl<H: Host> Vm<H> {
    /// Starts `code` as a scheduled script with `_this = this` (`spawn`).
    pub fn spawn(&mut self, code: &Code, this: Value) -> ScriptHandle {
        self.state.scheduler.spawn(code.clone(), this, None)
    }

    /// Whether a scheduled script has finished.
    pub fn script_done(&self, handle: ScriptHandle) -> bool {
        self.state.scheduler.is_done(handle)
    }

    /// Number of live scheduled scripts.
    pub fn scheduled_count(&self) -> usize {
        self.state.scheduler.len()
    }

    /// Starts a scripted FSM with `_this = this` (`execFSM`); it takes its first step at the
    /// next scheduler frame. Returns its handle.
    pub fn exec_fsm(&mut self, fsm: Rc<crate::fsm::CompiledFsm>, this: Value, name: &str) -> u32 {
        self.state.fsms.start(fsm, this, name)
    }

    /// Whether a scripted FSM has ended (`completedFSM`).
    pub fn fsm_completed(&self, handle: u32) -> bool {
        self.state.fsms.is_completed(handle)
    }

    /// A variable of a running scripted FSM (`getFSMVariable`).
    pub fn fsm_variable(&self, handle: u32, name: &str) -> Option<Value> {
        self.state.fsms.variable(handle, name)
    }

    /// Runs one frame of the scheduled environment with the given time
    /// budget.
    pub fn run_scheduled(&mut self, budget: Duration) -> FrameReport {
        let Vm {
            host,
            registry,
            state,
        } = self;
        run_frame(host, registry, state, Instant::now() + budget)
    }

    /// Runs scheduler frames until no script is left or `max_frames`
    /// frames have run, calling `between` after each frame (to advance the
    /// host's clock). Returns the number of frames run.
    pub fn run_until_idle(&mut self, max_frames: usize, mut between: impl FnMut(&mut H)) -> usize {
        for i in 0..max_frames {
            if self.state.scheduler.is_empty() {
                return i;
            }
            self.run_scheduled(Duration::from_secs(3600));
            between(&mut self.host);
        }
        max_frames
    }
}

fn run_frame<H: Host>(
    host: &mut H,
    reg: &Registry<H>,
    state: &mut VmState<H>,
    deadline: Instant,
) -> FrameReport {
    let mut report = FrameReport::default();
    {
        let s = &mut state.scheduler;
        report.finished += s.apply_terminations();
        let mut spawned = std::mem::take(&mut s.spawned);
        s.scripts.append(&mut spawned);
        s.frame += 1;
    }
    let frame = state.scheduler.frame;
    let mut list = std::mem::take(&mut state.scheduler.scripts);
    let mut kept: Vec<Scheduled<H>> = Vec::with_capacity(list.len());
    let mut iter = list.drain(..);
    while let Some(mut script) = iter.next() {
        if state.scheduler.terminate.remove(&script.state.handle) {
            state.scheduler.live.remove(&script.state.handle);
            report.finished += 1;
            continue;
        }
        let ready = match script.wake {
            Wake::Ready => true,
            Wake::AtTime(t) => host.time() >= t,
            Wake::AtTick(t) => host.tick_time() >= t,
            Wake::AtFrame(f) => frame >= f,
        };
        if !ready {
            kept.push(script);
            continue;
        }
        if Instant::now() >= deadline {
            report.out_of_time = true;
            kept.push(script);
            kept.extend(iter.by_ref());
            break;
        }
        report.ran += 1;
        state.scheduler.running = Some(script.state.handle);
        let outcome = exec::run(host, reg, state, &mut script.state, Some(deadline));
        state.scheduler.running = None;
        if !matches!(outcome, Outcome::Suspended(_) | Outcome::OutOfTime) {
            state.scheduler.live.remove(&script.state.handle);
        }
        match outcome {
            Outcome::Done(_) | Outcome::Terminated => report.finished += 1,
            Outcome::Failed(e) => {
                host.report_error(&e);
                report.errors.push(e);
                report.finished += 1;
            }
            Outcome::Suspended(s) => {
                script.wake = match s {
                    Suspend::Sleep(d) => Wake::AtTime(host.time() + d.max(0.0)),
                    Suspend::UiSleep(d) => Wake::AtTick(host.tick_time() + d.max(0.0)),
                    Suspend::NextFrame => Wake::AtFrame(frame + 1),
                };
                kept.push(script);
            }
            Outcome::OutOfTime => {
                report.out_of_time = true;
                script.wake = Wake::Ready;
                // Continue first next frame, then the ones that did not run.
                let rest: Vec<_> = iter.by_ref().collect();
                let mut front = vec![script];
                front.extend(rest);
                front.extend(kept);
                kept = front;
                break;
            }
        }
    }
    drop(iter);
    let s = &mut state.scheduler;
    s.scripts = kept;
    report.finished += s.apply_terminations();
    // Each scripted FSM takes one step per frame (`docs/re/ai-fsm.md` §3).
    crate::fsm::step_all(host, reg, state);
    report
}
