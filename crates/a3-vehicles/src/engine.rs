//! The engine: the torque curve the config gives it and the angular velocity it turns at.
//! `docs/re/sim-vehicles.md` §2 "Engine".

use crate::value;

/// The engine of a vehicle, from `CfgVehicles` (`enginePower`, `maxOmega`, `torqueCurve[]`,
/// the damping rates) — the inputs of PhysX's `PxVehicleEngineData`.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineData {
    /// `enginePower`, kW (`docs/re/sim-vehicles.md`: default 50).
    pub power: f64,
    /// `maxOmega`, rad/s (default 600).
    pub max_omega: f64,
    /// `minOmega`, rad/s: the idle speed the engine holds with the clutch out (default 0).
    pub min_omega: f64,
    /// `engineMOI`, kg·m² (default 1).
    pub moi: f64,
    /// `peakTorque`, N·m. Derived from the power when the config gives none.
    pub peak_torque: f64,
    /// `torqueCurve[]`: the torque multiplier over the normalised engine speed.
    pub torque_curve: TorqueCurve,
    /// The engine's damping torque per rad/s at full throttle (`dampingRateFullThrottle`,
    /// default 0.08).
    pub damping_full_throttle: f64,
    /// As above with the throttle closed and the clutch engaged — engine braking
    /// (`dampingRateZeroThrottleClutchEngaged`, default 2.0).
    pub damping_clutch_engaged: f64,
    /// As above with the throttle closed and the clutch disengaged
    /// (`dampingRateZeroThrottleClutchDisengaged`, default 0.35).
    pub damping_clutch_disengaged: f64,
}

impl Default for EngineData {
    fn default() -> Self {
        Self {
            power: 50.0,
            max_omega: 600.0,
            min_omega: 0.0,
            moi: 1.0,
            peak_torque: peak_torque_of(50.0, 600.0),
            torque_curve: TorqueCurve::default(),
            damping_full_throttle: 0.08,
            damping_clutch_engaged: 2.0,
            damping_clutch_disengaged: 0.35,
        }
    }
}

impl EngineData {
    /// Reads the engine entries of a vehicle config node.
    pub fn from_config(config: &a3_config::ConfigRef<'_>) -> EngineData {
        let power = value::number_or(&config.get("enginePower"), 50.0);
        let max_omega = value::number_or(&config.get("maxOmega"), 600.0);
        let default = EngineData::default();
        EngineData {
            power,
            max_omega,
            min_omega: value::number_or(&config.get("minOmega"), 0.0),
            moi: value::number_or(&config.get("engineMOI"), 1.0),
            peak_torque: value::number(&config.get("peakTorque"))
                .unwrap_or_else(|| peak_torque_of(power, max_omega)),
            torque_curve: TorqueCurve::new(
                value::points(&config.get("torqueCurve")),
                default.torque_curve,
            ),
            damping_full_throttle: value::number_or(
                &config.get("dampingRateFullThrottle"),
                default.damping_full_throttle,
            ),
            damping_clutch_engaged: value::number_or(
                &config.get("dampingRateZeroThrottleClutchEngaged"),
                default.damping_clutch_engaged,
            ),
            damping_clutch_disengaged: value::number_or(
                &config.get("dampingRateZeroThrottleClutchDisengaged"),
                default.damping_clutch_disengaged,
            ),
        }
    }

    /// The torque curve's multiplier at `omega`, held at the ends of the curve.
    pub fn torque_factor(&self, omega: f64) -> f64 {
        self.torque_curve.sample(omega / self.max_omega)
    }

    /// The torque the engine makes at `omega` and `throttle` (0..=1), N·m. The engine stops
    /// making torque at [`Self::max_omega`] — the limiter — and below the idle speed the torque
    /// of the idle speed is held, so it pulls itself up to idle instead of dying.
    pub fn torque(&self, omega: f64, throttle: f64) -> f64 {
        if omega >= self.max_omega {
            return 0.0;
        }
        let speed = omega.max(self.min_omega);
        self.peak_torque * self.torque_factor(speed) * throttle.clamp(0.0, 1.0)
    }

