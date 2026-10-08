//! Bridge to the decompression codecs of `a3-compress`.

use a3_compress::lzss::ChecksumKind;

use crate::cursor::Codec;

/// Decompresses one block of `len` output bytes from the start of `input`. Returns the output
/// and the number of input bytes consumed.
pub(crate) fn decompress(
    codec: Codec,
    input: &[u8],
    len: usize,
) -> Result<(Vec<u8>, usize), String> {
    let result = match codec {
        Codec::Lzo => a3_compress::lzo::decompress(input, len),
        // Pre-23 terrains: LZSS with an unsigned checksum _(uncertain: no such file shipped)_.
        Codec::Lzss => a3_compress::lzss::decompress(input, len, ChecksumKind::Unsigned),
    };
    result.map_err(|e| e.to_string())
}
