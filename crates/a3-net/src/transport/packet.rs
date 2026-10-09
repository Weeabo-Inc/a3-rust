//! Building and parsing one datagram (`docs/re/net-transport.md`, "Datagram layout").
//!
//! ```text
//! 0  u16  size      total datagram length, NOT obfuscated
//! 2  u16  flags     obfuscated
//! 4  u32  crc       CRC-32 of the plaintext datagram with this field zeroed, NOT obfuscated
//! 8  u32  serial    }
//! 12 u32  ack       }  obfuscated as one block, bytes [8, 24)
//! 16 u64  ack_mask  }  or u32 ack_mask + u32 extra when flags & 0x2500 != 0
//! 24 ..   payload   obfuscated with a keystream selected by the plaintext serial
//! ```
//!
//! Send order matters: compute the CRC over the plaintext, XOR the payload (it needs the plaintext
//! serial), then XOR the header. Receive reverses it: header first, then payload (again needing the
//! now-plaintext serial), then verify.

use crate::crc32::crc32;
use crate::error::NetError;
use crate::transport::flags::uses_extra;
use crate::transport::keys::{Keys, payload_offset};

/// Bytes of header before the payload.
pub const HEADER_SIZE: usize = 24;

/// The largest datagram the receiver accepts (`size <= 0x800`).
pub const MAX_DATAGRAM: usize = 0x800;

/// The largest payload one datagram can carry.
pub const MAX_PAYLOAD: usize = MAX_DATAGRAM - HEADER_SIZE;

/// The caller-controlled half of a datagram header.
///
/// `size` and `crc` are not here: [`pack`] derives them, and [`unpack`] checks them rather than
/// handing them out. `ack_mask` is 64 bits wide even when the flags select the 32-bit form, where
/// it always fits; `extra` is only meaningful when [`Header::uses_extra`] holds, and reading a
/// header clears it otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Header {
    pub flags: u16,
    pub serial: u32,
    pub ack: u32,
    pub ack_mask: u64,
    pub extra: u32,
}

impl Header {
    /// A header for the next datagram of a channel.
    pub fn new(flags: u16, serial: u32) -> Self {
        Self {
            flags,
            serial,
            ..Self::default()
        }
    }

    /// Whether `extra` is in use and the ack mask is therefore 32-bit.
    pub fn uses_extra(&self) -> bool {
        uses_extra(self.flags)
    }
}

/// Build one datagram: plaintext fields, CRC, payload XOR, header XOR.
pub fn pack(header: &Header, payload: &[u8], keys: &Keys) -> Result<Vec<u8>, NetError> {
    let size = HEADER_SIZE + payload.len();
    if size > MAX_DATAGRAM {
        return Err(NetError::payload_too_large(payload.len()));
    }
    let mut buf = vec![0u8; size];
    buf[0..2].copy_from_slice(&(size as u16).to_le_bytes());
    buf[2..4].copy_from_slice(&header.flags.to_le_bytes());
    // bytes 4..8 stay zero for the CRC.
    buf[8..12].copy_from_slice(&header.serial.to_le_bytes());
    buf[12..16].copy_from_slice(&header.ack.to_le_bytes());
    if header.uses_extra() {
        buf[16..20].copy_from_slice(&(header.ack_mask as u32).to_le_bytes());
        buf[20..24].copy_from_slice(&header.extra.to_le_bytes());
    } else {
        buf[16..24].copy_from_slice(&header.ack_mask.to_le_bytes());
    }
    buf[HEADER_SIZE..].copy_from_slice(payload);
    let crc = crc32(&buf);
    buf[4..8].copy_from_slice(&crc.to_le_bytes());
    obfuscate(&mut buf, header.serial, keys);
    Ok(buf)
}

