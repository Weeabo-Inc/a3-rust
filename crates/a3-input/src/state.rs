//! Raw per-input state, fed by the platform layer one event at a time.

use std::collections::HashMap;

use crate::code::{InputCode, MouseAxis};

/// Timing thresholds for input interpretation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputTiming {
    /// Maximum time in seconds between two presses that still counts as a double tap
    /// _(uncertain: RV's exact window is not yet reverse engineered)_.
    pub double_tap_window: f64,
    /// Level at which an analog half-axis (stick, trigger) counts as "down" for press, double
    /// tap and hold triggers.
    pub analog_press_threshold: f32,
}

impl Default for InputTiming {
    fn default() -> Self {
        InputTiming {
            double_tap_window: 0.3,
            analog_press_threshold: 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Record {
    /// Current level, `0..=1` for buttons and absolute axes.
    value: f32,
    down: bool,
    /// Went down at least once since the last `end_frame`.
    pressed: bool,
    /// Went up at least once since the last `end_frame`.
    released: bool,
    /// The most recent press completed a double tap; stays set until release.
    double_tap: bool,
    /// The double tap happened since the last `end_frame`.
    double_tapped_now: bool,
    down_since: f64,
    last_press: Option<f64>,
}

/// The state of every physical input, plus per-frame edges and relative motion.
///
/// Feed events with [`press`](Self::press), [`release`](Self::release),
/// [`set_analog`](Self::set_analog) and [`add_motion`](Self::add_motion) after setting the
/// event time with [`set_time`](Self::set_time); call [`end_frame`](Self::end_frame) after the
/// frame has read its input.
#[derive(Debug, Clone, Default)]
pub struct InputState {
    timing: InputTiming,
    now: f64,
    frame_start: f64,
    records: HashMap<InputCode, Record>,
    motion: HashMap<MouseAxis, f32>,
}

impl InputState {
    /// Empty state with default timing.
    pub fn new() -> InputState {
        InputState::default()
    }

    /// Empty state with the given timing thresholds.
    pub fn with_timing(timing: InputTiming) -> InputState {
        InputState {
            timing,
            ..InputState::default()
        }
    }

    pub fn timing(&self) -> InputTiming {
        self.timing
    }

    /// Set the current time in seconds (monotonic); applies to the following events and queries.
    pub fn set_time(&mut self, now: f64) {
        self.now = now;
    }

    /// Current time in seconds as last set.
    pub fn now(&self) -> f64 {
        self.now
    }

    /// Time at which the current frame began (the last `end_frame`).
    pub fn frame_start(&self) -> f64 {
        self.frame_start
    }

    /// A digital input went down. Repeated presses without a release are ignored (key repeat).
    pub fn press(&mut self, code: InputCode) {
        self.set_level(code, 1.0, true);
    }

    /// A digital input went up.
    pub fn release(&mut self, code: InputCode) {
        self.set_level(code, 0.0, false);
    }

    /// An absolute analog input (gamepad stick half-axis, trigger) changed to `value` in `0..=1`.
    pub fn set_analog(&mut self, code: InputCode, value: f32) {
        let value = value.clamp(0.0, 1.0);
        let down = value >= self.timing.analog_press_threshold;
        self.set_level(code, value, down);
    }

    /// Relative motion along one mouse half-axis (non-negative amount, e.g. pixels or wheel
    /// lines). Summed until `end_frame`.
    pub fn add_motion(&mut self, axis: MouseAxis, amount: f32) {
        if amount > 0.0 {
            *self.motion.entry(axis).or_default() += amount;
        }
    }

    /// Mouse motion by `(dx, dy)`, screen convention (positive y is down).
    pub fn mouse_motion(&mut self, dx: f32, dy: f32) {
        self.add_motion(MouseAxis::Right, dx.max(0.0));
        self.add_motion(MouseAxis::Left, (-dx).max(0.0));
        self.add_motion(MouseAxis::Down, dy.max(0.0));
        self.add_motion(MouseAxis::Up, (-dy).max(0.0));
    }

    /// Vertical wheel motion in lines (positive is away from the user).
    pub fn wheel(&mut self, lines: f32) {
        self.add_motion(MouseAxis::WheelUp, lines.max(0.0));
        self.add_motion(MouseAxis::WheelDown, (-lines).max(0.0));
    }

    /// Release every input, e.g. when the window loses focus.
    pub fn release_all(&mut self) {
        let down: Vec<_> = self
            .records
            .iter()
            .filter(|(_, r)| r.down || r.value != 0.0)
            .map(|(code, _)| *code)
            .collect();
        for code in down {
            self.release(code);
        }
    }

    /// Clear per-frame edges and relative motion; the next frame starts now.
    pub fn end_frame(&mut self) {
        for r in self.records.values_mut() {
            r.pressed = false;
            r.released = false;
            r.double_tapped_now = false;
        }
        self.motion.clear();
        self.frame_start = self.now;
    }

    /// Whether `code` is currently down.
    pub fn is_down(&self, code: InputCode) -> bool {
        self.records.get(&code).is_some_and(|r| r.down)
    }

    /// Whether `code` went down during this frame (even if already released again).
    pub fn was_pressed(&self, code: InputCode) -> bool {
        self.records.get(&code).is_some_and(|r| r.pressed)
    }

    /// Whether `code` went up during this frame.
    pub fn was_released(&self, code: InputCode) -> bool {
        self.records.get(&code).is_some_and(|r| r.released)
    }

    /// Whether the current press of `code` is the second press of a double tap.
    pub fn is_double_tap_down(&self, code: InputCode) -> bool {
        self.records.get(&code).is_some_and(|r| r.double_tap)
    }

    /// Whether a double tap of `code` completed during this frame.
    pub fn was_double_tapped(&self, code: InputCode) -> bool {
        self.records.get(&code).is_some_and(|r| r.double_tapped_now)
    }

    /// How long `code` has been down, or `None` if it is up.
    pub fn held_for(&self, code: InputCode) -> Option<f64> {
        self.records
            .get(&code)
            .filter(|r| r.down)
            .map(|r| self.now - r.down_since)
    }

    /// Current value of `code`: `0..=1` level for buttons and absolute axes, motion summed over
    /// this frame for mouse axes.
    pub fn value(&self, code: InputCode) -> f32 {
        match code {
            InputCode::MouseAxis(axis) => self.motion.get(&axis).copied().unwrap_or(0.0),
            _ => self.records.get(&code).map_or(0.0, |r| r.value),
        }
    }

    fn set_level(&mut self, code: InputCode, value: f32, down: bool) {
        let now = self.now;
        let window = self.timing.double_tap_window;
        let r = self.records.entry(code).or_default();
        r.value = value;
        if down && !r.down {
            r.down = true;
            r.pressed = true;
            r.down_since = now;
            let double = r.last_press.is_some_and(|t| now - t <= window);
            r.double_tap = double;
            r.double_tapped_now |= double;
            // A completed double tap does not start the next one.
            r.last_press = if double { None } else { Some(now) };
        } else if !down && r.down {
            r.down = false;
            r.released = true;
            r.double_tap = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::Dik;

    const W: InputCode = InputCode::Key(Dik::W);

    #[test]
    fn press_and_release_produce_edges_for_one_frame() {
        let mut s = InputState::new();
        s.press(W);
        assert!(s.is_down(W) && s.was_pressed(W));
        s.end_frame();
        assert!(s.is_down(W) && !s.was_pressed(W));
        s.release(W);
        assert!(!s.is_down(W) && s.was_released(W));
        s.end_frame();
        assert!(!s.was_released(W));
    }

    #[test]
    fn tap_within_one_frame_still_registers_as_pressed() {
        let mut s = InputState::new();
        s.press(W);
        s.release(W);
        assert!(s.was_pressed(W) && s.was_released(W) && !s.is_down(W));
    }

    #[test]
    fn key_repeat_does_not_restart_the_hold() {
        let mut s = InputState::new();
        s.set_time(1.0);
        s.press(W);
        s.set_time(1.5);
        s.press(W);
        assert_eq!(s.held_for(W), Some(0.5));
    }

    #[test]
    fn two_quick_presses_are_a_double_tap() {
        let mut s = InputState::new();
        s.set_time(1.0);
        s.press(W);
        s.set_time(1.1);
        s.release(W);
        s.end_frame();
        assert!(!s.was_double_tapped(W));
        s.set_time(1.2);
        s.press(W);
        assert!(s.was_double_tapped(W) && s.is_double_tap_down(W));
        s.end_frame();
        assert!(!s.was_double_tapped(W) && s.is_double_tap_down(W));
        s.release(W);
        assert!(!s.is_double_tap_down(W));
    }

    #[test]
    fn slow_presses_and_a_third_quick_press_are_not_double_taps() {
        let mut s = InputState::new();
        s.set_time(1.0);
        s.press(W);
        s.release(W);
        s.set_time(1.5);
        s.press(W);
        assert!(!s.was_double_tapped(W));
        s.release(W);
        s.set_time(1.6);
        s.press(W);
        assert!(s.was_double_tapped(W));
        s.release(W);
        s.end_frame();
        s.set_time(1.7);
        s.press(W);
        assert!(!s.was_double_tapped(W), "a triple tap is one double tap");
    }

    #[test]
    fn mouse_motion_splits_into_half_axes_and_resets_each_frame() {
        let mut s = InputState::new();
        s.mouse_motion(3.0, -2.0);
        s.mouse_motion(1.0, 0.0);
        s.wheel(-1.0);
        assert_eq!(s.value(InputCode::MouseAxis(MouseAxis::Right)), 4.0);
        assert_eq!(s.value(InputCode::MouseAxis(MouseAxis::Left)), 0.0);
        assert_eq!(s.value(InputCode::MouseAxis(MouseAxis::Up)), 2.0);
        assert_eq!(s.value(InputCode::MouseAxis(MouseAxis::WheelDown)), 1.0);
        s.end_frame();
        assert_eq!(s.value(InputCode::MouseAxis(MouseAxis::Right)), 0.0);
    }

    #[test]
    fn analog_input_counts_as_down_past_the_threshold() {
        use crate::code::GamepadInput;
        let lt = InputCode::Gamepad(GamepadInput::LeftTrigger);
        let mut s = InputState::new();
        s.set_analog(lt, 0.3);
        assert_eq!(s.value(lt), 0.3);
        assert!(!s.is_down(lt));
        s.set_analog(lt, 0.8);
        assert!(s.is_down(lt) && s.was_pressed(lt));
    }

    #[test]
    fn release_all_lifts_every_input() {
        let mut s = InputState::new();
        s.press(W);
        s.press(InputCode::MouseButton(0));
        s.release_all();
        assert!(!s.is_down(W) && !s.is_down(InputCode::MouseButton(0)));
    }
}
