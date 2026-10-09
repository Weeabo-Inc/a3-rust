//! The fixed obfuscation keys (`docs/re/net-transport.md`, "Obfuscation (not encryption)").
//!
//! Every key derives from one 32-bit `magic`, which this build passes as the constant
//! [`MAGIC`] at `NetServer::Init` and at the client connect. There is therefore exactly one key
//! set for every server and client of 2.22.0.154103, and the whole scheme is a fixed XOR mask
//! (RC4 re-keyed per datagram, so the mask never advances) plus a table-selected payload XOR.

use crate::crypto::{rc4_drop512, sha1};

/// The `magic` this build hard-codes on both sides (`NetServer::Init` call at 0xc88ff6).
pub const MAGIC: u32 = 0x2525_2525;

/// Length of the payload XOR table (`0x345d90` writes 2048 bytes).
const TABLE_LEN: usize = 0x800;

/// Galois LFSR polynomial mask of the payload table.
const LFSR_MASK: u16 = 0xB400;

/// Seed used when the magic yields an all-zero LFSR state.
const LFSR_FALLBACK: u16 = 0xACE1;

/// `2^-31` as an `f32`, the scale the PRNG applies to its LCG state.
const F32_2_POW_M31: f32 = f32::from_bits(0x3000_0000);

/// The obfuscation keys of one `magic`: two constant header masks and the payload XOR table.
#[derive(Debug, Clone)]
pub struct Keys {
    flags_mask: [u8; 2],
    tail_mask: [u8; 16],
    payload_table: Box<[u8; TABLE_LEN]>,
}

impl Keys {
    /// Derive the key set the `NetPeerUDP` constructor (0x352200) builds.
    pub fn derive(magic: u32) -> Self {
        let m = magic as i32;
        let key1 = header_key(
            (-0x050A_5AA3i32).wrapping_sub(m) as u32,
            m.wrapping_add(0x0012_CA1B) as u32,
            magic,
        );
        let key2 = header_key(
            m.wrapping_add(0x008E_3B8C) as u32,
            (-0x29CD_4BC6i32).wrapping_sub(m) as u32,
            magic,
        );
        let mut flags_mask = [0u8; 2];
        rc4_drop512(&key1, &mut flags_mask);
        let mut tail_mask = [0u8; 16];
        rc4_drop512(&key2, &mut tail_mask);
        Self {
            flags_mask,
            tail_mask,
            payload_table: Box::new(payload_table(magic)),
        }
    }

    /// The mask XORed over header bytes `[2, 4)` (the `flags` field).
    pub fn flags_mask(&self) -> [u8; 2] {
        self.flags_mask
    }

    /// The mask XORed over header bytes `[8, 24)`.
    pub fn tail_mask(&self) -> &[u8; 16] {
        &self.tail_mask
    }

    /// The 2048-byte table the payload XOR indexes.
    pub fn payload_table(&self) -> &[u8; TABLE_LEN] {
        &self.payload_table
    }
}

impl Default for Keys {
    fn default() -> Self {
        Self::derive(MAGIC)
    }
}

/// The payload XOR start index for a plaintext serial (`0x352a20`).
///
/// `u = ((sar(u ^ 0x3D0000, 16) ^ u) * 9); u = ((sar(u, 4) ^ u) * 0x27D4EB2D)`, all in 32-bit
/// wrapping arithmetic, then `0x18 + ((sar(u, 15) ^ u) & 0x7FFF)`. The payload byte at datagram
/// offset `24 + n` is XORed with `table[(start + n) & 0x7FF]`.
pub fn payload_offset(serial: u32) -> u32 {
    let mut u = serial;
    u = (sar(u ^ 0x003D_0000, 16) ^ u).wrapping_mul(9);
    u = (sar(u, 4) ^ u).wrapping_mul(0x27D4_EB2D);
    0x18 + ((sar(u, 15) ^ u) & 0x7FFF)
}

/// Arithmetic shift right of a 32-bit value, as the engine's `sar` on a signed register.
fn sar(value: u32, shift: u32) -> u32 {
    ((value as i32) >> shift) as u32
}

/// `key = SHA1(R(A, B) ‖ le32(magic))`.
fn header_key(a: u32, b: u32, magic: u32) -> [u8; 20] {
    let mut input = [0u8; 36];
    input[..32].copy_from_slice(&prng_block(a, b));
    input[32..].copy_from_slice(&magic.to_le_bytes());
    sha1(&input)
}

