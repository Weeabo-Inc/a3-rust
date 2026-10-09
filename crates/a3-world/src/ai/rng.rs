//! The engine's random generator (`docs/re/ai-fsm.md` §1.3): one 31-bit linear congruential
//! generator shared by all engine randomness, `seed = (seed * 0xC1C64E6D + 0x3039) & 0x7fffffff`,
//! a draw in `0..1` being `seed * 2^-31`. The World keeps one, seeded at creation, so a run is
//! repeatable.

/// The engine's random generator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineRng {
    seed: u32,
}

/// The seed a World starts with.
pub const DEFAULT_SEED: u32 = 0x1234_5678 & 0x7fff_ffff;

impl Default for EngineRng {
    fn default() -> Self {
        Self::new(DEFAULT_SEED)
    }
}

impl EngineRng {
    pub fn new(seed: u32) -> Self {
        Self {
            seed: seed & 0x7fff_ffff,
        }
    }

    /// The next uniform number in `0..1` (`rng01`).
    pub fn next_unit(&mut self) -> f64 {
        self.seed = self.seed.wrapping_mul(0xC1C6_4E6D).wrapping_add(0x3039) & 0x7fff_ffff;
        f64::from(self.seed) * (1.0 / 2_147_483_648.0)
    }

    /// A uniform number in `centre - spread .. centre + spread` (`rngAround`).
    pub fn around(&mut self, centre: f64, spread: f64) -> f64 {
        centre - spread + 2.0 * spread * self.next_unit()
    }

    /// A random number between `min` and `max` whose median is `mid` (`Rand_MinMidMax`,
    /// `docs/re/ai.md` §3): the mean of four draws, mapped below the middle onto `min..mid`
    /// and above it onto `mid..max`.
    pub fn min_mid_max(&mut self, min: f64, mid: f64, max: f64) -> f64 {
        let u = (self.next_unit() + self.next_unit() + self.next_unit() + self.next_unit()) / 4.0;
        if u < 0.5 {
            min + (mid - min) * 2.0 * u
        } else {
            mid + (max - mid) * (2.0 * u - 1.0)
        }
    }
}