    /// The engine's drag torque at `omega`, N·m, against the direction it turns. The throttle
    /// and whether the clutch is engaged choose the rate (`docs/re/sim-vehicles.md`).
    pub fn damping(&self, throttle: f64, clutch_engaged: bool) -> f64 {
        if throttle > 0.0 {
            self.damping_full_throttle
        } else if clutch_engaged {
            self.damping_clutch_engaged
        } else {
            self.damping_clutch_disengaged
        }
    }

    /// How fast the idle governor drives the engine, per second _(ours; the original's idle
    /// control is not decoded)_. It brings a stalled engine up to `minOmega` in about this long.
    pub const IDLE_TIME: f64 = 0.25;
}

/// The peak torque of an engine that does not give one: the power turned into torque at the
/// maximum engine speed, the original's conversion (`docs/re/sim-vehicles.md` §2).
fn peak_torque_of(power_kw: f64, max_omega: f64) -> f64 {
    if max_omega <= 0.0 {
        return 0.0;
    }
    (power_kw * 7040.2144 / (max_omega * 9.549296) * 1.3558179).max(0.0)
}

/// The engine's torque multiplier over the normalised engine speed, from `torqueCurve[]` —
/// `{{ω/ωmax, T/Tpeak}, …}`, at most 8 points (`docs/re/sim-vehicles.md`). Sampled linearly
/// between the points and held at the ends.
#[derive(Debug, Clone, PartialEq)]
pub struct TorqueCurve {
    points: Vec<(f64, f64)>,
}

impl Default for TorqueCurve {
    fn default() -> Self {
        // The wiki's default curve.
        TorqueCurve {
            points: vec![(0.0, 0.8), (0.33, 1.0), (1.0, 0.8)],
        }
    }
}

impl TorqueCurve {
    /// The most points the original keeps.
    pub const MAX_POINTS: usize = 8;

    /// A curve of `points`, sorted by engine speed: the first [`Self::MAX_POINTS`] kept, y
    /// clamped to at most 1 (the original clamps it, `docs/re/sim-vehicles.md`). An empty or
    /// single-point set falls back to `default`.
    pub fn new(mut points: Vec<(f64, f64)>, default: TorqueCurve) -> TorqueCurve {
        points.retain(|(x, y)| x.is_finite() && y.is_finite());
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        points.truncate(Self::MAX_POINTS);
        for (_, y) in &mut points {
            *y = y.clamp(0.0, 1.0);
        }
        if points.len() < 2 {
            return default;
        }
        TorqueCurve { points }
    }

    /// The multiplier at the normalised engine speed `x`, linear between the points and held at
    /// the ends.
    pub fn sample(&self, x: f64) -> f64 {
        let Some(first) = self.points.first() else {
            return 0.0;
        };
        if x <= first.0 {
            return first.1;
        }
        for pair in self.points.windows(2) {
            let (x0, y0) = pair[0];
            let (x1, y1) = pair[1];
            if x <= x1 {
                let t = if x1 > x0 { (x - x0) / (x1 - x0) } else { 0.0 };
                return y0 + (y1 - y0) * t;
            }
        }
        self.points.last().map_or(0.0, |last| last.1)
    }

    /// The points of the curve.
    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }
}

/// The turning engine: its angular velocity, integrated from the torque it makes and the load
/// the drivetrain takes from it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EngineState {
    /// The engine's angular velocity, rad/s.
    pub omega: f64,
}

