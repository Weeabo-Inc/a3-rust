//! Decoders for the game's sound files: BI `WSS0`, Ogg Vorbis and WAV.
//!
//! Every format decodes to a [`Sound`]: interleaved signed 16-bit PCM. [`probe`] reads only the
//! header. The format is detected from the file signature, not the extension.
//!
//! See `docs/re/wss.md` for the WSS layout, its delta compression and a survey of the install.

mod error;
mod ogg;
mod wav;
mod wss;

use std::time::Duration;

pub use error::{Error, Result};
pub use ogg::VorbisStream;
pub use wss::{WssCompression, delta4_step, delta8_step};

/// Decoded PCM audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sound {
    /// Frames per second.
    pub sample_rate: u32,
    /// Interleaved channels per frame.
    pub channels: u16,
    /// Interleaved samples, `channels` per frame.
    pub samples: Vec<i16>,
}

impl Sound {
    /// Number of frames (samples per channel).
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    /// Play length.
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.frames() as f64 / f64::from(self.sample_rate.max(1)))
    }

    /// The samples as `f32` in `-1.0..1.0`.
    pub fn to_f32(&self) -> Vec<f32> {
        self.samples
            .iter()
            .map(|&s| f32::from(s) / 32768.0)
            .collect()
    }

    /// Encodes the sound as a 16-bit PCM WAV file.
    pub fn to_wav(&self) -> Vec<u8> {
        wav::encode(self)
    }
}

/// A sound file format, as detected from its signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// BI `WSS0` with the given compression.
    Wss(WssCompression),
    /// Ogg Vorbis.
    OggVorbis,
    /// RIFF WAVE.
    Wav,
}

/// What a sound file holds, read from its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundInfo {
    /// File format.
    pub format: Format,
    /// Frames per second.
    pub sample_rate: u32,
    /// Interleaved channels per frame.
    pub channels: u16,
    /// Bits per decoded sample in the file (Vorbis: none).
    pub bits_per_sample: Option<u16>,
    /// Number of frames, when the header or the end of the stream gives it.
    pub frames: Option<u64>,
}

/// Reads the header of a sound file in any supported format.
pub fn probe(data: &[u8]) -> Result<SoundInfo> {
    match detect(data)? {
        Detected::Wss => wss::probe(data),
        Detected::Ogg => ogg::probe(data),
        Detected::Wav => wav::probe(data),
    }
}

/// Decodes a sound file in any supported format to 16-bit PCM.
pub fn decode(data: &[u8]) -> Result<Sound> {
    match detect(data)? {
        Detected::Wss => wss::decode(data),
        Detected::Ogg => ogg::decode(data),
        Detected::Wav => wav::decode(data),
    }
}

enum Detected {
    Wss,
    Ogg,
    Wav,
}

fn detect(data: &[u8]) -> Result<Detected> {
    match data.get(..4) {
        Some(b"WSS0") => Ok(Detected::Wss),
        Some(b"OggS") => Ok(Detected::Ogg),
        Some(b"RIFF") => Ok(Detected::Wav),
        _ => Err(Error::UnknownFormat),
    }
}
