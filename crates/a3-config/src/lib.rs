//! The Real Virtuality config system.
//!
//! - [`Config`] is one parsed config file (config.cpp / config.bin / description.ext /
//!   mission.sqm / rvmat ...).
//! - [`read_rap`] / [`write_rap`] convert to and from the rapified binary form.
//! - [`parse_text`] / [`write_text`] convert to and from config.cpp syntax (the input must
//!   already be preprocessed).
//! - [`ConfigTree`] merges many configs in load order and answers inheritance-aware queries
//!   through [`ConfigRef`] (`tree.root() >> "CfgVehicles" >> "B_Soldier_F"`).
//! - [`load_order`] sorts addons by their CfgPatches `requiredAddons`.

mod model;
mod order;
mod rap;
mod text;
mod tree;

pub use model::{Config, ConfigClass, Entry, EntryKind, EnumEntry, Value};
pub use order::{AddonPatches, LoadOrder, load_order};
pub use rap::{RapError, is_rap, read_rap, write_rap};
pub use text::{ParseError, parse_text, write_text};
pub use tree::{ConfigRef, ConfigTree};