impl EngineState {
    /// Turns the engine by `dt`: its own torque at `throttle`, less its damping, less `load` —
    /// the torque the clutch takes out of it (N·m, positive when the drivetrain resists).
    pub fn step(&mut self, data: &EngineData, throttle: f64, load: f64, dt: f64) {
        let drag = data.damping(throttle, load != 0.0) * self.omega;
        let mut torque = data.torque(self.omega, throttle) - drag - load;
        // The idle governor, when the engine would drop below its idle speed. It makes up the
        // engine's own drag too, so a closed throttle settles exactly at the idle speed.
        if self.omega < data.min_omega {
            torque += (data.min_omega - self.omega) * data.moi / EngineData::IDLE_TIME + drag;
        }
        // The limiter holds the engine at `maxOmega`; it never turns faster.
        self.omega = (self.omega + torque / data.moi * dt).clamp(0.0, data.max_omega);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_peak_torque_is_power_over_speed() {
        // 150 kW at 450 rad/s: 333 N·m.
        let torque = peak_torque_of(150.0, 450.0);
        assert!((torque - 333.33).abs() < 0.5, "{torque}");
        assert_eq!(peak_torque_of(0.0, 450.0), 0.0);
        assert_eq!(peak_torque_of(150.0, 0.0), 0.0);
    }

    #[test]
    fn torque_follows_the_curve_and_stops_at_max_omega() {
        let engine = EngineData {
            max_omega: 450.0,
            min_omega: 0.0,
            peak_torque: 425.0,
            torque_curve: TorqueCurve::new(
                vec![(0.0, 0.0), (0.57, 1.0), (1.0, 0.7)],
                TorqueCurve::default(),
            ),
            ..EngineData::default()
        };
        assert_eq!(engine.torque(0.0, 1.0), 0.0);
        assert!((engine.torque(450.0 * 0.57, 1.0) - 425.0).abs() < 1e-6);
        assert!((engine.torque(450.0 * 0.785, 1.0) - 425.0 * 0.85).abs() < 1.0);
        assert_eq!(engine.torque(450.0, 1.0), 0.0);
        assert_eq!(engine.torque(500.0, 1.0), 0.0);
        // A closed throttle makes no torque.
        assert_eq!(engine.torque(200.0, 0.0), 0.0);
    }

    #[test]
    fn torque_curve_holds_at_the_ends_and_clamps_y() {
        let curve = TorqueCurve::new(vec![(0.0, 0.5), (1.0, 1.4)], TorqueCurve::default());
        assert_eq!(curve.points()[1].1, 1.0, "y is clamped to at most 1");
        assert_eq!(curve.sample(-1.0), 0.5);
        assert_eq!(curve.sample(0.5), 0.75);
        assert_eq!(curve.sample(2.0), 1.0);
        // Unsorted input is sorted, and more than 8 points are dropped.
        let many: Vec<(f64, f64)> = (0..12).map(|i| (1.0 - i as f64 / 12.0, 0.5)).collect();
        let curve = TorqueCurve::new(many, TorqueCurve::default());
        assert_eq!(curve.points().len(), TorqueCurve::MAX_POINTS);
        assert!(curve.points()[0].0 < curve.points()[7].0);
        // One point is not a curve.
        assert_eq!(
            TorqueCurve::new(vec![(0.5, 0.5)], TorqueCurve::default()).points(),
            TorqueCurve::default().points()
        );
    }

    #[test]
    fn damping_follows_the_throttle_and_the_clutch() {
        let engine = EngineData::default();
        assert_eq!(engine.damping(0.5, true), 0.08);
        assert_eq!(engine.damping(0.0, true), 2.0);
        assert_eq!(engine.damping(0.0, false), 0.35);
    }

    #[test]
    fn a_loaded_engine_revs_up_and_is_held_at_idle() {
        let engine = EngineData {
            peak_torque: 425.0,
            max_omega: 450.0,
            min_omega: 100.0,
            moi: 1.0,
            torque_curve: TorqueCurve::new(vec![(0.0, 1.0), (1.0, 1.0)], TorqueCurve::default()),
            ..EngineData::default()
        };
        // With the throttle closed the engine settles at its idle speed, not at zero.
        let mut state = EngineState { omega: 0.0 };
        for _ in 0..200 {
            state.step(&engine, 0.0, 0.0, 0.01);
        }
        assert!(
            (state.omega - 100.0).abs() < 5.0,
            "idle at {}, expected 100",
            state.omega
        );
        // Full throttle against no load runs up to the limiter.
        let mut state = EngineState { omega: 100.0 };
        for _ in 0..1000 {
            state.step(&engine, 1.0, 0.0, 0.01);
        }
        assert!(state.omega > 400.0, "{}", state.omega);
        assert!(state.omega <= engine.max_omega + 1.0);
        // A load the engine cannot pull stalls it back toward idle.
        let mut state = EngineState { omega: 400.0 };
        for _ in 0..100 {
            state.step(&engine, 1.0, 1000.0, 0.01);
        }
        assert!(state.omega < 150.0, "{}", state.omega);
    }
}
