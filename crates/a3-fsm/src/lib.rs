//! FSMs: the state machines that drive AI behaviour and mission logic.
//!
//! The engine has two flavours with one graph shape (`docs/re/ai-fsm.md`):
//!
//! - **Scripted** (`.fsm` files, `execFSM`, a unit's `fsmFormation`/`fsmDanger` given as a file
//!   path): every state init, link condition and link action is SQF.
//! - **Native** (`CfgFSMs` classes, e.g. `Formation`, the soldiers' formation FSM): every
//!   condition and action names an engine function with numeric parameters, and conditions
//!   compare their value with random thresholds the actions draw.
//!
//! [`Fsm`] is the loaded definition (shared); [`Machine`] is one running instance (per unit or
//! per script) and steps through the definition against a [`Driver`] that evaluates conditions
//! and runs actions — SQF through the VM for a scripted FSM, Rust functions for a native one.
//!
//! ```
//! use a3_fsm::Fsm;
//!
//! let text = r#"class FSM {
//!     fsmName = "Hello";
//!     class States {
//!         class Start { name = "Start"; init = "x = 1;";
//!             class Links { class Go { priority = 0; to = "End"; condition = "true"; action = ""; }; };
//!         };
//!         class End { name = "End"; init = ""; class Links {}; };
//!     };
//!     initState = "Start";
//!     finalStates[] = {"End"};
//! };"#;
//! let fsm = Fsm::parse_scripted(text).unwrap().fsm;
//! assert_eq!(fsm.name, "Hello");
//! assert!(fsm.state(fsm.state_named("End").unwrap()).is_final);
//! ```

mod load;
mod model;
mod run;

pub use run::{Driver, Machine};

pub use load::{LoadError, Loaded, SCRIPT_PREFIX};
pub use model::{
    Action, Condition, Fsm, FsmKind, Link, NativeAction, NativeCondition, State, StateId,
    THRESHOLD_START, ThresholdDraw, sort_links,
};
