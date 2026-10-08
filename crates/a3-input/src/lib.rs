//! Device-independent input for the engine.
//!
//! The platform layer feeds raw events (keys by DirectInput scancode, mouse buttons and motion,
//! gamepad buttons and axes) into an [`InputState`]. An [`ActionMap`] turns that state into
//! named user actions (`moveForward`, `defaultAction`, `cameraMoveUp`, ...) the way Real
//! Virtuality's `CfgDefaultKeysPresets` and profile keybindings do: each action owns a list of
//! [`Binding`]s, and a binding is an input plus an optional modifier (`LCtrl+X`) and a
//! [`Trigger`] (press, double tap, hold).
//!
//! Bindings are plain data. [`Binding::from_rv_code`] decodes the integer key codes used by the
//! game's config and profiles (see `docs/re/input-keys.md`).

mod action;
mod binding;
mod code;
mod rv_code;
mod state;

pub mod actions;

pub use action::{ActionMap, ActionName};
pub use binding::{Binding, Modifier, Trigger};
pub use code::{Dik, GamepadInput, InputCode, MouseAxis};
pub use rv_code::RvCodeError;
pub use state::{InputState, InputTiming};