/// Parse one datagram: length checks, de-obfuscation, CRC check.
///
/// Every failure is a drop on the receive path, not a fatal error.
pub fn unpack(datagram: &[u8], keys: &Keys) -> Result<(Header, Vec<u8>), NetError> {
    let len = datagram.len();
    if !(HEADER_SIZE..=MAX_DATAGRAM).contains(&len) {
        return Err(NetError::datagram_length(len));
    }
    let declared = u16::from_le_bytes([datagram[0], datagram[1]]);
    if usize::from(declared) != len {
        return Err(NetError::DatagramSize {
            declared,
            actual: len,
        });
    }
    let mut buf = datagram.to_vec();
    // Header first: the payload keystream is selected by the plaintext serial.
    xor_header(&mut buf, keys);
    let serial = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    xor_payload(&mut buf, serial, keys);
    let crc = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    buf[4..8].fill(0);
    let computed = crc32(&buf);
    if crc != computed {
        return Err(NetError::DatagramCrc {
            declared: crc,
            computed,
        });
    }
    let flags = u16::from_le_bytes([buf[2], buf[3]]);
    let header = Header {
        flags,
        serial,
        ack: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
        ack_mask: if uses_extra(flags) {
            u64::from(u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]))
        } else {
            u64::from_le_bytes([
                buf[16], buf[17], buf[18], buf[19], buf[20], buf[21], buf[22], buf[23],
            ])
        },
        extra: if uses_extra(flags) {
            u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]])
        } else {
            0
        },
    };
    Ok((header, buf[HEADER_SIZE..].to_vec()))
}

/// Apply the obfuscation in place, or remove it: the operations are XORs.
///
/// `serial` must be the *plaintext* serial; callers pass it explicitly so the order cannot be got
/// wrong (the payload keystream depends on it).
fn obfuscate(buf: &mut [u8], serial: u32, keys: &Keys) {
    xor_payload(buf, serial, keys);
    xor_header(buf, keys);
}

/// XOR the payload bytes `[24, size)` with the table selected by `serial`.
fn xor_payload(buf: &mut [u8], serial: u32, keys: &Keys) {
    let table = keys.payload_table();
    let start = payload_offset(serial) as usize;
    for (n, byte) in buf[HEADER_SIZE..].iter_mut().enumerate() {
        *byte ^= table[(start + n) & 0x7FF];
    }
}

