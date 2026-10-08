//! BI `WSS0` sounds: a 26-byte header and PCM or logarithmic-delta sample data.

use std::sync::OnceLock;

use crate::{Error, Format, Result, Sound, SoundInfo};

const HEADER_SIZE: usize = 26;

/// How the sample data of a WSS file is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WssCompression {
    /// Plain little-endian PCM (8-bit unsigned, 16- or 24-bit signed).
    None,
    /// One signed byte per 16-bit sample, a logarithmically quantised delta from the previous
    /// sample of the same channel (compression value 8).
    Delta8,
}

struct Header {
    compression: WssCompression,
    channels: u16,
    sample_rate: u32,
    bits: u16,
}

fn header(data: &[u8]) -> Result<Header> {
    let h = data.get(..HEADER_SIZE).ok_or(Error::Truncated)?;
    let u16_at = |i: usize| u16::from_le_bytes([h[i], h[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
    // 4: compression; 8: WAVEFORMATEX (format tag, channels, rate, bytes/s, block align, bits,
    // and a trailing u16 that is 0 except in a few files where it holds garbage).
    let compression = match u32_at(4) {
        0 => WssCompression::None,
        8 => WssCompression::Delta8,
        other => return Err(Error::Unsupported(format!("WSS compression {other}"))),
    };
    let format_tag = u16_at(8);
    if format_tag != 1 {
        return Err(Error::Unsupported(format!("WSS format tag {format_tag}")));
    }
    let header = Header {
        compression,
        channels: u16_at(10),
        sample_rate: u32_at(12),
        bits: u16_at(22),
    };
    if header.channels == 0 {
        return Err(Error::Malformed("WSS with 0 channels".into()));
    }
    let bits_ok = match compression {
        WssCompression::None => matches!(header.bits, 8 | 16 | 24),
        WssCompression::Delta8 => header.bits == 16,
    };
    if !bits_ok {
        return Err(Error::Unsupported(format!(
            "WSS {compression:?} with {} bits per sample",
            header.bits
        )));
    }
    Ok(header)
}

impl Header {
    /// Bytes of file data per stored sample.
    fn stored_sample_size(&self) -> usize {
        match self.compression {
            WssCompression::None => usize::from(self.bits / 8),
            WssCompression::Delta8 => 1,
        }
    }
}

pub fn probe(data: &[u8]) -> Result<SoundInfo> {
    let h = header(data)?;
    let frame_size = h.stored_sample_size() * usize::from(h.channels);
    Ok(SoundInfo {
        format: Format::Wss(h.compression),
        sample_rate: h.sample_rate,
        channels: h.channels,
        bits_per_sample: Some(h.bits),
        frames: Some(((data.len() - HEADER_SIZE) / frame_size) as u64),
    })
}

pub fn decode(data: &[u8]) -> Result<Sound> {
    let h = header(data)?;
    let body = &data[HEADER_SIZE..];
    // A trailing partial frame (seen in some files) is dropped.
    let frame_size = h.stored_sample_size() * usize::from(h.channels);
    let body = &body[..body.len() - body.len() % frame_size];
    let samples = match (h.compression, h.bits) {
        (WssCompression::None, 8) => body.iter().map(|&b| (i16::from(b) - 128) << 8).collect(),
        (WssCompression::None, 16) => body
            .chunks_exact(2)
            .map(|s| i16::from_le_bytes([s[0], s[1]]))
            .collect(),
        (WssCompression::None, _) => body
            .chunks_exact(3)
            .map(|s| i16::from_le_bytes([s[1], s[2]]))
            .collect(),
        (WssCompression::Delta8, _) => {
            let channels = usize::from(h.channels);
            let mut last = vec![0i16; channels];
            body.iter()
                .enumerate()
                .map(|(i, &code)| {
                    let channel = &mut last[i % channels];
                    *channel = channel.saturating_add(delta8_step(code));
                    *channel
                })
                .collect()
        }
    };
    Ok(Sound {
        sample_rate: h.sample_rate,
        channels: h.channels,
        samples,
    })
}

/// The sample change a `Delta8` code stands for: `sign(c) * round(32767^(|c| / 127))` for the
/// code `c` read as a signed byte, and 0 for code 0.
pub fn delta8_step(code: u8) -> i16 {
    static TABLE: OnceLock<[i16; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            let code = i as u8 as i8;
            if code == 0 {
                return 0;
            }
            let magnitude = (f64::from(code.unsigned_abs()) * 32767f64.ln() / 127.0)
                .exp()
                .round() as i16;
            magnitude * i16::from(code.signum())
        })
    })[usize::from(code)]
}