/// `R(A, B)` (0x2814a0) without the extra-data variant: 32 bytes from the engine's LCG.
fn prng_block(a: u32, b: u32) -> [u8; 32] {
    let mut state = interleave(b, a);
    let mut out = [0u8; 32];
    for slot in out.iter_mut() {
        state = state.wrapping_mul(0xC1C6_4E6D).wrapping_add(0x3039) & 0x7FFF_FFFF;
        // f = float32(state) * 2^-31; byte = clamp(cvtss2si(f * 255 - 0.5), 0, 254). The hardware
        // conversion rounds to nearest even under the default MXCSR mode, as `round_ties_even`.
        let f = (state as f32) * F32_2_POW_M31;
        let v = (f * 255.0 - 0.5).round_ties_even().clamp(0.0, 254.0);
        *slot = v as u8;
    }
    out
}

/// `0x30dde0`: interleave the low bits of two 32-bit values into the LCG seed.
///
/// The engine's call is `interleave(B, A)` in the document's `R(A, B)` notation, which is why the
/// arguments are named for their position in the word being built rather than for `R`.
fn interleave(x: u32, y: u32) -> u32 {
    let mut x = x as i32;
    let mut y = y as i32;
    let mut acc = (x & 1) * 2;
    x >>= 1;
    acc |= y & 1;
    y >>= 1;
    for _ in 0..3 {
        acc = (acc * 2) | (x & 1);
        x >>= 1;
        acc = (acc * 2) | (y & 1);
        y >>= 1;
    }
    acc = (acc * 2) | (x & 1);
    let tail_bit = x & 2;
    acc = (acc * 2) | (y & 1);
    y >>= 1;
    ((acc << 2) | (y & 1) | tail_bit) as u32
}

/// The 2048-byte payload XOR table (`0x345d90`): a 16-bit Galois LFSR.
fn payload_table(magic: u32) -> [u8; TABLE_LEN] {
    let lo = (magic & 0xFFFF) as u16;
    let hi = (magic >> 16) as u16;
    let mut lfsr = ((lo ^ hi) & 0x5555) ^ hi;
    if lfsr == 0 {
        lfsr = LFSR_FALLBACK;
    }
    let mut table = [0u8; TABLE_LEN];
    for slot in table.iter_mut() {
        // The byte is taken before the step, so the first table entry is (seed >> 1).
        *slot = (lfsr >> 1) as u8;
        lfsr = if lfsr & 1 != 0 {
            (lfsr >> 1) ^ LFSR_MASK
        } else {
            lfsr >> 1
        };
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Key and table values printed by `python tools/re/a3net.py keys` (the reference codec).
    const REF_HEADER_KEY_1: &str = "b1c1c66c642ffb918ff96c2833048e02f1c91ba0";
    const REF_HEADER_KEY_2: &str = "74b04acc32e57f05e87ca93f813eab171ec8cd4a";
    const REF_TABLE_PREFIX: &str =
        "9249a4d269b4daedf67b3d1e0f8743a15028944a259249a45229148ac5e27138";

    fn decode_hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("test hex"))
            .collect()
    }

    #[test]
    fn header_keys_match_the_reference_implementation() {
        let m = MAGIC as i32;
        assert_eq!(
            header_key(
                (-0x050A_5AA3i32).wrapping_sub(m) as u32,
                m.wrapping_add(0x0012_CA1B) as u32,
                MAGIC
            )
            .to_vec(),
            decode_hex(REF_HEADER_KEY_1)
        );
        assert_eq!(
            header_key(
                m.wrapping_add(0x008E_3B8C) as u32,
                (-0x29CD_4BC6i32).wrapping_sub(m) as u32,
                MAGIC
            )
            .to_vec(),
            decode_hex(REF_HEADER_KEY_2)
        );
    }

    #[test]
    fn header_masks_are_the_documented_ones() {
        // docs/re/net-transport.md, "Header masks": the constants every implementation shares.
        let keys = Keys::default();
        assert_eq!(keys.flags_mask(), [0x9b, 0x6d]);
        assert_eq!(
            keys.tail_mask().to_vec(),
            decode_hex("bf2761a1c3e5d4737f19b5995dc020d0")
        );
    }

    #[test]
    fn payload_table_starts_like_the_reference_implementation() {
        let keys = Keys::default();
        assert_eq!(
            keys.payload_table()[..32].to_vec(),
            decode_hex(REF_TABLE_PREFIX)
        );
    }

    #[test]
    fn payload_offsets_match_the_reference_implementation() {
        // `a3net._payload_mask_start(serial)` for the same serials; the doc's `0x18 + ...` form.
        for (serial, expected) in [
            (0u32, 0x4982u32),
            (1, 0x2CB5),
            (1000, 0x716A),
            (0xDEAD_BEEF, 0x55DB),
            (0xFFFF_FFFF, 0x4982),
        ] {
            assert_eq!(payload_offset(serial), expected, "serial {serial}");
        }
    }
}
