//! A running FSM: one [`Machine`] per instance, stepped by its owner (the scheduler for a
//! mission FSM, the unit's think for an AI FSM) against a [`Driver`] that evaluates the
//! conditions and runs the actions. `docs/re/ai-fsm.md` §1.5 (native) and §4.3 (scripted).
//!
//! **Scripted** (`FSMScripted::Step`), one step:
//! 1. the first step runs the init state's `init`;
//! 2. the current state's `precondition` runs;
//! 3. the links are checked from the highest priority down — each link's `precondition`, then
//!    its condition (empty holds); the first that holds runs its `action`, the machine moves to
//!    its state and runs that state's `init` (also when the link leads back to the same state);
//! 4. a final state ends the machine one step after it is entered.
//!
//! **Native** (`FSMEntity::Update`), one step:
//! 1. the first step runs the init state's `Init`;
//! 2. the links are checked from the highest priority down: a link holds when
//!    `thresholds[slot] <= value` (`1-name` gives `1 - value`); the first that holds runs its
//!    action (enter, then exit), the old state's `Init` exit runs, and the new state's `Init`
//!    runs — at most one such transition per step;
//! 3. then, in zero time, the machine follows every state whose only link is a plain `true`
//!    that holds, until it comes back to where this chain started or has passed as many states
//!    as the FSM has;
//! 4. a final state's only link is such a `true`, which ends the machine.
//!
//! Running an action draws its threshold slots first (`min + (max - min) * random`).

use crate::model::{Action, Condition, Fsm, FsmKind, Link, StateId, THRESHOLD_START};

/// What a machine needs from whoever runs it.
pub trait Driver {
    /// Runs a state's `init` or a link's `action` (for a native function: its *enter* half).
    fn action(&mut self, action: &Action);
    /// The *exit* half of a native action: when a state is left (its `Init`), and right after
    /// a link's action. Most engine functions have an empty exit.
    fn exit(&mut self, _action: &Action) {}
    /// Runs a state's or a link's `precondition` (scripted FSMs only). Not called for empty
    /// code.
    fn precondition(&mut self, code: &str);
    /// The value of a condition: `1.0` for a scripted condition that is true and `0.0`
    /// otherwise; for a native one the function's value (before any `1-`, which the machine
    /// applies) or the number a `script:` condition returns.
    fn condition(&mut self, condition: &Condition) -> f32;
    /// A uniform random number in `0..1`, for the thresholds a native action draws.
    fn random(&mut self) -> f32;
}

/// One running instance of an [`Fsm`].
#[derive(Debug, Clone, PartialEq)]
pub struct Machine {
    /// The current state; `None` once the machine has ended.
    state: Option<StateId>,
    /// The init state's `init` has not run yet.
    init_needed: bool,
    thresholds: Vec<f32>,
}

impl Machine {
    /// A machine in `fsm`'s init state, whose `init` runs on the first [`step`](Self::step).
    pub fn new(fsm: &Fsm) -> Self {
        Machine {
            state: Some(fsm.init_state),
            init_needed: true,
            thresholds: vec![THRESHOLD_START; fsm.threshold_count()],
        }
    }

    /// The state the machine is in; `None` once it has ended.
    pub fn state(&self) -> Option<StateId> {
        self.state
    }

    /// Whether the machine has ended (left a final state).
    pub fn is_finished(&self) -> bool {
        self.state.is_none()
    }

    /// The threshold slots native conditions compare with.
    pub fn thresholds(&self) -> &[f32] {
        &self.thresholds
    }

    /// One step. Returns whether the machine is still running afterwards.
    pub fn step(&mut self, fsm: &Fsm, driver: &mut impl Driver) -> bool {
        match fsm.kind {
            FsmKind::Scripted => self.step_scripted(fsm, driver),
            FsmKind::Native => self.step_native(fsm, driver),
        }
        !self.is_finished()
    }

