//! The audio engine: a software mixer on its own thread, feeding a cpal output stream (or
//! nothing, for CI and `--audio off`), with 3D placement of voices in RV world space.
//!
//! - [`AudioEngine`] is the game-side handle: it starts the output and sends [`Command`]s.
//! - [`Mixer`] does the work on the audio thread and can be driven directly in tests.
//! - [`Clip`] holds decoded sounds; [`Stream`] decodes long Ogg files while they play.
//! - [`spatial`] holds the listener/emitter model: distance curves, panning, doppler, filters.

mod clip;
pub mod config;
mod curve;
mod engine;
mod error;
pub mod expr;
mod filter;
mod mixer;
pub mod spatial;
mod stream;

pub use clip::Clip;
pub use curve::Curve;
pub use engine::{AudioEngine, Backend, EngineConfig, EngineStats};
pub use error::{Error, Result};
pub use filter::LowPass;
pub use mixer::{Command, Mixer, MixerStats, PlayParams, Source, VoiceId};
pub use spatial::{DistanceFilter, Emitter, Listener};
pub use stream::Stream;
