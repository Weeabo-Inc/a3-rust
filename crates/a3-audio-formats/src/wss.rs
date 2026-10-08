//! BI `WSS0` sounds: a 26-byte header and PCM or delta-coded sample data.

use std::sync::OnceLock;

use crate::{Error, Format, Result, Sound, SoundInfo};

const HEADER_SIZE: usize = 26;

/// How the sample data of a WSS file is stored (the `u32` after the signature).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WssCompression {
    /// 0: plain little-endian PCM (8-bit unsigned, 16- or 24-bit signed).
    None,
    /// 4: two 4-bit codes per byte (high nibble first), each a delta from a fixed table.
    Delta4,
    /// 8: one signed byte per 16-bit sample, a logarithmically quantised delta.
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
        4 => WssCompression::Delta4,
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
        WssCompression::Delta4 | WssCompression::Delta8 => header.bits == 16,
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
    /// Whole frames held by `len` bytes of sample data.
    fn frames(&self, len: usize) -> usize {
        let channels = usize::from(self.channels);
        match self.compression {
            WssCompression::None => len / (usize::from(self.bits / 8) * channels),
            WssCompression::Delta4 => len * 2 / channels,
            WssCompression::Delta8 => len / channels,
        }
    }
}

pub fn probe(data: &[u8]) -> Result<SoundInfo> {
    let h = header(data)?;
    Ok(SoundInfo {
        format: Format::Wss(h.compression),
        sample_rate: h.sample_rate,
        channels: h.channels,
        bits_per_sample: Some(h.bits),
        frames: Some(h.frames(data.len() - HEADER_SIZE) as u64),
    })
}

pub fn decode(data: &[u8]) -> Result<Sound> {
    let h = header(data)?;
    let body = &data[HEADER_SIZE..];
    let channels = usize::from(h.channels);
    let mut samples: Vec<i16> = match (h.compression, h.bits) {
        (WssCompression::None, 8) => body.iter().map(|&b| (i16::from(b) - 128) << 8).collect(),
        (WssCompression::None, 16) => body
            .chunks_exact(2)
            .map(|s| i16::from_le_bytes([s[0], s[1]]))
            .collect(),
        (WssCompression::None, _) => body
            .chunks_exact(3)
            .map(|s| i16::from_le_bytes([s[1], s[2]]))
            .collect(),
        (WssCompression::Delta4, _) => {
            let codes = body.iter().flat_map(|&b| [b >> 4, b & 0x0f]);
            accumulate(codes.map(delta4_step), channels)
        }
        (WssCompression::Delta8, _) => accumulate(body.iter().map(|&c| delta8_step(c)), channels),
    };
    // A trailing partial frame (seen in some files) is dropped.
    samples.truncate(h.frames(body.len()) * channels);
    Ok(Sound {
        sample_rate: h.sample_rate,
        channels: h.channels,
        samples,
    })
}

/// Sums interleaved deltas per channel. The running sums are unbounded `i32`s; only the output
/// samples saturate to the `i16` range (as the engine does).
fn accumulate(deltas: impl Iterator<Item = i32>, channels: usize) -> Vec<i16> {
    let mut sums = vec![0i32; channels];
    deltas
        .enumerate()
        .map(|(i, delta)| {
            let sum = &mut sums[i % channels];
            *sum += delta;
            (*sum).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
        })
        .collect()
}

/// The sample change a `Delta4` nibble stands for.
pub fn delta4_step(nibble: u8) -> i32 {
    const TABLE: [i32; 16] = [
        -8192, -4096, -2048, -1024, -512, -256, -64, 0, 64, 256, 512, 1024, 2048, 4096, 8192, 0,
    ];
    TABLE[usize::from(nibble & 0x0f)]
}

/// The sample change a `Delta8` code stands for. For the code `c` read as a signed byte:
/// `sign(c) * round(1.0853122^|c|)`, computed in double precision, narrowed to `f32` and rounded
/// to nearest-even; 0 for `c = 0` and for `c = -128`.
pub fn delta8_step(code: u8) -> i32 {
    static TABLE: OnceLock<[i32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        // About 32767^(1/127); the engine's constant is this f32 (1.0853122...).
        let base = f64::from(f32::from_bits(0x3f8a_eb83));
        std::array::from_fn(|i| {
            let code = i as u8 as i8;
            if code == 0 || code == i8::MIN {
                return 0;
            }
            let magnitude = (base.powf(f64::from(code.unsigned_abs())) as f32).round_ties_even();
            magnitude as i32 * i32::from(code.signum())
        })
    })[usize::from(code)]
}
