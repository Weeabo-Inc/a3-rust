//! LZ4 block decompression (a thin wrapper over `lz4_flex`).
//!
//! No shipped Arma 3 2.22 data examined so far uses LZ4: ODOL v73, OPRW v25 and PAA use LZO1X
//! or LZSS, and the executable carries no LZ4 strings. The codec is kept for formats or versions
//! that turn out to need it. An LZ4 block does not mark its own end, so the caller must know the
//! compressed length.

use crate::Error;

/// Best case LZ4 ratio is just under 255:1; anything above this cannot be valid.
const MAX_RATIO: usize = 255;

/// Decompresses one LZ4 block, exactly `input`, that must produce exactly `expected_len` bytes.
pub fn decompress(input: &[u8], expected_len: usize) -> Result<Vec<u8>, Error> {
    if expected_len > input.len().saturating_mul(MAX_RATIO) + 16 {
        // Refuse before allocating: a corrupt length must not reserve gigabytes.
        return Err(Error::UnexpectedEof {
            consumed: input.len(),
            produced: 0,
            expected: expected_len,
        });
    }
    let mut out = vec![0u8; expected_len];
    let produced = lz4_flex::block::decompress_into(input, &mut out)?;
    if produced != expected_len {
        return Err(Error::OutputUnderrun {
            produced,
            expected: expected_len,
        });
    }
    Ok(out)
}
