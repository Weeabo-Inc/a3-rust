//! A running FSM: one [`Machine`] per instance, stepped once per simulation step against a
//! [`Driver`] that evaluates the conditions and runs the actions.
//!
//! The order a step runs things in (`docs/re/ai-fsm.md` §3):
//!
//! - Entering a state runs its `init` (a native init also draws its thresholds). The machine
//!   then suspends until the next step — except right after the init state, whose links are
//!   checked in the same step.
//! - Resuming in a state runs its `precondition` once (scripted only), then checks the links.
//! - Links are checked from the highest priority down; for each, its `precondition` runs, then
//!   its condition. The first that holds runs its `action`, and the machine enters the link's
//!   state. At most one link is taken per step.
//! - When no link holds, the machine stays and checks the links again next step.
//! - Entering a final state runs its `init` and ends the machine.

use crate::model::{Action, Condition, Fsm, StateId};

/// What a machine needs from whoever runs it.
pub trait Driver {
    /// Runs a state's `init` or a link's `action`.
    fn action(&mut self, action: &Action);
    /// Runs a state's or a link's `precondition` (scripted FSMs only; empty for native ones).
    fn precondition(&mut self, code: &str);
    /// The value of a condition: `1.0`/`0.0` for a scripted one, the function's value for a
    /// native one (before any `1-` inversion, which the machine applies).
    fn condition(&mut self, condition: &Condition) -> f32;
    /// A uniform random number in `0..1`, for the thresholds a native action draws.
    fn random(&mut self) -> f32;
}

/// Where a machine is in its current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Not started: the init state is entered on the first step.
    New,
    /// Entered; the state's precondition runs when the machine next resumes.
    Entered,
    /// Checking the links each step.
    Waiting,
    /// A final state was entered.
    Finished,
}

/// One running instance of an [`Fsm`].
#[derive(Debug, Clone, PartialEq)]
pub struct Machine {
    state: StateId,
    phase: Phase,
    thresholds: Vec<f32>,
}

impl Machine {
    /// A machine about to enter `fsm`'s init state on its first [`step`](Self::step).
    pub fn new(fsm: &Fsm) -> Self {
        Machine {
            state: fsm.init_state,
            phase: Phase::New,
            thresholds: vec![0.0; fsm.threshold_count()],
        }
    }

    /// The state the machine is in.
    pub fn state(&self) -> StateId {
        self.state
    }

    /// Whether the machine has entered a final state.
    pub fn is_finished(&self) -> bool {
        self.phase == Phase::Finished
    }

    /// The thresholds native conditions compare with.
    pub fn thresholds(&self) -> &[f32] {
        &self.thresholds
    }

    /// One simulation step. Returns whether the machine is still running afterwards.
    pub fn step(&mut self, fsm: &Fsm, driver: &mut impl Driver) -> bool {
        match self.phase {
            Phase::Finished => return false,
            Phase::New => {
                self.enter(fsm, fsm.init_state, driver);
                if self.phase == Phase::Finished {
                    return false;
                }
                // The init state does not suspend between its init and its precondition.
                driver.precondition(&fsm.state(self.state).precondition);
                self.phase = Phase::Waiting;
            }
            Phase::Entered => {
                driver.precondition(&fsm.state(self.state).precondition);
                self.phase = Phase::Waiting;
            }
            Phase::Waiting => {}
        }
        self.check_links(fsm, driver);
        !self.is_finished()
    }

    fn check_links(&mut self, fsm: &Fsm, driver: &mut impl Driver) {
        for link in &fsm.state(self.state).links {
            driver.precondition(&link.precondition);
            if self.holds(&link.condition, driver) {
                driver.action(&link.action);
                self.draw(&link.action, driver);
                self.enter(fsm, link.to, driver);
                return;
            }
        }
    }

    fn holds(&self, condition: &Condition, driver: &mut impl Driver) -> bool {
        let value = driver.condition(condition);
        match condition {
            Condition::Script(_) => value != 0.0,
            Condition::Native(native) => {
                let value = if native.inverted { 1.0 - value } else { value };
                let threshold = self
                    .thresholds
                    .get(native.threshold)
                    .copied()
                    .unwrap_or(0.0);
                value > threshold
            }
        }
    }

    fn enter(&mut self, fsm: &Fsm, state: StateId, driver: &mut impl Driver) {
        self.state = state;
        let init = &fsm.state(state).init;
        driver.action(init);
        self.draw(init, driver);
        self.phase = if fsm.state(state).is_final {
            Phase::Finished
        } else {
            Phase::Entered
        };
    }

    /// Draws the thresholds a native action sets, uniformly in `min..max`.
    fn draw(&mut self, action: &Action, driver: &mut impl Driver) {
        let Action::Native(native) = action else {
            return;
        };
        for draw in &native.thresholds {
            if let Some(slot) = self.thresholds.get_mut(draw.index) {
                *slot = draw.min + (draw.max - draw.min) * driver.random();
            }
        }
    }
}
