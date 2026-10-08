//! Bridge to the decompression codecs.

use crate::cursor::Codec;

/// Decompresses one block of `len` output bytes from the start of `input`. Returns the output
/// and the number of input bytes consumed.
pub(crate) fn decompress(
    codec: Codec,
    input: &[u8],
    len: usize,
) -> Result<(Vec<u8>, usize), String> {
    // Placeholder until `a3-compress` (#8) lands.
    let _ = (input, len);
    Err(format!("{codec:?} decompression is not available yet"))
}
