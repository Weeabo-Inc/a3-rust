//! Named user actions and the bindings that trigger them.

use std::collections::HashMap;
use std::fmt;

use crate::binding::{Binding, Trigger};
use crate::state::InputState;

/// Name of a user action, as used by `CfgDefaultKeysPresets`, profiles and the `inputAction`
/// script command (`moveForward`, `defaultAction`, `cameraMoveUp`).
///
/// Names compare case-insensitively, like RV's.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActionName(String);

impl ActionName {
    pub fn new(name: &str) -> ActionName {
        ActionName(name.to_ascii_lowercase())
    }

    /// The normalised (lower-case) name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ActionName {
    fn from(name: &str) -> ActionName {
        ActionName::new(name)
    }
}

impl fmt::Display for ActionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The bindings of every action. Pure data: evaluate it against an [`InputState`].
///
/// Evaluation rules:
/// - An action's value is the largest value of its active level bindings (keys, buttons,
///   stick half-axes) plus the sum of its active relative bindings (mouse motion).
/// - A binding with a modifier is active only while the modifier is held.
/// - A binding without a modifier is suppressed while another binding in the map on the same
///   input has its modifier held, so `LCtrl+X` does not also fire `X` _(uncertain: matches
///   observed RV behaviour, exact rule not reverse engineered)_.
#[derive(Debug, Clone, Default)]
pub struct ActionMap {
    bindings: HashMap<ActionName, Vec<Binding>>,
}

impl ActionMap {
    pub fn new() -> ActionMap {
        ActionMap::default()
    }

    /// Add one binding to `action`.
    pub fn bind(&mut self, action: impl Into<ActionName>, binding: Binding) -> &mut Self {
        self.bindings
            .entry(action.into())
            .or_default()
            .push(binding);
        self
    }

    /// Replace all bindings of `action`.
    pub fn set_bindings(&mut self, action: impl Into<ActionName>, bindings: Vec<Binding>) {
        self.bindings.insert(action.into(), bindings);
    }

    /// Replace all bindings of `action` from RV integer key codes, skipping (and returning)
    /// codes that do not decode.
    pub fn set_rv_codes(
        &mut self,
        action: impl Into<ActionName>,
        codes: &[i64],
    ) -> Vec<crate::RvCodeError> {
        let mut errors = Vec::new();
        let bindings = codes
            .iter()
            .filter_map(|&c| Binding::from_rv_code(c).map_err(|e| errors.push(e)).ok())
            .collect();
        self.set_bindings(action, bindings);
        errors
    }

    /// The bindings of `action` (empty if unknown).
    pub fn bindings(&self, action: &str) -> &[Binding] {
        self.bindings
            .get(&ActionName::new(action))
            .map_or(&[], Vec::as_slice)
    }

    /// All actions with their bindings.
    pub fn iter(&self) -> impl Iterator<Item = (&ActionName, &[Binding])> {
        self.bindings.iter().map(|(k, v)| (k, v.as_slice()))
    }

    /// Current value of `action`: `0` when inactive, `1` for a held key, the stick level for
    /// analog inputs, plus this frame's motion for mouse axes.
    pub fn value(&self, state: &InputState, action: &str) -> f32 {
        let mut level = 0.0f32;
        let mut relative = 0.0f32;
        for b in self.bindings(action) {
            if !self.binding_active(state, b) {
                continue;
            }
            let v = state.value(b.input);
            if b.input.is_relative() {
                relative += v;
            } else {
                level = level.max(v);
            }
        }
        level + relative
    }

    /// Whether any binding of `action` is active.
    pub fn is_active(&self, state: &InputState, action: &str) -> bool {
        self.bindings(action)
            .iter()
            .any(|b| self.binding_active(state, b))
    }

