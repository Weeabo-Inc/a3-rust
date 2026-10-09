//! Loading an FSM from config: a `.fsm` file (scripted) or a `CfgFSMs` class (native).
//!
//! Both are config syntax with the same shape — `class States { class S { class Links { ... } } }`,
//! `initState`, `finalStates[]` — and are read by one loader over [`ConfigRef`]. The engine's
//! loader (`FSMEntityType`, `docs/re/ai-fsm.md` §2) reports a bad init state or thresholds entry
//! and carries on; so does this one, through [`Loaded::warnings`].

use a3_config::{Config, ConfigRef, ConfigTree, Value, parse_text};

use crate::model::{
    Action, Condition, Fsm, FsmKind, Link, NativeAction, NativeCondition, State, StateId,
    ThresholdDraw,
};

/// The prefix that makes a native action SQF instead of an engine function.
pub const SCRIPT_PREFIX: &str = "script:";

/// Why an FSM could not be loaded at all.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum LoadError {
    #[error("FSM text does not parse: {0}")]
    Parse(#[from] a3_config::ParseError),
    #[error("FSM has no class FSM")]
    NoFsmClass,
    #[error("FSM {0:?} has no states")]
    NoStates(String),
}

/// A loaded FSM and what the loader complained about on the way.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub fsm: Fsm,
    /// Problems the engine reports and tolerates: an unknown `initState` (the first state is
    /// used), a link to an unknown state (the link is dropped), a malformed `thresholds[]`
    /// item (skipped).
    pub warnings: Vec<String>,
}

impl Fsm {
    /// A scripted FSM from the text of a `.fsm` file. Comments (the FSM Editor's layout data)
    /// are skipped; the text must not need the preprocessor otherwise.
    pub fn parse_scripted(text: &str) -> Result<Loaded, LoadError> {
        let config = parse_text(text)?;
        Self::from_scripted_config(&config)
    }

    /// A scripted FSM from the parsed config of a `.fsm` file (its root holds `class FSM`).
    pub fn from_scripted_config(config: &Config) -> Result<Loaded, LoadError> {
        let tree = ConfigTree::from_config(config);
        let class = tree.root().get("FSM");
        if !class.is_class() {
            return Err(LoadError::NoFsmClass);
        }
        let name = class.get("fsmName").text();
        load(&class, name, FsmKind::Scripted)
    }

    /// A native FSM from its `CfgFSMs` class.
    pub fn from_native_config(class: &ConfigRef<'_>) -> Result<Loaded, LoadError> {
        load(class, class.name().to_owned(), FsmKind::Native)
    }
}

fn load(class: &ConfigRef<'_>, name: String, kind: FsmKind) -> Result<Loaded, LoadError> {
    let mut warnings = Vec::new();
    let classes: Vec<ConfigRef<'_>> = class
        .get("States")
        .entries()
        .into_iter()
        .filter(ConfigRef::is_class)
        .collect();
    if classes.is_empty() {
        return Err(LoadError::NoStates(name));
    }
    let names: Vec<String> = classes.iter().map(|c| c.name().to_owned()).collect();
    let find = |wanted: &str| -> Option<StateId> {
        names.iter().position(|n| n.eq_ignore_ascii_case(wanted))
    };

    let init_name = class.get("initState").text();
    let init_state = find(&init_name).unwrap_or_else(|| {
        warnings.push(format!("FSM {name:?}: wrong init state {init_name:?}"));
        0
    });
    let finals: Vec<StateId> = class
        .get("finalStates")
        .array()
        .iter()
        .filter_map(|v| match v {
            Value::String(s) => find(s),
            _ => None,
        })
        .collect();

    let mut states = Vec::with_capacity(classes.len());
    for (id, state) in classes.iter().enumerate() {
        let is_final = finals.contains(&id);
        let (init, precondition) = match kind {
            FsmKind::Scripted => (
                Action::Script(state.get("init").text()),
                state.get("precondition").text(),
            ),
            FsmKind::Native => (
                native_action(&state.get("Init"), &name, &mut warnings),
                String::new(),
            ),
        };
        // A final state has no way out: the engine does not load its links.
        let links = if is_final {
            Vec::new()
        } else {
            links(state, kind, &name, &find, &mut warnings)
        };
        states.push(State {
            class_name: state.name().to_owned(),
            name: state.get("name").text(),
            init,
            precondition,
            links,
            is_final,
        });
    }
    Ok(Loaded {
        fsm: Fsm {
            name,
            kind,
            states,
            init_state,
        },
        warnings,
    })
}

