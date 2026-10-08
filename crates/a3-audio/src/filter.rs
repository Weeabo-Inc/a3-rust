//! The voice low-pass: a Chamberlin state-variable filter parameterised like XAudio2's
//! (`Frequency = 2 sin(pi * cutoff / rate)`, `OneOverQ`), which the engine drives for its
//! distance filters (`docs/re/audio.md`).

use std::f32::consts::PI;

/// Low-pass state-variable filter for one channel.
#[derive(Debug, Clone, Copy, Default)]
pub struct LowPass {
    f: f32,
    one_over_q: f32,
    low: f32,
    band: f32,
    cutoff: f32,
    active: bool,
}

impl LowPass {
    /// Sets the cutoff (Hz) and `one_over_q` (the config `qFactor`, passed through as XAudio2
    /// does) at a sample rate. As in the engine, a cutoff at or above a sixth of the sample rate
    /// leaves the filter fully open (pass-through).
    pub fn set(&mut self, cutoff_hz: f32, one_over_q: f32, sample_rate: f32) {
        if !cutoff_hz.is_finite() || cutoff_hz * 6.0 >= sample_rate {
            if self.active {
                *self = Self::default();
            }
            return;
        }
        if self.active && (cutoff_hz - self.cutoff).abs() < 0.5 {
            return;
        }
        self.f = 2.0 * (PI * cutoff_hz.max(1.0) / sample_rate).sin();
        self.one_over_q = one_over_q.clamp(0.05, 1.5);
        self.cutoff = cutoff_hz;
        self.active = true;
    }

    /// Whether the filter changes the signal.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Filters one sample.
    pub fn process(&mut self, x: f32) -> f32 {
        if !self.active {
            return x;
        }
        let high = x - self.low - self.one_over_q * self.band;
        self.band += self.f * high;
        self.low += self.f * self.band;
        self.low
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms_after(filter: &mut LowPass, freq: f32, rate: f32) -> f32 {
        let n = 4800;
        let mut sum = 0.0;
        for i in 0..n {
            let x = (2.0 * PI * freq * i as f32 / rate).sin();
            let y = filter.process(x);
            if i >= n / 2 {
                sum += y * y;
            }
        }
        (sum / (n / 2) as f32).sqrt()
    }

    #[test]
    fn passes_low_and_cuts_high_frequencies() {
        let rate = 48_000.0;
        let mut lp = LowPass::default();
        lp.set(500.0, 1.0, rate);
        let low = rms_after(&mut lp.clone(), 100.0, rate);
        let high = rms_after(&mut lp, 6000.0, rate);
        assert!(
            (low - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.05,
            "{low}"
        );
        assert!(high < 0.01, "{high}");
    }

    #[test]
    fn a_cutoff_above_a_sixth_of_the_rate_disables_the_filter() {
        let mut lp = LowPass::default();
        lp.set(8_000.0, 1.0, 48_000.0);
        assert!(!lp.is_active());
        assert_eq!(lp.process(0.25), 0.25);
        lp.set(7_000.0, 1.0, 48_000.0);
        assert!(lp.is_active());
    }
}