    /// Whether `action` was triggered this frame: a press, completed double tap or reached hold
    /// time since the previous frame.
    pub fn just_triggered(&self, state: &InputState, action: &str) -> bool {
        self.bindings(action).iter().any(|b| {
            if !self.modifier_ok(state, b) {
                return false;
            }
            match b.trigger {
                Trigger::Press if b.input.is_relative() => state.value(b.input) > 0.0,
                Trigger::Press => state.was_pressed(b.input),
                Trigger::DoubleTap => state.was_double_tapped(b.input),
                Trigger::Hold { seconds } => state.held_for(b.input).is_some_and(|held| {
                    let since_frame_start = state.now() - state.frame_start();
                    held >= f64::from(seconds) && held - since_frame_start < f64::from(seconds)
                }),
            }
        })
    }

    fn binding_active(&self, state: &InputState, b: &Binding) -> bool {
        if !self.modifier_ok(state, b) {
            return false;
        }
        match b.trigger {
            Trigger::Press if b.input.is_relative() => state.value(b.input) > 0.0,
            Trigger::Press => state.is_down(b.input) || state.value(b.input) > 0.0,
            Trigger::DoubleTap => state.is_double_tap_down(b.input),
            Trigger::Hold { seconds } => state
                .held_for(b.input)
                .is_some_and(|held| held >= f64::from(seconds)),
        }
    }

    fn modifier_ok(&self, state: &InputState, b: &Binding) -> bool {
        match b.modifier {
            Some(m) => modifier_down(state, m),
            None => !self.shadowed_by_combo(state, b),
        }
    }

    /// Whether some other binding on the same input has its modifier held.
    fn shadowed_by_combo(&self, state: &InputState, b: &Binding) -> bool {
        self.bindings
            .values()
            .flatten()
            .filter(|other| other.input == b.input)
            .filter_map(|other| other.modifier)
            .any(|m| modifier_down(state, m))
    }
}

