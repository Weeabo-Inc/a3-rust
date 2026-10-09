//! CRC-32 of the plaintext datagram header (`docs/re/net-transport.md`, "Datagram layout").
//!
//! The receive path calls `Hashes::CRC32SSE` (RVA 0x286e10): the standard IEEE/zlib CRC-32, table
//! at 0x1a95320, polynomial 0xEDB88320, initial value 0xFFFFFFFF, final xor 0xFFFFFFFF, reflected
//! input and output. The header's own `crc` field is excluded by zeroing the four bytes before
//! the computation, so a datagram with a wrong crc is dropped without any further parsing.

/// CRC-32 polynomial, reversed form (`0xEDB88320` = the reverse of `0x04C11DB7`).
const POLY: u32 = 0xEDB8_8320;

/// The byte-at-a-time table the engine's non-SSE path uses; the SSE path folds the same CRC.
const TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// The engine's CRC-32 of `data`: init `0xFFFFFFFF`, final xor `0xFFFFFFFF`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc = (crc >> 8) ^ TABLE[((crc ^ u32::from(*byte)) & 0xFF) as usize];
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_standard_check_value() {
        // The CRC-32 check value every implementation is measured by.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
    }
}
