//! `Cprs` entries: BI LZSS data followed by a 32-bit additive checksum.

use a3_compress::lzss::{self, ChecksumKind};

/// The checksum kind [`PboWriter`](crate::PboWriter) writes for `Cprs` entries.
///
/// **Assumption**, not yet confirmed: no shipped 2.22 PBO contains a `Cprs` entry, so `Signed`
/// is taken from PAA, the only RV format whose LZSS checksum has been verified (see
/// `docs/re/compression.md`; confirmation tracked in issue #59). Until then the reader accepts
/// both the signed and the unsigned byte sum.
pub const CPRS_CHECKSUM: ChecksumKind = ChecksumKind::Signed;

/// Unpacks the stored bytes of a `Cprs` entry to its `original_size` bytes.
pub(crate) fn unpack(raw: &[u8], original_size: usize) -> Result<Vec<u8>, a3_compress::Error> {
    let (out, used) = lzss::decompress(raw, original_size, ChecksumKind::None)?;
    let Some(stored) = raw.get(used..used + 4) else {
        return Err(a3_compress::Error::UnexpectedEof {
            consumed: raw.len(),
            produced: out.len(),
            expected: original_size,
        });
    };
    let stored = u32::from_le_bytes(stored.try_into().expect("4 bytes"));
    let signed = ChecksumKind::Signed.compute(&out);
    let unsigned = ChecksumKind::Unsigned.compute(&out);
    if Some(stored) != signed && Some(stored) != unsigned {
        let computed = CPRS_CHECKSUM
            .compute(&out)
            .expect("CPRS_CHECKSUM has a checksum");
        return Err(a3_compress::Error::ChecksumMismatch { stored, computed });
    }
    Ok(out)
}

/// Packs `data` as the stored bytes of a `Cprs` entry.
pub(crate) fn pack(data: &[u8]) -> Vec<u8> {
    lzss::compress(data, CPRS_CHECKSUM)
}