fn modifier_down(state: &InputState, m: crate::Modifier) -> bool {
    match m {
        crate::Modifier::Input(code) => state.is_down(code),
        other => other.inputs().iter().any(|&code| state.is_down(code)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Modifier;
    use crate::code::{Dik, GamepadInput, InputCode, MouseAxis};

    fn key(d: Dik) -> InputCode {
        InputCode::Key(d)
    }

    #[test]
    fn held_key_activates_its_action() {
        let mut map = ActionMap::new();
        map.bind("moveForward", Binding::key(Dik::W));
        let mut s = InputState::new();
        assert_eq!(map.value(&s, "moveForward"), 0.0);
        s.press(key(Dik::W));
        assert_eq!(map.value(&s, "moveForward"), 1.0);
        assert!(
            map.is_active(&s, "MoveForward"),
            "names are case-insensitive"
        );
    }

    #[test]
    fn two_held_keys_do_not_double_the_value() {
        let mut map = ActionMap::new();
        map.bind("moveForward", Binding::key(Dik::W));
        map.bind("moveForward", Binding::key(Dik::UP));
        let mut s = InputState::new();
        s.press(key(Dik::W));
        s.press(key(Dik::UP));
        assert_eq!(map.value(&s, "moveForward"), 1.0);
    }

    #[test]
    fn mouse_motion_sums_with_key_level() {
        let mut map = ActionMap::new();
        map.bind("turnLeft", Binding::key(Dik::A));
        map.bind(
            "turnLeft",
            Binding::press(InputCode::MouseAxis(MouseAxis::Left)),
        );
        let mut s = InputState::new();
        s.mouse_motion(-5.0, 0.0);
        assert_eq!(map.value(&s, "turnLeft"), 5.0);
        s.press(key(Dik::A));
        assert_eq!(map.value(&s, "turnLeft"), 6.0);
    }

    #[test]
    fn gamepad_stick_gives_analog_value() {
        let mut map = ActionMap::new();
        map.bind(
            "moveForward",
            Binding::press(InputCode::Gamepad(GamepadInput::LeftStickUp)),
        );
        let mut s = InputState::new();
        s.set_analog(InputCode::Gamepad(GamepadInput::LeftStickUp), 0.25);
        assert_eq!(map.value(&s, "moveForward"), 0.25);
    }

    #[test]
    fn combo_needs_its_modifier_and_shadows_the_plain_binding() {
        let mut map = ActionMap::new();
        map.bind(
            "deployWeapon",
            Binding::key(Dik::X).with_modifier(Modifier::Input(key(Dik::LCONTROL))),
        );
        map.bind("crouch", Binding::key(Dik::X));
        let mut s = InputState::new();
        s.press(key(Dik::X));
        assert!(map.is_active(&s, "crouch"));
        assert!(!map.is_active(&s, "deployWeapon"));
        s.press(key(Dik::LCONTROL));
        assert!(map.is_active(&s, "deployWeapon"));
        assert!(!map.is_active(&s, "crouch"));
    }

    #[test]
    fn legacy_any_ctrl_accepts_right_ctrl() {
        let mut map = ActionMap::new();
        map.bind("a", Binding::key(Dik::C).with_modifier(Modifier::AnyCtrl));
        let mut s = InputState::new();
        s.press(key(Dik::RCONTROL));
        s.press(key(Dik::C));
        assert!(map.is_active(&s, "a"));
    }

    #[test]
    fn press_triggers_once_per_press() {
        let mut map = ActionMap::new();
        map.bind("reloadMagazine", Binding::key(Dik::R));
        let mut s = InputState::new();
        s.press(key(Dik::R));
        assert!(map.just_triggered(&s, "reloadMagazine"));
        s.end_frame();
        assert!(!map.just_triggered(&s, "reloadMagazine"));
        assert!(map.is_active(&s, "reloadMagazine"));
    }

    #[test]
    fn double_tap_binding_fires_on_the_second_press_only() {
        let mut map = ActionMap::new();
        map.bind("compassToggle", Binding::double_tap(key(Dik::K)));
        map.bind("compass", Binding::key(Dik::K));
        let mut s = InputState::new();
        s.set_time(0.0);
        s.press(key(Dik::K));
        assert!(map.just_triggered(&s, "compass"));
        assert!(!map.just_triggered(&s, "compassToggle"));
        s.release(key(Dik::K));
        s.end_frame();
        s.set_time(0.2);
        s.press(key(Dik::K));
        assert!(map.just_triggered(&s, "compassToggle"));
        assert!(map.is_active(&s, "compassToggle"));
        s.release(key(Dik::K));
        assert!(!map.is_active(&s, "compassToggle"));
    }

    #[test]
    fn hold_binding_triggers_when_the_hold_time_is_reached() {
        let mut map = ActionMap::new();
        map.bind("optics", Binding::hold(InputCode::MouseButton(1), 0.5));
        let mut s = InputState::new();
        s.set_time(1.0);
        s.press(InputCode::MouseButton(1));
        s.end_frame();
        s.set_time(1.4);
        assert!(!map.is_active(&s, "optics"));
        assert!(!map.just_triggered(&s, "optics"));
        s.end_frame();
        s.set_time(1.6);
        assert!(map.is_active(&s, "optics"));
        assert!(map.just_triggered(&s, "optics"));
        s.end_frame();
        s.set_time(1.8);
        assert!(map.is_active(&s, "optics"));
        assert!(!map.just_triggered(&s, "optics"));
    }

    #[test]
    fn bindings_load_from_rv_codes() {
        let mut map = ActionMap::new();
        let errors = map.set_rv_codes("zoomIn", &[0x4E, 0x0001_0000 + 4, -5]);
        assert_eq!(errors.len(), 1);
        assert_eq!(
            map.bindings("ZOOMIN"),
            &[
                Binding::key(Dik::ADD),
                Binding::press(InputCode::MouseButton(4))
            ]
        );
    }

    #[test]
    fn unknown_action_is_inactive() {
        let map = ActionMap::new();
        let s = InputState::new();
        assert_eq!(map.value(&s, "nothing"), 0.0);
        assert!(!map.just_triggered(&s, "nothing"));
    }
}
