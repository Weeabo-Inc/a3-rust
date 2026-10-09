//! The environment of a World: date and time, sun and moon, the lighting tables of
//! `CfgWorlds`, weather (overcast) and fog.
//!
//! [`EnvironmentState`] is the mutable state SQF drives (`setDate`, `skipTime`,
//! `setOvercast`, `setFog`); [`WorldEnvironment`] holds the world's config tables; and
//! [`WorldEnvironment::evaluate`] turns both into the [`EnvironmentFrame`] the renderer
//! needs: sun and moon directions, light colours and intensities, sky and fog colours, and
//! exposure limits. See `docs/re/environment.md` for what follows the engine and what is
//! approximated.

mod celestial;
mod cfg;
mod datetime;
mod environment;
mod lighting;
mod weather;

pub use celestial::{
    Horizontal, Observer, legacy_directions, moon_illumination, moon_phase, moon_position,
    star_rotation, sun_position,
};
pub use datetime::{DateTime, days_in_month, is_leap_year};
pub use environment::{EnvironmentFrame, EnvironmentState, MAX_WIND_SPEED, WorldEnvironment};
pub use lighting::{LightingEntry, LightingTable, ev_color};
pub use weather::{Fog, FogLimits, OvercastLevel, OvercastSample, OvercastTable};
