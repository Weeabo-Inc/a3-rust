//! The audio engine: a software mixer on its own thread, feeding a cpal output stream (or
//! nothing, for CI and `--audio off`), with 3D placement of voices in RV world space.
//!
//! - [`AudioEngine`] is the game-side handle: it starts the output and sends [`Command`]s.
//! - [`Mixer`] does the work on the audio thread and can be driven directly in tests.
//! - [`VolumeBus`] holds the master, sound, music and radio gains the fade commands drive.
//! - [`Clip`] holds decoded sounds; [`Stream`] decodes long Ogg files while they play.
//! - [`spatial`] holds the listener/emitter model: distance curves, panning, doppler, filters.

mod clip;
pub mod config;
mod curve;
mod engine;
mod error;
pub mod expr;
mod filter;
mod loader;
mod mixer;
pub mod player;
pub mod spatial;
mod stream;
mod volume;

pub use clip::Clip;
pub use curve::Curve;
pub use engine::{AudioEngine, Backend, EngineConfig, EngineStats};
pub use error::{Error, Result};
pub use filter::LowPass;
pub use loader::{SOUND_EXTENSIONS, STREAM_SECONDS, SoundLoader, resolve_sound_path};
pub use mixer::{Command, Mixer, MixerStats, PlayParams, Source, VoiceId};
pub use spatial::{DistanceFilter, Emitter, Listener};
pub use stream::Stream;
pub use volume::{Bus, Fade, TICKS_PER_SECOND, VolumeBus, seconds_to_ticks};
