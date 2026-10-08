//! Frame timing: variable per-frame delta plus a fixed-step accumulator.
//!
//! See `docs/adr/0002-main-loop.md`. Every frame gets one variable `dt` (RV-style frame-coupled
//! simulation); subsystems that need a stable step (physics) consume `fixed_steps` steps of
//! `fixed_dt` and interpolate with `alpha`.

use std::time::Instant;

/// Tuning of the [`FrameClock`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockConfig {
    /// Longest frame delta passed to the simulation, in seconds. Longer frames (hitches, window
    /// drags, breakpoints) are clamped so the world does not jump.
    pub max_frame_dt: f64,
    /// Step of the fixed-rate tick, in seconds.
    pub fixed_dt: f64,
    /// Most fixed steps run in one frame; time beyond that is dropped to avoid a spiral of
    /// death when a step costs more than it simulates.
    pub max_fixed_steps: u32,
}

impl Default for ClockConfig {
    fn default() -> Self {
        ClockConfig {
            max_frame_dt: 0.25,
            fixed_dt: 1.0 / 60.0,
            max_fixed_steps: 8,
        }
    }
}

/// Timing of one frame, produced by [`FrameClock::tick`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FrameTime {
    /// Frame counter, starting at 0 (RV `diag_frameNo` analogue).
    pub frame: u64,
    /// Wall-clock seconds since the clock started.
    pub real_time: f64,
    /// Unclamped, unscaled wall-clock seconds since the previous frame.
    pub real_dt: f64,
    /// Simulation seconds to advance this frame: clamped, scaled by the time scale, `0` when
    /// paused.
    pub dt: f64,
    /// Total simulation seconds (RV `time` analogue).
    pub sim_time: f64,
    /// Fixed steps to run this frame.
    pub fixed_steps: u32,
    /// Length of one fixed step in simulation seconds.
    pub fixed_dt: f64,
    /// Fraction of a fixed step left in the accumulator, `0..1`, for interpolating fixed-step
    /// state to the frame.
    pub alpha: f64,
}

/// Turns wall-clock instants into [`FrameTime`]s.
#[derive(Debug, Clone)]
pub struct FrameClock {
    config: ClockConfig,
    start: Option<Instant>,
    last: Option<Instant>,
    frame: u64,
    sim_time: f64,
    accumulator: f64,
    time_scale: f64,
    paused: bool,
}

impl FrameClock {
    pub fn new(config: ClockConfig) -> FrameClock {
        FrameClock {
            config,
            start: None,
            last: None,
            frame: 0,
            sim_time: 0.0,
            accumulator: 0.0,
            time_scale: 1.0,
            paused: false,
        }
    }

    pub fn config(&self) -> ClockConfig {
        self.config
    }

    /// Simulation speed multiplier (RV `accTime` analogue); negative values are treated as 0.
    pub fn set_time_scale(&mut self, scale: f64) {
        self.time_scale = scale.max(0.0);
    }

    pub fn time_scale(&self) -> f64 {
        self.time_scale
    }

    /// Stop or resume simulation time; frames keep ticking with `dt == 0`.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Wall-clock seconds since the first tick at `now`.
    pub fn real_seconds(&self, now: Instant) -> f64 {
        self.start
            .map_or(0.0, |s| now.saturating_duration_since(s).as_secs_f64())
    }

    /// Advance to the frame starting at `now`.
    pub fn tick(&mut self, now: Instant) -> FrameTime {
        let start = *self.start.get_or_insert(now);
        let real_dt = self
            .last
            .map_or(0.0, |l| now.saturating_duration_since(l).as_secs_f64());
        self.last = Some(now);

        let dt = if self.paused {
            0.0
        } else {
            real_dt.min(self.config.max_frame_dt) * self.time_scale
        };
        self.sim_time += dt;

        let fixed_dt = self.config.fixed_dt;
        self.accumulator += dt;
        // The epsilon absorbs rounding so that e.g. 2 x 5 ms makes one 10 ms step.
        let mut steps = (self.accumulator / fixed_dt + 1e-9).floor() as u64;
        if steps > u64::from(self.config.max_fixed_steps) {
            steps = u64::from(self.config.max_fixed_steps);
            self.accumulator = fixed_dt * steps as f64;
        }
        self.accumulator = (self.accumulator - fixed_dt * steps as f64).max(0.0);

        let time = FrameTime {
            frame: self.frame,
            real_time: now.saturating_duration_since(start).as_secs_f64(),
            real_dt,
            dt,
            sim_time: self.sim_time,
            fixed_steps: steps as u32,
            fixed_dt,
            alpha: self.accumulator / fixed_dt,
        };
        self.frame += 1;
        time
    }
}

impl Default for FrameClock {
    fn default() -> Self {
        FrameClock::new(ClockConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn clock(fixed_dt: f64) -> FrameClock {
        FrameClock::new(ClockConfig {
            max_frame_dt: 0.25,
            fixed_dt,
            max_fixed_steps: 4,
        })
    }

    #[test]
    fn first_frame_has_zero_dt() {
        let mut c = FrameClock::default();
        let t = c.tick(Instant::now());
        assert_eq!((t.frame, t.dt, t.fixed_steps), (0, 0.0, 0));
    }

    #[test]
    fn dt_follows_wall_clock() {
        let mut c = clock(0.01);
        let t0 = Instant::now();
        c.tick(t0);
        let t = c.tick(t0 + ms(20));
        assert_eq!(t.frame, 1);
        assert!((t.dt - 0.020).abs() < 1e-9);
        assert!((t.sim_time - 0.020).abs() < 1e-9);
        assert!((t.real_time - 0.020).abs() < 1e-9);
    }

    #[test]
    fn fixed_steps_accumulate_across_frames() {
        let mut c = clock(0.010);
        let t0 = Instant::now();
        c.tick(t0);
        let a = c.tick(t0 + ms(15));
        assert_eq!(a.fixed_steps, 1);
        assert!((a.alpha - 0.5).abs() < 1e-6);
        let b = c.tick(t0 + ms(20));
        assert_eq!(b.fixed_steps, 1);
        assert!(b.alpha.abs() < 1e-6);
    }

    #[test]
    fn long_frames_are_clamped() {
        let mut c = clock(0.010);
        let t0 = Instant::now();
        c.tick(t0);
        let t = c.tick(t0 + Duration::from_secs(3));
        assert!((t.real_dt - 3.0).abs() < 1e-9);
        assert!((t.dt - 0.25).abs() < 1e-9);
    }

    #[test]
    fn fixed_steps_are_capped_and_excess_time_dropped() {
        let mut c = clock(0.010);
        let t0 = Instant::now();
        c.tick(t0);
        let t = c.tick(t0 + ms(100));
        assert_eq!(t.fixed_steps, 4);
        assert_eq!(t.alpha, 0.0);
        let next = c.tick(t0 + ms(105));
        assert_eq!(next.fixed_steps, 0);
    }

    #[test]
    fn time_scale_and_pause_affect_simulation_time_only() {
        let mut c = clock(0.010);
        let t0 = Instant::now();
        c.tick(t0);
        c.set_time_scale(2.0);
        let t = c.tick(t0 + ms(10));
        assert!((t.dt - 0.020).abs() < 1e-9);
        c.set_paused(true);
        let p = c.tick(t0 + ms(20));
        assert_eq!(p.dt, 0.0);
        assert!((p.real_dt - 0.010).abs() < 1e-9);
        assert!((p.sim_time - 0.020).abs() < 1e-9);
    }
}