fn links(
    state: &ConfigRef<'_>,
    kind: FsmKind,
    fsm: &str,
    find: &impl Fn(&str) -> Option<StateId>,
    warnings: &mut Vec<String>,
) -> Vec<Link> {
    let mut links = Vec::new();
    for link in state.get("Links").entries() {
        if !link.is_class() {
            continue;
        }
        let to_name = link.get("to").text();
        let Some(to) = find(&to_name) else {
            warnings.push(format!(
                "FSM {fsm:?}: link {:?} of state {:?} leads to unknown state {to_name:?}",
                link.name(),
                state.name()
            ));
            continue;
        };
        let (condition, action, precondition) = match kind {
            FsmKind::Scripted => (
                Condition::Script(link.get("condition").text()),
                Action::Script(link.get("action").text()),
                link.get("precondition").text(),
            ),
            FsmKind::Native => (
                native_condition(&link.get("Condition")),
                native_action(&link.get("Action"), fsm, warnings),
                String::new(),
            ),
        };
        links.push(Link {
            name: link.name().to_owned(),
            priority: link.get("priority").number(),
            to,
            precondition,
            condition,
            action,
        });
    }
    // Highest priority first; a stable sort keeps file order among equals.
    links.sort_by(|a, b| b.priority.total_cmp(&a.priority));
    links
}

fn native_action(class: &ConfigRef<'_>, fsm: &str, warnings: &mut Vec<String>) -> Action {
    let function = class.get("function").text();
    if function.len() >= SCRIPT_PREFIX.len()
        && function[..SCRIPT_PREFIX.len()].eq_ignore_ascii_case(SCRIPT_PREFIX)
    {
        return Action::Script(function[SCRIPT_PREFIX.len()..].to_owned());
    }
    let function = if function.is_empty() {
        "nothing".to_owned()
    } else {
        function
    };
    let mut thresholds = Vec::new();
    for item in class.get("thresholds").array() {
        match item {
            Value::Array(parts) if parts.len() == 3 => {
                let index = number(&parts[0]).max(0.0) as usize;
                thresholds.push(ThresholdDraw {
                    index,
                    min: number(&parts[1]),
                    max: number(&parts[2]),
                });
            }
            _ => warnings.push(format!("FSM {fsm:?}: wrong thresholds structure")),
        }
    }
    Action::Native(NativeAction {
        function,
        parameters: numbers(&class.get("parameters")),
        thresholds,
    })
}

fn native_condition(class: &ConfigRef<'_>) -> Condition {
    let written = class.get("function").text();
    let (function, inverted) = match written.strip_prefix("1-") {
        Some(rest) => (rest.trim().to_owned(), true),
        None => (written.trim().to_owned(), false),
    };
    Condition::Native(NativeCondition {
        function,
        inverted,
        parameters: numbers(&class.get("parameters")),
        threshold: class.get("threshold").number().max(0.0) as usize,
    })
}

fn numbers(entry: &ConfigRef<'_>) -> Vec<f32> {
    entry.array().iter().map(number).collect()
}

fn number(value: &Value) -> f32 {
    match value {
        Value::Float(f) => *f,
        Value::Int(i) => *i as f32,
        Value::Int64(i) => *i as f32,
        Value::String(s) | Value::Expression(s) => s.trim().parse().unwrap_or(0.0),
        Value::Array(_) => 0.0,
    }
}
