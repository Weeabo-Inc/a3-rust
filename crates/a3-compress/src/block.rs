//! Reading compressed blocks inline from a stream, as ODOL and WRP readers do.

use std::io::Read;

use crate::lzss::ChecksumKind;
use crate::{Error, lz4, lzo, lzss};

/// In WRP (OPRW) files, arrays whose data is at least this many bytes are stored compressed and
/// smaller ones raw. ODOL v73 does not use this rule: a per-array flag byte decides there.
pub const COMPRESSED_ARRAY_THRESHOLD: usize = 1024;

/// Which codec a compressed block uses. The file format and its version decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Codec {
    /// BI LZSS with the given trailing checksum.
    Lzss(ChecksumKind),
    /// LZO1X; the block ends with its own end marker.
    Lzo,
    /// An LZ4 block. LZ4 blocks do not mark their end, so the compressed length must be known.
    Lz4 { compressed_len: usize },
}

/// Decompresses one block of `codec` data producing `expected_len` bytes from `reader`.
///
/// Reads exactly the bytes of the block, leaving `reader` at the first byte after it. Reads one
/// byte at a time for LZSS and LZO, so pass a buffered reader (or a slice).
pub fn read_compressed_block<R: Read>(
    mut reader: R,
    expected_len: usize,
    codec: Codec,
) -> Result<Vec<u8>, Error> {
    match codec {
        Codec::Lzss(checksum) => lzss::decompress_from(reader, expected_len, checksum).map(|r| r.0),
        Codec::Lzo => lzo::decompress_from(reader, expected_len).map(|r| r.0),
        Codec::Lz4 { compressed_len } => {
            let mut packed = Vec::new();
            let read = reader
                .by_ref()
                .take(compressed_len as u64)
                .read_to_end(&mut packed)?;
            if read != compressed_len {
                return Err(Error::UnexpectedEof {
                    consumed: read,
                    produced: 0,
                    expected: expected_len,
                });
            }
            lz4::decompress(&packed, expected_len)
        }
    }
}

/// Reads the data of a WRP (OPRW) compressed array of `byte_len` bytes: raw when it is shorter
/// than [`COMPRESSED_ARRAY_THRESHOLD`], otherwise a [`read_compressed_block`] of `codec`.
///
/// Do not use it for ODOL v73: those arrays carry a flag byte (`0` raw, `2` LZO1X) that decides
/// regardless of size; small arrays can be LZO and large ones raw. The caller reads the element
/// count first.
pub fn read_compressed_array<R: Read>(
    mut reader: R,
    byte_len: usize,
    codec: Codec,
) -> Result<Vec<u8>, Error> {
    if byte_len >= COMPRESSED_ARRAY_THRESHOLD {
        return read_compressed_block(reader, byte_len, codec);
    }
    let mut raw = vec![0u8; byte_len];
    reader.read_exact(&mut raw).map_err(|e| match e.kind() {
        std::io::ErrorKind::UnexpectedEof => Error::UnexpectedEof {
            consumed: 0,
            produced: 0,
            expected: byte_len,
        },
        _ => Error::Io(e),
    })?;
    Ok(raw)
}
