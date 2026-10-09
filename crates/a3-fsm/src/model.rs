//! What an FSM is made of, once loaded: states, their links, and the code or native calls on
//! each. Both flavours (`docs/re/ai-fsm.md`) share the graph; they differ in what a condition and
//! an action are.

/// Index of a state in [`Fsm::states`].
pub type StateId = usize;

/// Which language an FSM's conditions and actions are written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsmKind {
    /// A `.fsm` file: every condition, action and state init is SQF text.
    Scripted,
    /// A `CfgFSMs` class: every condition and action names a function of the engine.
    Native,
}

/// One state machine definition, shared by every machine that runs it.
#[derive(Debug, Clone, PartialEq)]
pub struct Fsm {
    /// `fsmName` of a scripted FSM, the `CfgFSMs` class name of a native one.
    pub name: String,
    pub kind: FsmKind,
    /// In file order.
    pub states: Vec<State>,
    /// `initState`: where a new machine starts.
    pub init_state: StateId,
}

/// One state: what runs when a machine enters it and the links that lead out of it.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// The state's class name, which `initState`, `finalStates[]` and a link's `to` name.
    pub class_name: String,
    /// `name` (the editor's label; the same as the class name in every shipped file).
    pub name: String,
    /// `init` (scripted) / `class Init` (native): runs once on entering the state.
    pub init: Action,
    /// `precondition` (scripted only): runs once after `init`, when the machine next resumes.
    pub precondition: String,
    /// The links, highest priority first; links of equal priority keep their file order.
    pub links: Vec<Link>,
    /// Listed in `finalStates[]`: entering it ends the machine.
    pub is_final: bool,
}

/// A link (transition): when its condition holds, its action runs and the machine moves on.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    /// The link's class name.
    pub name: String,
    /// `priority`: links are checked from the highest down.
    pub priority: f32,
    /// The state the link leads to.
    pub to: StateId,
    /// `precondition` (scripted only): runs before the condition is checked.
    pub precondition: String,
    pub condition: Condition,
    /// Runs when the link is taken, before the next state is entered.
    pub action: Action,
}

/// What runs on entering a state or taking a link.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// SQF text (a scripted FSM, or a native action written `script:<code>`). Empty does nothing.
    Script(String),
    /// A function of the engine, with its arguments and the thresholds it draws.
    Native(NativeAction),
}

impl Action {
    /// Whether running the action can do nothing at all: empty code, or the native `nothing`
    /// with no thresholds to draw.
    pub fn is_empty(&self) -> bool {
        match self {
            Action::Script(code) => code.trim().is_empty(),
            Action::Native(native) => {
                native.function.eq_ignore_ascii_case("nothing") && native.thresholds.is_empty()
            }
        }
    }
}

/// A native action: `class Init { function; parameters[]; thresholds[]; }`.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeAction {
    /// The engine function, by name (`formationInit`, `searchPath`, `wait`, ...).
    pub function: String,
    /// `parameters[]`: the function's arguments.
    pub parameters: Vec<f32>,
    /// `thresholds[]`: the random thresholds drawn when the action runs.
    pub thresholds: Vec<ThresholdDraw>,
}

/// One `thresholds[]` item, `{index, min, max}`: when the action runs, threshold `index` is
/// drawn uniformly from `min..max` for the conditions of the state that follows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThresholdDraw {
    pub index: usize,
    pub min: f32,
    pub max: f32,
}

/// When a link may be taken.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// SQF text that returns a Boolean.
    Script(String),
    /// A function of the engine that returns a value, compared with a threshold.
    Native(NativeCondition),
}

/// A native condition: `class Condition { function; parameters[]; threshold; }`.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeCondition {
    /// The engine function, by name (`true`, `const`, `behaviourCombat`, ...), without a `1-`
    /// prefix.
    pub function: String,
    /// The function was written `1-name`: the condition's value is one minus the function's.
    pub inverted: bool,
    /// `parameters[]`: the function's arguments.
    pub parameters: Vec<f32>,
    /// `threshold`: which of the machine's thresholds the value is compared with.
    pub threshold: usize,
}

impl Fsm {
    /// The state with this class name (ASCII case-insensitive), as `initState`, `finalStates[]`
    /// and `to` name states.
    pub fn state_named(&self, name: &str) -> Option<StateId> {
        self.states
            .iter()
            .position(|s| s.class_name.eq_ignore_ascii_case(name))
    }

    /// The state at `id`.
    pub fn state(&self, id: StateId) -> &State {
        &self.states[id]
    }

    /// The highest threshold index any action draws or any condition reads, plus one: how many
    /// thresholds a machine running this FSM keeps.
    pub fn threshold_count(&self) -> usize {
        let draws = self.states.iter().flat_map(|s| {
            std::iter::once(&s.init)
                .chain(s.links.iter().map(|l| &l.action))
                .filter_map(|a| match a {
                    Action::Native(n) => Some(n.thresholds.iter().map(|t| t.index + 1)),
                    Action::Script(_) => None,
                })
                .flatten()
        });
        let reads = self.states.iter().flat_map(|s| {
            s.links.iter().filter_map(|l| match &l.condition {
                Condition::Native(c) => Some(c.threshold + 1),
                Condition::Script(_) => None,
            })
        });
        draws.chain(reads).max().unwrap_or(0)
    }
}
