//! Decoded sounds held in memory.

use std::sync::Arc;
use std::time::Duration;

use a3_audio_formats::Sound;

use crate::{Error, Result};

/// A decoded sound, shared between voices: 16-bit PCM, mono or stereo.
#[derive(Debug, Clone)]
pub struct Clip(Arc<ClipData>);

#[derive(Debug)]
struct ClipData {
    sample_rate: u32,
    channels: u16,
    samples: Vec<i16>,
}

impl Clip {
    /// Wraps decoded PCM. More than two channels keep only the first two.
    pub fn from_sound(sound: Sound) -> Result<Self> {
        if sound.channels == 0 || sound.sample_rate == 0 {
            return Err(Error::Unsupported("sound with no channels or rate".into()));
        }
        let (channels, samples) = if sound.channels <= 2 {
            (sound.channels, sound.samples)
        } else {
            let n = usize::from(sound.channels);
            let stereo = sound
                .samples
                .chunks_exact(n)
                .flat_map(|frame| [frame[0], frame[1]])
                .collect();
            (2, stereo)
        };
        Ok(Self(Arc::new(ClipData {
            sample_rate: sound.sample_rate,
            channels,
            samples,
        })))
    }

    /// Decodes a WSS, Ogg Vorbis or WAV file.
    pub fn decode(data: &[u8]) -> Result<Self> {
        Self::from_sound(a3_audio_formats::decode(data)?)
    }

    /// Frames per second.
    pub fn sample_rate(&self) -> u32 {
        self.0.sample_rate
    }

    /// 1 or 2.
    pub fn channels(&self) -> u16 {
        self.0.channels
    }

    /// Number of frames.
    pub fn frames(&self) -> u64 {
        (self.0.samples.len() / usize::from(self.0.channels)) as u64
    }

    /// Play length at the native rate.
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.frames() as f64 / f64::from(self.0.sample_rate))
    }

    /// Frame `index` as `[left, right]` in `-1..1` (mono is duplicated).
    pub fn frame(&self, index: u64) -> Option<[f32; 2]> {
        let i = usize::try_from(index).ok()?;
        let s = &self.0.samples;
        let to_f32 = |v: i16| f32::from(v) / 32768.0;
        if self.0.channels == 1 {
            s.get(i).map(|&v| [to_f32(v); 2])
        } else {
            Some([to_f32(*s.get(2 * i)?), to_f32(*s.get(2 * i + 1)?)])
        }
    }
}
