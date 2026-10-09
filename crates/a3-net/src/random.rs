//! The pseudo-random numbers the protocol needs where the value is ours to choose.
//!
//! Only two things need randomness: the connect challenge and the A2S challenge. The engine draws
//! the former from MSVC's `rand()` seeded with `GetTickCount() ^ 0x55555555` (see
//! `docs/re/net-handshake.md`), but the document is explicit that the value is arbitrary: the
//! client echoes whatever the server sent, so any u32 works. Reproducing `rand()`'s sequence would
//! buy nothing, so this is a plain SplitMix64 seeded from the system clock.

use std::time::{SystemTime, UNIX_EPOCH};

/// A small, seedable generator: SplitMix64 (public domain algorithm, not the engine's).
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Seed from the system clock mixed with an address, so two servers started in the same
    /// millisecond still diverge.
    pub fn from_clock(salt: u64) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        Self::new(nanos ^ salt.rotate_left(17))
    }

    /// Seed explicitly, which is what tests do.
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }

    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// The next 32 bits.
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// A serial a channel may start on: the peer adopts the first one in `2..1_000_000`.
    pub fn next_serial(&mut self) -> u32 {
        use crate::channel::START_SERIAL_RANGE;
        2 + (self.next_u32() % (START_SERIAL_RANGE.end - START_SERIAL_RANGE.start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::START_SERIAL_RANGE;

    #[test]
    fn sequences_are_reproducible_from_a_seed_and_diverge_between_seeds() {
        let mut a = SplitMix64::new(1);
        let mut b = SplitMix64::new(1);
        let mut c = SplitMix64::new(2);
        let first: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let second: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        assert_eq!(first, second);
        assert_ne!(first[0], c.next_u64());
    }

    #[test]
    fn serials_land_inside_the_documented_start_range() {
        let mut rng = SplitMix64::new(7);
        for _ in 0..10_000 {
            assert!(START_SERIAL_RANGE.contains(&rng.next_serial()));
        }
    }
}
