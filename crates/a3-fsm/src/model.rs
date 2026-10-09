//! What an FSM is made of, once loaded: states, their links, and the code or native calls on
//! each. Both flavours (`docs/re/ai-fsm.md`) share the graph; they differ in what a condition and
//! an action are.

/// Index of a state in [`Fsm::states`].
pub type StateId = usize;

/// The value every native threshold slot starts at (`FSMEntity` constructor).
pub const THRESHOLD_START: f32 = 0.5;

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
    /// `init` (scripted) / `class Init` (native): runs on entering the state.
    pub init: Action,
    /// `precondition` (scripted only): runs at the start of every step spent in the state.
    pub precondition: String,
    /// The links, highest priority first (sorted as the engine sorts them, see
    /// [`crate::sort_links`]). Empty for a final state: the machine gives a final state one
    /// link of its own that ends it.
    pub links: Vec<Link>,
    /// Listed in `finalStates[]`.
    pub is_final: bool,
}

/// A link (transition): when its condition holds, its action runs and the machine moves on.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    /// The link's class name.
    pub name: String,
    /// `priority`: links are checked from the highest down.
    pub priority: f32,
    /// The state the link leads to. `None` when `to` names no state: a native link then runs
    /// its action and stays (the engine keeps such a link; a scripted one is dropped at load).
    pub to: Option<StateId>,
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
    /// The engine function, by name (`formationInit`, `searchPath`, ...). A name resolves to a
    /// pair: *enter* runs when the action runs; *exit* when the state is left (a state's
    /// `Init`) or straight after enter (a link's `Action`).
    pub function: String,
    /// `parameters[]`: the function's arguments.
    pub parameters: Vec<f32>,
    /// `thresholds[]`: the threshold slots drawn each time the action runs, before its function.
    pub thresholds: Vec<ThresholdDraw>,
}

/// One `thresholds[]` item, `{index, min, max}`: when the action runs, threshold slot `index` is
/// drawn uniformly from `min..max`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThresholdDraw {
    pub index: usize,
    pub min: f32,
    pub max: f32,
}

/// When a link may be taken.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// SQF text that returns a Boolean (a scripted FSM). Empty always holds.
    Script(String),
    /// A native condition: a value compared with a threshold slot.
    Native(NativeCondition),
}

/// A native condition: `class Condition { function; parameters[]; threshold; }`. It holds when
/// `thresholds[threshold] <= value`.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeCondition {
    /// The engine function, by name (`true`, `const`, `behaviourCombat`, ...), without its `1-`
    /// prefix; or, when [`script`](Self::script) is set, SQF text whose number is the value.
    pub function: String,
    /// The function was written `1-name`: the value is one minus the function's. Never set for
    /// a `script:` condition.
    pub inverted: bool,
    /// Written `script:<code>`: `function` is the code.
    pub script: bool,
    /// `parameters[]`: the function's arguments.
    pub parameters: Vec<f32>,
    /// `threshold`: the slot the value is compared with.
    pub threshold: usize,
}

impl NativeCondition {
    /// Whether this is the engine's `true` written plainly: what the zero-time chain of a native
    /// machine follows (`docs/re/ai-fsm.md` §1.5).
    pub fn is_plain_true(&self) -> bool {
        !self.script && !self.inverted && self.function.eq_ignore_ascii_case("true")
    }
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

    /// How many threshold slots a machine running this FSM keeps: one more than the highest
    /// slot any action draws or any condition reads, zero when nothing refers to one.
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

/// Sorts links by descending priority exactly as the engine does: the MSVC C runtime `qsort`
/// (insertion of the greatest into the end for 8 elements or fewer, median-swap partitioning
/// above), which is not stable, so links of equal priority come out in the engine's order
/// (`docs/re/ai-fsm.md` §1.4).
pub fn sort_links(links: &mut [Link]) {
    // The engine's comparator: `b.priority - a.priority`, positive when `a` sorts after `b`.
    let cmp = |a: &Link, b: &Link| -> i32 {
        let d = b.priority - a.priority;
        if d > 0.0 {
            1
        } else if d >= 0.0 {
            0
        } else {
            -1
        }
    };
    msvc_qsort(links, cmp);
}

const CUTOFF: usize = 8;

fn msvc_qsort<T>(v: &mut [T], cmp: impl Fn(&T, &T) -> i32) {
    if v.len() < 2 {
        return;
    }
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let (mut lo, mut hi) = (0usize, v.len() - 1);
    loop {
        let size = hi - lo + 1;
        if size <= CUTOFF {
            shortsort(v, lo, hi, &cmp);
        } else {
            let mid = lo + size / 2;
            v.swap(mid, lo);
            let mut loguy = lo;
            let mut higuy = hi + 1;
            loop {
                loop {
                    loguy += 1;
                    if !(loguy <= hi && cmp(&v[loguy], &v[lo]) <= 0) {
                        break;
                    }
                }
                loop {
                    higuy -= 1;
                    if !(higuy > lo && cmp(&v[higuy], &v[lo]) >= 0) {
                        break;
                    }
                }
                if higuy < loguy {
                    break;
                }
                v.swap(loguy, higuy);
            }
            v.swap(lo, higuy);
            // Recurse into the smaller part, push the larger.
            if higuy as isize - 1 - lo as isize >= hi as isize - loguy as isize {
                if lo + 1 < higuy {
                    stack.push((lo, higuy - 1));
                }
                if loguy < hi {
                    lo = loguy;
                    continue;
                }
            } else {
                if loguy < hi {
                    stack.push((loguy, hi));
                }
                if lo + 1 < higuy {
                    hi = higuy - 1;
                    continue;
                }
            }
        }
        match stack.pop() {
            Some((l, h)) => (lo, hi) = (l, h),
            None => return,
        }
    }
}

/// Selection of the greatest into the end: the first strictly greatest element of `lo..=hi`
/// is swapped to `hi`, then `hi` moves down.
fn shortsort<T>(v: &mut [T], lo: usize, mut hi: usize, cmp: &impl Fn(&T, &T) -> i32) {
    while hi > lo {
        let mut max = lo;
        for p in lo + 1..=hi {
            if cmp(&v[p], &v[max]) > 0 {
                max = p;
            }
        }
        v.swap(max, hi);
        hi -= 1;
    }
}