/// XOR header bytes `[2, 4)` and `[8, 24)` with their constant masks.
fn xor_header(buf: &mut [u8], keys: &Keys) {
    for (i, mask) in keys.flags_mask().iter().enumerate() {
        buf[2 + i] ^= mask;
    }
    for (i, mask) in keys.tail_mask().iter().enumerate() {
        buf[8 + i] ^= mask;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::flags;

    /// One datagram built by the reference codec (independent implementation, same document):
    /// `python tools/re/a3net.py packet <flags> <serial> <ack> <mask> <extra> <payload-hex>`.
    struct Fixture {
        name: &'static str,
        header: Header,
        payload: &'static str,
        datagram: &'static str,
    }

    const FIXTURES: &[Fixture] = &[
        Fixture {
            name: "client HELLO on a connection-less control datagram",
            header: Header {
                flags: 0x0801,
                serial: 1000,
                ack: 7,
                ack_mask: 0xFFFF,
                extra: 0,
            },
            payload: "6415babb25252525de000000",
            datagram: "24009a65e91e4b67572461a1c4e5d47380e6b5995dc020d0528e77dd96fc4913c50d0683",
        },
        Fixture {
            name: "accepted RESULT, reliable, 64-bit ack mask",
            header: Header {
                flags: 0x8001,
                serial: 42,
                ack: 17,
                ack_mask: 0x8000_0000_0005_0001,
                extra: 0,
            },
            payload: "7e1aa5aa0000000003000000",
            datagram: "24009aed2a07d45d952761a1d2e5d4737e19b0995dc020508b6098b48fc763b1dbec763b",
        },
        Fixture {
            name: "ORDERED datagram: 32-bit ack mask and extra in use",
            header: Header {
                flags: 0x2000,
                serial: 5,
                ack: 4,
                ack_mask: 0x3,
                extra: 2,
            },
            payload: "4141414142",
            datagram: "1d009b4dc99538b3ba2761a1c7e5d4737c19b5995fc020d08a24f3982e",
        },
        Fixture {
            name: "rejected RESULT, no channel",
            header: Header {
                flags: 0x1001,
                serial: 0,
                ack: 0,
                ack_mask: 0,
                extra: 0,
            },
            payload: "7e1aa5aa0200000000000000",
            datagram: "24009a7d007aa04bbf2761a1c3e5d4737f19b5995dc020d0c0c54a5df97dbedfeff7fb7d",
        },
    ];

    fn decode_hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("test hex"))
            .collect()
    }

    #[test]
    fn pack_reproduces_the_reference_datagrams() {
        let keys = Keys::default();
        for fixture in FIXTURES {
            let packed = pack(&fixture.header, &decode_hex(fixture.payload), &keys)
                .unwrap_or_else(|e| panic!("{}: {e}", fixture.name));
            assert_eq!(
                packed,
                decode_hex(fixture.datagram),
                "{}: packed datagram differs",
                fixture.name
            );
        }
    }

    #[test]
    fn unpack_parses_the_reference_datagrams() {
        let keys = Keys::default();
        for fixture in FIXTURES {
            let (header, payload) = unpack(&decode_hex(fixture.datagram), &keys)
                .unwrap_or_else(|e| panic!("{}: {e}", fixture.name));
            assert_eq!(header, fixture.header, "{}: header", fixture.name);
            assert_eq!(
                payload,
                decode_hex(fixture.payload),
                "{}: payload",
                fixture.name
            );
        }
    }

    #[test]
    fn size_is_plaintext_and_flags_are_obfuscated() {
        let keys = Keys::default();
        let header = Header {
            flags: flags::FLAGS_CONTROL,
            serial: 1,
            ack: 0,
            ack_mask: 0,
            extra: 0,
        };
        let datagram = pack(&header, b"payload", &keys).expect("pack");
        assert_eq!(
            u16::from_le_bytes([datagram[0], datagram[1]]),
            datagram.len() as u16
        );
        // A non-zero mask means the plaintext flags never appear verbatim.
        assert_ne!(&datagram[2..4], &header.flags.to_le_bytes());
        assert_ne!(&datagram[8..12], &header.serial.to_le_bytes());
    }

    #[test]
    fn rejects_datagrams_that_fail_the_receive_checks() {
        let keys = Keys::default();
        let header = Header::new(flags::FLAGS_CONTROL, 1000);
        let datagram = pack(&header, b"hello arma", &keys).expect("pack");

        // Not a datagram: too short, too long, or a wrong size field.
        assert!(matches!(
            unpack(&datagram[..HEADER_SIZE - 1], &keys),
            Err(NetError::DatagramLength { .. })
        ));
        assert!(matches!(
            unpack(&vec![0u8; MAX_DATAGRAM + 1], &keys),
            Err(NetError::DatagramLength { .. })
        ));
        let mut wrong_size = datagram.clone();
        wrong_size[0] = wrong_size[0].wrapping_add(1);
        assert!(matches!(
            unpack(&wrong_size, &keys),
            Err(NetError::DatagramSize { .. })
        ));

        // A flipped payload byte breaks the CRC (the size field stays intact).
        let mut corrupt = datagram.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0x40;
        assert!(matches!(
            unpack(&corrupt, &keys),
            Err(NetError::DatagramCrc { .. })
        ));

        // A flipped header byte de-obfuscates to a different CRC too.
        let mut corrupt = datagram.clone();
        corrupt[10] ^= 0x08;
        assert!(matches!(
            unpack(&corrupt, &keys),
            Err(NetError::DatagramCrc { .. })
        ));

        // A payload that cannot fit one datagram is refused before anything is written.
        assert!(matches!(
            pack(&header, &vec![0u8; MAX_PAYLOAD + 1], &keys),
            Err(NetError::PayloadTooLarge { .. })
        ));
        assert!(pack(&header, &vec![0u8; MAX_PAYLOAD], &keys).is_ok());
    }

    #[test]
    fn reading_a_header_clears_extra_when_the_flags_do_not_use_it() {
        let keys = Keys::default();
        let header = Header {
            flags: flags::RELIABLE,
            serial: 9,
            ack: 3,
            ack_mask: u64::MAX,
            extra: 0xDEAD_BEEF,
        };
        let datagram = pack(&header, b"", &keys).expect("pack");
        let (parsed, _) = unpack(&datagram, &keys).expect("unpack");
        assert_eq!(parsed.ack_mask, u64::MAX);
        assert_eq!(parsed.extra, 0, "extra is part of the 64-bit mask here");
    }
}