    fn step_scripted(&mut self, fsm: &Fsm, driver: &mut impl Driver) {
        let Some(current) = self.state else {
            return;
        };
        let state = fsm.state(current);
        if std::mem::take(&mut self.init_needed) {
            driver.action(&state.init);
        }
        if !state.precondition.is_empty() {
            driver.precondition(&state.precondition);
        }
        if state.is_final {
            // The final state's own link: no condition, no action, out of the machine.
            self.state = None;
            return;
        }
        for link in &state.links {
            if !link.precondition.is_empty() {
                driver.precondition(&link.precondition);
            }
            let holds = match &link.condition {
                Condition::Script(code) if code.trim().is_empty() => true,
                condition => driver.condition(condition) != 0.0,
            };
            if holds {
                driver.action(&link.action);
                let Some(next) = link.to else {
                    return;
                };
                self.state = Some(next);
                driver.action(&fsm.state(next).init);
                return;
            }
        }
    }

    fn step_native(&mut self, fsm: &Fsm, driver: &mut impl Driver) {
        let Some(current) = self.state else {
            return;
        };
        if std::mem::take(&mut self.init_needed) {
            self.execute(&fsm.state(current).init, driver);
        }
        let state = fsm.state(current);
        let mut taken = None;
        if state.is_final {
            if self.final_link_holds() {
                taken = Some(None);
            }
        } else {
            for link in &state.links {
                if self.native_holds(link, driver) {
                    taken = Some(link.to);
                    self.execute(&link.action, driver);
                    driver.exit(&link.action);
                    break;
                }
            }
        }
        let Some(to) = taken else {
            return;
        };
        if state.is_final {
            self.set_state(fsm, None, driver);
            return;
        }
        let Some(to) = to else {
            // A link to no state: its action ran, the machine stays.
            return;
        };
        if self.set_state(fsm, Some(to), driver) {
            return;
        }
        self.follow_chain(fsm, driver);
    }

    /// The zero-time chain after a transition: through states whose only link is a plain
    /// `true` that holds.
    fn follow_chain(&mut self, fsm: &Fsm, driver: &mut impl Driver) {
        let Some(first) = self.state else {
            return;
        };
        let mut budget = fsm.states.len() as isize;
        loop {
            let Some(current) = self.state else {
                return;
            };
            let state = fsm.state(current);
            let to = if state.is_final {
                if !self.final_link_holds() {
                    return;
                }
                None
            } else {
                let [link] = state.links.as_slice() else {
                    return;
                };
                let Condition::Native(condition) = &link.condition else {
                    return;
                };
                if !condition.is_plain_true() || self.slot(condition.threshold) > 1.0 {
                    return;
                }
                self.execute(&link.action, driver);
                driver.exit(&link.action);
                match link.to {
                    Some(to) => Some(to),
                    None => return,
                }
            };
            if self.set_state(fsm, to, driver) {
                return;
            }
            budget -= 1;
            if to == Some(first) || budget < 0 {
                return;
            }
        }
    }

    /// Leaves the current state (its `Init` exit) for `to`, running `to`'s `Init`. Returns
    /// whether the machine ended.
    fn set_state(&mut self, fsm: &Fsm, to: Option<StateId>, driver: &mut impl Driver) -> bool {
        if let Some(current) = self.state {
            driver.exit(&fsm.state(current).init);
        }
        self.state = to;
        match to {
            None => true,
            Some(to) => {
                self.execute(&fsm.state(to).init, driver);
                false
            }
        }
    }

    /// Runs a native action: its threshold draws, then its function (enter half).
    fn execute(&mut self, action: &Action, driver: &mut impl Driver) {
        if let Action::Native(native) = action {
            for draw in &native.thresholds {
                if let Some(slot) = self.thresholds.get_mut(draw.index) {
                    *slot = draw.min + (draw.max - draw.min) * driver.random();
                }
            }
        }
        driver.action(action);
    }

    fn native_holds(&self, link: &Link, driver: &mut impl Driver) -> bool {
        let Condition::Native(condition) = &link.condition else {
            return false;
        };
        let value = driver.condition(&link.condition);
        let value = if condition.inverted && !condition.script {
            1.0 - value
        } else {
            value
        };
        self.slot(condition.threshold) <= value
    }

    /// The final state's own link: a plain `true` (value 1.0) on slot 0.
    fn final_link_holds(&self) -> bool {
        self.slot(0) <= 1.0
    }

    fn slot(&self, index: usize) -> f32 {
        self.thresholds
            .get(index)
            .copied()
            .unwrap_or(THRESHOLD_START)
    }
}
