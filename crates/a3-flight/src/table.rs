//! The speed-indexed coefficient tables of the aircraft configs (`envelope[]`, `thrustCoef[]`,
//! `elevatorCoef[]`, ...): `n` values spread evenly from 0 to a span tied to `maxSpeed`
//! (`docs/re/sim-air.md` §2.5, §3.1, §3.4). Each table has its own span and its own rule past
//! the last entry, so each sampler here is one of the original's.

/// The fractional index of `speed` in a table of `n` entries spanning `0..=span`.
fn index(n: usize, speed: f64, span: f64) -> f64 {
    (n as f64 - 1.0) * speed / span
}

/// Linear interpolation between entries `i` and `i + 1` at fractional index `x`.
fn lerp(values: &[f64], i: usize, x: f64) -> f64 {
    let a = values[i];
    a + (values[i + 1] - a) * (x - i as f64)
}

/// A plane coefficient (`thrustCoef`, `elevatorCoef`, `aileronCoef`, `rudderCoef`,
/// `draconicTorqueXCoef`, ...; `0x140d197c0`): the table spans `0..=1.5·max_speed`; below 0 it
/// is the first entry, past the end the last; an empty table gives `default`. `max_speed` in
/// m/s; a non-positive `max_speed` gives 0.
pub fn plane_coef(values: &[f64], speed: f64, max_speed: f64, default: f64) -> f64 {
    if max_speed <= 0.0 {
        return 0.0;
    }
    if values.is_empty() {
        return default;
    }
    let last = values.len() - 1;
    let x = index(values.len(), speed, 1.5 * max_speed);
    if x < 0.0 {
        return values[0];
    }
    let i = x.floor() as usize;
    if i >= last {
        values[last]
    } else {
        lerp(values, i, x)
    }
}

/// The helicopter lift envelope (`0x140da6b00`): spans `0..=1.4·max_speed` of horizontal speed.
/// Past the end it reads the **second-to-last** entry (the original's indexing).
pub fn heli_envelope(values: &[f64], speed: f64, max_speed: f64) -> f64 {
    let n = values.len();
    if n < 2 || max_speed <= 0.0 {
        return values.first().copied().unwrap_or(0.0);
    }
    let x = index(n, speed, 1.4 * max_speed);
    if x < 0.0 {
        return 0.0;
    }
    let i = x.floor() as usize;
    if i >= n - 1 {
        values[n - 2]
    } else {
        lerp(values, i, x)
    }
}

/// Where a plane's forward speed falls in its lift `envelope` (`0x140d2da40`): the table spans
/// `0..=1.25·max_speed`. `None` below 0; `Some(Err(last))` past the end, where the original
/// returns the last entry without the angle-of-attack factor; `Some(Ok(value))` inside.
pub fn plane_envelope(values: &[f64], speed: f64, max_speed: f64) -> Option<Result<f64, f64>> {
    let n = values.len();
    if n == 0 || max_speed <= 0.0 {
        return None;
    }
    let x = index(n, 0.8 * speed, max_speed);
    if x < 0.0 {
        return None;
    }
    let i = x.floor() as usize;
    if i >= n - 1 {
        Some(Err(values[n - 1]))
    } else {
        Some(Ok(lerp(values, i, x)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plane_coefficient_spans_one_and_a_half_max_speed() {
        // Three entries: 0 at 0, 1 at 0.75·max, 2 at 1.5·max.
        let t = [0.0, 1.0, 2.0];
        assert_eq!(plane_coef(&t, 0.0, 100.0, 9.0), 0.0);
        assert!((plane_coef(&t, 75.0, 100.0, 9.0) - 1.0).abs() < 1e-12);
        assert!((plane_coef(&t, 37.5, 100.0, 9.0) - 0.5).abs() < 1e-12);
        assert_eq!(plane_coef(&t, 500.0, 100.0, 9.0), 2.0);
        assert_eq!(plane_coef(&t, -5.0, 100.0, 9.0), 0.0);
        assert_eq!(plane_coef(&[], 10.0, 100.0, 9.0), 9.0);
    }

    #[test]
    fn the_heli_envelope_spans_one_point_four_max_speed_and_falls_back_past_the_end() {
        let t = [0.0, 1.0, 2.0, 3.0];
        // Index 1 at 1.4·max/3.
        let at = 1.4 * 60.0 / 3.0;
        assert!((heli_envelope(&t, at, 60.0) - 1.0).abs() < 1e-12);
        assert!((heli_envelope(&t, 1.5 * at, 60.0) - 1.5).abs() < 1e-12);
        // Past the end: entry n-2.
        assert_eq!(heli_envelope(&t, 1000.0, 60.0), 2.0);
    }

    #[test]
    fn the_plane_envelope_spans_one_and_a_quarter_max_speed() {
        let t = [0.0, 2.0, 4.0];
        match plane_envelope(&t, 0.625 * 100.0, 100.0) {
            Some(Ok(v)) => assert!((v - 2.0).abs() < 1e-12),
            other => panic!("{other:?}"),
        }
        assert_eq!(plane_envelope(&t, 200.0, 100.0), Some(Err(4.0)));
        assert_eq!(plane_envelope(&t, -1.0, 100.0), None);
    }
}
