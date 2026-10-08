/// Errors from the audio engine.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No output device, or the device rejected the stream.
    #[error("audio device: {0}")]
    Device(String),
    /// A sound file could not be decoded.
    #[error(transparent)]
    Decode(#[from] a3_audio_formats::Error),
    /// A sound this engine cannot play.
    #[error("unsupported sound: {0}")]
    Unsupported(String),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
