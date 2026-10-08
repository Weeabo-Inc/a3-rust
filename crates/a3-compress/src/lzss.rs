//! Bohemia Interactive's LZSS variant.
//!
//! Used by PBO entries packed with `Cprs`, non-DXT PAA mipmaps, rapified files and compressed
//! arrays in older ODOL and WRP versions.
//!
//! # Stream format
//!
//! The data is a sequence of groups. Each group starts with a flag byte whose bits, read from
//! least to most significant, describe up to eight items:
//!
//! - bit `1`: a literal, one byte copied to the output.
//! - bit `0`: a back-reference, two bytes `b0 b1`:
//!   - `distance = b0 | (b1 & 0xF0) << 4` (12 bits, 1..=4095), counted back from the current
//!     output position (BI's variant; the classic Okumura LZSS stores an absolute ring-buffer
//!     position instead),
//!   - `length = (b1 & 0x0F) + 3` (3..=18).
//!
//!   Bytes before the start of the output read as spaces (`0x20`): the 4096-byte window starts
//!   out filled with them. Source and destination may overlap (run-length style).
//!
//! Decoding stops as soon as the expected output length is reached; the rest of the last flag
//! byte is ignored. A little-endian `u32` checksum may follow: the wrapping sum of all output
//! bytes, taken as signed or unsigned bytes depending on the file format (see [`ChecksumKind`]).

use std::io::Read;

use crate::Error;
use crate::input::{Input, MAX_PREALLOC, ReadSource, SliceSource, Source};

/// Size of the sliding window.
const WINDOW: usize = 4096;
/// Shortest back-reference.
const MIN_MATCH: usize = 3;
/// Longest back-reference.
const MAX_MATCH: usize = 18;
/// What the window holds before any output exists.
const FILL: u8 = b' ';

/// How the 32-bit checksum after an LZSS block is computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChecksumKind {
    /// Wrapping sum of the output bytes read as `i8` (sign-extended).
    Signed,
    /// Wrapping sum of the output bytes read as `u8`.
    Unsigned,
    /// No checksum follows the compressed data.
    None,
}

impl ChecksumKind {
    /// The checksum of `data`, or `None` for [`ChecksumKind::None`].
    pub fn compute(self, data: &[u8]) -> Option<u32> {
        match self {
            ChecksumKind::Signed => Some(
                data.iter()
                    .fold(0u32, |sum, &b| sum.wrapping_add(b as i8 as i32 as u32)),
            ),
            ChecksumKind::Unsigned => Some(
                data.iter()
                    .fold(0u32, |sum, &b| sum.wrapping_add(u32::from(b))),
            ),
            ChecksumKind::None => None,
        }
    }
}

/// Decompresses an LZSS block that produces exactly `expected_len` bytes.
///
/// `input` may extend past the block. Returns the output and the number of input bytes the block
/// occupied, checksum included.
pub fn decompress(
    input: &[u8],
    expected_len: usize,
    checksum: ChecksumKind,
) -> Result<(Vec<u8>, usize), Error> {
    decode(SliceSource::new(input), expected_len, checksum)
}

/// Like [`decompress`], reading from a stream. Reads exactly the bytes of the block, leaving
/// `reader` at the first byte after it.
pub fn decompress_from<R: Read>(
    reader: R,
    expected_len: usize,
    checksum: ChecksumKind,
) -> Result<(Vec<u8>, usize), Error> {
    decode(ReadSource(reader), expected_len, checksum)
}

