//! The engine's random source for the simulation: the 31-bit linear congruential generator behind
//! `Rand_MinMidMax` (`0x14030e020`, state at `0x142165668`; `docs/re/sim-ballistics.md` §4.4).

/// The generator's multiplier and increment (`x = x·0xC1C64E6D + 0x3039`, low 31 bits kept).
const MULTIPLIER: u32 = 0xC1C6_4E6D;
const INCREMENT: u32 = 0x3039;

/// `1 / 2³¹ / 4`: four 31-bit draws summed and scaled to 0..1.
const SUM_SCALE: f32 = 1.164_153_2e-10;

/// The World's random source. Seeded per World ([`crate::World::set_random_seed`]); the original
/// seeds its global generator from the clock, so only the distributions are reproducible against
/// it, not the sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EngineRandom {
    state: u32,
}

impl Default for EngineRandom {
    fn default() -> Self {
        Self::new(0x1234_5678)
    }
}

impl EngineRandom {
    pub(crate) fn new(seed: u32) -> Self {
        Self {
            state: seed & 0x7fff_ffff,
        }
    }

    fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT) & 0x7fff_ffff;
        self.state
    }

    /// A value in `0..1`: one draw over 2³¹.
    pub(crate) fn uniform(&mut self) -> f64 {
        f64::from(self.next()) / f64::from(1u32 << 31)
    }

    /// `Rand_MinMidMax(min, mid, max)`: the mean `f` of four uniform draws (a bell around 0.5)
    /// mapped piecewise linearly so that 0 → `min`, 0.5 → `mid`, 1 → `max`.
    pub(crate) fn min_mid_max(&mut self, min: f64, mid: f64, max: f64) -> f64 {
        let (a, b, c, d) = (self.next(), self.next(), self.next(), self.next());
        // The original sums the four draws as `f32`, in reverse order of drawing.
        let f = f64::from((d as f32 + c as f32 + b as f32 + a as f32) * SUM_SCALE);
        if f < 0.5 {
            (mid - min) * (f + f) + min
        } else {
            ((f + f) - 1.0) * (max - mid) + mid
        }
    }

    /// `Rand_MinMidMax(-d, 0, d)`: the symmetric per-axis spread of the ricochet normal and the
    /// penetration direction.
    pub(crate) fn spread(&mut self, d: f64) -> f64 {
        self.min_mid_max(-d, 0.0, d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_mid_max_stays_in_range_and_centres_on_mid() {
        let mut r = EngineRandom::new(7);
        let mut below = 0;
        for _ in 0..10_000 {
            let v = r.min_mid_max(0.6, 0.9, 1.0);
            assert!((0.6..=1.0).contains(&v), "{v}");
            if v < 0.9 {
                below += 1;
            }
        }
        // `mid` is the median.
        assert!((4_500..5_500).contains(&below), "{below}");
    }

    #[test]
    fn the_generator_is_the_engines_lcg() {
        let mut r = EngineRandom::new(1);
        assert_eq!(
            r.next(),
            (0xC1C6_4E6Du32.wrapping_add(0x3039)) & 0x7fff_ffff
        );
    }
}
