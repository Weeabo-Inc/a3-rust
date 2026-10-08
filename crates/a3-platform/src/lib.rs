//! Platform layer: the main window, the main loop and raw input collection.
//!
//! [`run`] opens a winit window and drives an [`App`] once per frame with a [`FrameTime`] from
//! the [`FrameClock`] (see `docs/adr/0002-main-loop.md`). Keyboard, mouse and gamepad (gilrs)
//! events are translated into an [`a3_input::InputState`] the app reads through [`Context`].
//!
//! The clock and the key translation tables are usable without a window, e.g. for headless runs.

pub mod clock;
pub mod keymap;
mod runner;

pub use clock::{ClockConfig, FrameClock, FrameTime};
pub use runner::{App, Context, PlatformError, WindowConfig, run};

/// Re-export so apps can name window types without depending on winit directly.
pub use winit;