/// Compresses `data` into a block that [`decompress`] reads back with the same `checksum`.
///
/// A greedy encoder over hash chains: correct and reasonably compact, not byte-identical to
/// BI's tools. It never references the initial window of spaces.
pub fn compress(data: &[u8], checksum: ChecksumKind) -> Vec<u8> {
    const HASH_BITS: u32 = 13;
    const MAX_CHAIN: usize = 64;
    const NONE: usize = usize::MAX;

    let hash = |p: usize| {
        let v = u32::from(data[p]) | u32::from(data[p + 1]) << 8 | u32::from(data[p + 2]) << 16;
        (v.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
    };
    // Most recent position per hash, and per position (mod window) the previous one.
    let mut head = vec![NONE; 1 << HASH_BITS];
    let mut prev = vec![NONE; WINDOW];
    let insert = |head: &mut [usize], prev: &mut [usize], p: usize| {
        if p + MIN_MATCH <= data.len() {
            let h = hash(p);
            prev[p % WINDOW] = head[h];
            head[h] = p;
        }
    };

    let mut out = Vec::with_capacity(data.len() + data.len() / 8 + 5);
    let mut flag_at = 0;
    let mut bit = 8;
    let mut pos = 0;
    while pos < data.len() {
        if bit == 8 {
            flag_at = out.len();
            out.push(0);
            bit = 0;
        }

        let max_len = MAX_MATCH.min(data.len() - pos);
        let (mut best_len, mut best_dist) = (0, 0);
        if max_len >= MIN_MATCH {
            let mut cand = head[hash(pos)];
            let mut steps = 0;
            while cand != NONE && pos - cand < WINDOW && steps < MAX_CHAIN {
                let len = (0..max_len)
                    .take_while(|&k| data[cand + k] == data[pos + k])
                    .count();
                if len > best_len {
                    (best_len, best_dist) = (len, pos - cand);
                    if len == max_len {
                        break;
                    }
                }
                let next = prev[cand % WINDOW];
                if next == NONE || next >= cand {
                    break;
                }
                cand = next;
                steps += 1;
            }
        }

        if best_len >= MIN_MATCH {
            out.push(best_dist as u8);
            out.push(((best_dist >> 4) & 0xF0) as u8 | (best_len - MIN_MATCH) as u8);
            for p in pos..pos + best_len {
                insert(&mut head, &mut prev, p);
            }
            pos += best_len;
        } else {
            out[flag_at] |= 1 << bit;
            out.push(data[pos]);
            insert(&mut head, &mut prev, pos);
            pos += 1;
        }
        bit += 1;
    }

    if let Some(sum) = checksum.compute(data) {
        out.extend_from_slice(&sum.to_le_bytes());
    }
    out
}

fn decode<S: Source>(
    source: S,
    expected_len: usize,
    checksum: ChecksumKind,
) -> Result<(Vec<u8>, usize), Error> {
    let mut input = Input::new(source, expected_len);
    let mut out = Vec::with_capacity(expected_len.min(MAX_PREALLOC));
    let mut flags: u32 = 0;

    while out.len() < expected_len {
        flags >>= 1;
        if flags & 0x100 == 0 {
            // The high byte marks how many flag bits remain.
            flags = u32::from(input.byte(out.len())?) | 0xFF00;
        }

        if flags & 1 != 0 {
            out.push(input.byte(out.len())?);
            continue;
        }

        let b0 = usize::from(input.byte(out.len())?);
        let b1 = usize::from(input.byte(out.len())?);
        let distance = b0 | ((b1 & 0xF0) << 4);
        let length = (b1 & 0x0F) + MIN_MATCH;

        if distance == 0 {
            return Err(Error::InvalidDistance {
                at: out.len(),
                distance,
            });
        }
        if out.len() + length > expected_len {
            return Err(Error::OutputOverrun {
                expected: expected_len,
            });
        }
        for _ in 0..length {
            let pos = out.len();
            let b = if distance > pos {
                FILL
            } else {
                out[pos - distance]
            };
            out.push(b);
        }
    }

    if let Some(computed) = checksum.compute(&out) {
        let stored = input.u32_le(out.len())?;
        if stored != computed {
            return Err(Error::ChecksumMismatch { stored, computed });
        }
    }

    Ok((out, input.consumed()))
}
