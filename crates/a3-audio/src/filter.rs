//! A second-order low-pass filter (RBJ biquad).

use std::f32::consts::PI;

/// Low-pass biquad for one channel, transposed direct form II.
#[derive(Debug, Clone, Copy, Default)]
pub struct LowPass {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
    cutoff: f32,
    active: bool,
}

impl LowPass {
    /// Sets the cutoff frequency (Hz) and resonance `q` at a sample rate. Cutoffs at or above
    /// 45% of the sample rate switch the filter off (pass-through).
    pub fn set(&mut self, cutoff_hz: f32, q: f32, sample_rate: f32) {
        if cutoff_hz >= 0.45 * sample_rate || !cutoff_hz.is_finite() {
            if self.active {
                *self = Self::default();
            }
            return;
        }
        if self.active && (cutoff_hz - self.cutoff).abs() < 0.5 {
            return;
        }
        let w0 = 2.0 * PI * cutoff_hz.max(10.0) / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q.max(0.1));
        let a0 = 1.0 + alpha;
        self.b0 = (1.0 - cos) / 2.0 / a0;
        self.b1 = (1.0 - cos) / a0;
        self.b2 = self.b0;
        self.a1 = -2.0 * cos / a0;
        self.a2 = (1.0 - alpha) / a0;
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
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
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
        lp.set(500.0, std::f32::consts::FRAC_1_SQRT_2, rate);
        let low = rms_after(&mut lp, 100.0, rate);
        let mut lp2 = lp;
        let high = rms_after(&mut lp2, 8000.0, rate);
        assert!(
            (low - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.03,
            "{low}"
        );
        assert!(high < 0.01, "{high}");
    }

    #[test]
    fn a_cutoff_near_nyquist_disables_the_filter() {
        let mut lp = LowPass::default();
        lp.set(30_000.0, 1.0, 48_000.0);
        assert!(!lp.is_active());
        assert_eq!(lp.process(0.25), 0.25);
    }
}
