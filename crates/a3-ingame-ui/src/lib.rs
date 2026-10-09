//! The in-game UI (the engine's `InGameUI`): the HUD displays of `RscInGameUI` and the
//! controls the engine drives in them every frame.
//!
//! - [`InGameUi`]: the HUD layer — its own [`a3_ui::Ui`] of in-game displays, a VM for their
//!   config expressions and handlers, and [`InGameUi::update`], which rebuilds the unit info
//!   displays the player's vehicle names (`unitInfoType`) and fills them from a [`UnitInfo`].
//! - [`unit_info`]: the [`UnitInfo`] snapshot and what each engine-driven IDC of a unit info
//!   display shows ([`idc`]).
//! - [`colors`]: the weapon-state colours (`RscInGameUI >> colorReady`, ...).
//! - [`stance`]: the stance indicator textures (`CfgStanceIndicatorTextures`).
//! - [`loadout`]: the [`UnitInfo`] of a unit as its config class spawns it.
//! - [`hint`]: `hint` / `hintSilent` in `RscHint` ([`InGameUi::show_hint`]).
//! - [`chat`]: the chat area (`RscChatListMission`), `systemChat` ([`InGameUi::system_chat`]).
//!
//! Scripts run on the layer's VM with [`InGameUi::exec`]; `hint`, `hintSilent` and
//! `systemChat` reach the HUD.
//!
//! The behaviour follows `docs/re/ingame-ui.md`.
//!
//! ```no_run
//! # use std::sync::Arc;
//! # let config: Arc<a3_config::ConfigTree> = todo!();
//! # let mut fonts: a3_ui::Fonts = todo!();
//! use a3_ingame_ui::{InGameUi, loadout::spawned_unit_info};
//! let mut hud = InGameUi::new(config.clone(), None, None, a3_ui::Screen::new(1920, 1080));
//! let info = spawned_unit_info(&config, "B_Soldier_F", &|_| None);
//! hud.update(&info, 0.0);
//! let quads = hud.draw(&mut fonts); // hand to the UI render feature
//! ```

pub mod chat;
pub mod colors;
pub mod format;
pub mod hint;
pub mod host;
mod hud;
pub mod idc;
pub mod loadout;
pub mod stance;
pub mod unit_info;

pub use colors::{IguiColors, MagazineTimers, Side};
pub use hint::{HintConfig, HintText, SoundRef};
pub use hud::{Dimm, HudOptions, InGameUi};
pub use stance::{Stance, StanceAdjust, StanceState, StanceTextures};
pub use unit_info::{
    Freefall, LoadedMagazine, ThrowableState, UnitInfo, WeaponState, driven_control,
};
