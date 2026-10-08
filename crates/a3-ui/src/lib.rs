//! The config-driven user interface: displays and controls built from `RscDisplay*` / `Rsc*`
//! config classes, their layout in the engine's coordinate system, text and drawing, and the
//! SQF UI commands.
//!
//! - [`UiMetrics`]: the 4:3 viewport scaled by interface size, the safe zone and the pixel
//!   grid (`safeZone*`, `pixelW/H`, `pixelGrid*`, `getResolution`).
//! - [`Ui`]: the display and control arenas and the display stack. [`Ui::create_display`]
//!   builds a display from config, evaluating expression strings through an [`Eval`].
//! - [`Fonts`]: `CfgFontFamilies` and FXY fonts; text layout into glyph quads.
//! - [`draw`]: turns the open displays into [`DrawList`] quads in screen pixels for a renderer.
//! - [`commands`]: the SQF UI commands on a [`UiHost`].
//!
//! See `docs/re/ui.md` for the engine behaviour this follows.

pub mod commands;
pub mod draw;
pub mod kinds;
pub mod metrics;
pub mod model;
pub mod text;
pub mod ui;

pub use commands::{UiHost, register_ui_commands};
pub use draw::{DrawList, Quad, TextureKey, Uv, build_draw_list};
pub use kinds::{ControlType, style};
pub use metrics::{Screen, UiMetrics, ui_scale};
pub use model::{Control, ControlId, Display, DisplayId, EventHandler, ListItem, Rgba};
pub use text::{FontLoader, Fonts};
pub use ui::{Error, Eval, NoEval, Ui, read_color, read_number, read_text};
