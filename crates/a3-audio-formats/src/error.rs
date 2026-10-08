/// Errors from reading a sound file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The data starts with none of `WSS0`, `OggS` or `RIFF`.
    #[error("unknown sound format")]
    UnknownFormat,

    /// The file ends inside its header.
    #[error("sound header is truncated")]
    Truncated,

    /// A valid file using a variant this crate does not decode.
    #[error("unsupported sound: {0}")]
    Unsupported(String),

    /// The header holds impossible values.
    #[error("malformed sound: {0}")]
    Malformed(String),

    /// The Ogg Vorbis decoder rejected the stream.
    #[error("Ogg Vorbis: {0}")]
    Vorbis(String),

    /// The WAV decoder rejected the file.
    #[error("WAV: {0}")]
    Wav(String),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
