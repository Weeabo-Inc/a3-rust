//! `mission.sqm`: the mission file, in config syntax or rapified.
//!
//! The file is the same syntax as `config.cpp`, so parsing reuses [`a3_config`]: [`parse_sqm`]
//! decodes the bytes (UTF-8, or UTF-16 with a byte-order mark, as [`a3_gamedata::decode_text`]
//! does for scripts) and parses them with [`a3_config::parse_text`]; a rapified file
//! ([`a3_config::is_rap`]) goes through [`a3_config::read_rap`]. See
//! `docs/adr/0010-mission-sqm-parsing.md` and `docs/re/missions.md`.
//!
//! Shape of the parsed config (from the shipped campaign missions):
//!
//! ```text
//! version=12;
//! class Mission {
//!     addOns[]={...}; addOnsAuto[]={...}; randomSeed=...;
//!     class Intel { briefingName=...; year=...; hour=...; ... };
//!     class Groups  { items=N; class ItemK { side="WEST"; class Vehicles { items=N; class ItemK {...}; };
//!                                           class Waypoints { items=N; class ItemK {...}; }; }; };
//!     class Vehicles { items=N; class ItemK {...}; };   // units outside any group
//!     class Markers { items=N; class ItemK {...}; };
//!     class Sensors { items=N; class ItemK {...}; };
//! };
//! class Intro { addOns[]={...}; randomSeed=...; class Intel {...}; };   // also the outro scenes
//! ```
//!
//! Every list is `items=N` plus `class Item0..ItemN-1`; the item index is not the entity id
//! (`id=`), which is ordered differently (in `boot_m02.altis` the ungrouped objects start at
//! `id=32`).

use a3_config::{Config, ParseError, RapError, is_rap, parse_text, read_rap};
use a3_gamedata::decode_text;

/// Errors from reading a `mission.sqm`.
#[derive(Debug, thiserror::Error)]
pub enum SqmError {
    /// The text form did not parse.
    #[error("{0}")]
    Text(#[from] ParseError),
    /// The rapified form did not parse.
    #[error("rapified mission.sqm: {0}")]
    Rap(#[from] RapError),
    /// The file is neither a text nor a rapified config (e.g. a gzip or binarised file).
    #[error("not a config file")]
    NotAConfig,
}

/// Parses the bytes of a `mission.sqm`: rapified with [`read_rap`], text through
/// [`parse_text`] after [`decode_text`].
pub fn parse_sqm(bytes: &[u8]) -> Result<Config, SqmError> {
    if is_rap(bytes) {
        return read_rap(bytes).map_err(SqmError::Rap);
    }
    let text = decode_text(bytes);
    parse_text(&text).map_err(SqmError::Text)
}
