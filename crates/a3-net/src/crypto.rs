//! The two primitives the fixed obfuscation keys are built from.
//!
//! SHA-1 is CryptoAPI's `CALG_SHA1` in the engine (helpers 0x2810b0/0x281150/0x281180); RC4 is the
//! `0x352fc0` keystream generator used as RC4-drop512 by the header masks and by the Steam blob.

use sha1::{Digest, Sha1};

/// SHA-1 of `data`.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// How many keystream bytes the engine throws away before using RC4 (`RC4-drop512`).
pub const RC4_DROP: usize = 512;

/// Fill `out` with the RC4 keystream of `key` after discarding [`RC4_DROP`] bytes.
///
/// This is textbook RC4: the key schedule ends with the array permuted, and the keystream loop
/// restarts from `i = j = 0` (the key schedule's `j` is a local, as in the published description).
pub fn rc4_drop512(key: &[u8], out: &mut [u8]) {
    assert!(!key.is_empty(), "rc4 needs a non-empty key");
    let mut s = [0u8; 256];
    for (i, slot) in s.iter_mut().enumerate() {
        *slot = i as u8;
    }
    let mut j = 0u8;
    for i in 0..256usize {
        j = j
            .wrapping_add(s[i])
            .wrapping_add(key[i % key.len()]);
        s.swap(i, usize::from(j));
    }
    let (mut i, mut j) = (0u8, 0u8);
    let step = |i: &mut u8, j: &mut u8, s: &mut [u8; 256]| {
        *i = i.wrapping_add(1);
        *j = j.wrapping_add(s[usize::from(*i)]);
        s.swap(usize::from(*i), usize::from(*j));
        s[usize::from(s[usize::from(*i)].wrapping_add(s[usize::from(*j)]))]
    };
    for _ in 0..RC4_DROP {
        step(&mut i, &mut j, &mut s);
    }
    for slot in out.iter_mut() {
        *slot = step(&mut i, &mut j, &mut s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_matches_the_published_vectors() {
        assert_eq!(
            sha1(b""),
            hex("da39a3ee5e6b4b0d3255bfef95601890afd80709")
        );
        assert_eq!(
            sha1(b"abc"),
            hex("a9993e364706816aba3e25717850c26c9cd0d89d")
        );
        assert_eq!(
            sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            hex("84983e441c3bd26ebaae4aa1f95129e5e54670f1")
        );
    }

    #[test]
    fn rc4_drop512_matches_the_reference_keystream() {
        // The reference codec `tools/re/a3net.py rc4_drop` produced these 24 bytes after dropping
        // 512, for the key SHA-1("a3-rust rc4 test"). Independent implementation, same algorithm.
        let mut out = [0u8; 24];
        rc4_drop512(
            &sha1(b"a3-rust rc4 test"),
            &mut out,
        );
        assert_eq!(
            out.to_vec(),
            decode_hex("8765ffdc849c97f9876f80dc8dac41c4f5b557ebf464ff8e")
        );
    }

    /// Parse a hex string of even length into bytes (test helper).
    fn decode_hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("test hex"))
            .collect()
    }

    fn hex(s: &str) -> [u8; 20] {
        let mut out = [0u8; 20];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("test hex");
        }
        out
    }
}
