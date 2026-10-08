//! Keybindings from the merged config (`CfgDefaultKeysPresets`) and the player's profile.
//!
//! A preset's `Mappings` class holds one array per user action. Each element is a key code: an
//! integer, a string expression (`"256+0x25"`), or a two-element `{modifier, key}` combo. The
//! profile stores the player's changes as `key<Action>[] = {codes}` at its top level. See
//! `docs/re/input-keys.md`.
//!
//! Loading overlays an [`ActionMap`]: every action the source mentions gets the source's
//! bindings, while gamepad (XInput) bindings already in the map are kept, because RV defines
//! controller schemes separately from keyboard presets.

use a3_config::{ConfigRef, Value};
use thiserror::Error;

use crate::action::ActionMap;
use crate::binding::Binding;
use crate::code::InputCode;
use crate::expr::eval_key_expression;
use crate::rv_code::RvCodeError;

/// Root class of the keyboard presets.
pub const PRESETS_CLASS: &str = "CfgDefaultKeysPresets";

/// One selectable keyboard preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetInfo {
    /// Class name, e.g. `Arma3Apex`.
    pub name: String,
    /// `displayName` (usually a `$STR_` stringtable key).
    pub display_name: String,
    /// `default = 1`: the preset a new profile starts with.
    pub is_default: bool,
}

/// A key code that could not be turned into a binding.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum KeyValueError {
    #[error(transparent)]
    Expression(#[from] crate::expr::KeyExprError),
    #[error(transparent)]
    Code(#[from] RvCodeError),
    #[error("key code {0} is not an integer")]
    NotInteger(f32),
    #[error("combo arrays need exactly two elements, got {0}")]
    ComboLength(usize),
}

/// A problem with one element of one action's binding array; the rest still loads.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyLoadIssue {
    pub action: String,
    pub error: KeyValueError,
}

/// Errors that stop loading a preset.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PresetError {
    #[error("config has no class {PRESETS_CLASS}")]
    NoPresets,
    #[error("preset {0} not found in {PRESETS_CLASS}")]
    UnknownPreset(String),
    #[error("preset {0} has no Mappings class")]
    NoMappings(String),
}

/// The keyboard presets defined in `root` (the merged config root), in config order.
pub fn presets(root: &ConfigRef<'_>) -> Vec<PresetInfo> {
    let class = root.get(PRESETS_CLASS);
    class
        .entries()
        .into_iter()
        .filter(|p| p.is_class() && p.get("Mappings").is_class())
        .map(|p| PresetInfo {
            name: p.name().to_owned(),
            display_name: p.get("displayName").text(),
            is_default: p.get("default").is_number() && p.get("default").number() != 0.0,
        })
        .collect()
}

/// Name of the preset marked `default = 1`, if any.
pub fn default_preset(root: &ConfigRef<'_>) -> Option<String> {
    presets(root)
        .into_iter()
        .find(|p| p.is_default)
        .map(|p| p.name)
}

/// Decode one element of a binding array.
pub fn binding_from_value(value: &Value) -> Result<Binding, KeyValueError> {
    match value {
        Value::Array(items) => match items.as_slice() {
            [modifier, key] => Ok(Binding::from_rv_combo(code(modifier)?, code(key)?)?),
            other => Err(KeyValueError::ComboLength(other.len())),
        },
        single => Ok(Binding::from_rv_code(code(single)?)?),
    }
}

fn code(value: &Value) -> Result<i64, KeyValueError> {
    match value {
        Value::Int(i) => Ok(i64::from(*i)),
        Value::Int64(i) => Ok(*i),
        Value::Float(f) if f.fract() == 0.0 => Ok(*f as i64),
        Value::Float(f) => Err(KeyValueError::NotInteger(*f)),
        Value::String(s) | Value::Expression(s) => Ok(eval_key_expression(s)?),
        Value::Array(items) => Err(KeyValueError::ComboLength(items.len())),
    }
}

impl ActionMap {
    /// Overlay the bindings of preset `name` from the merged config `root`, including mappings
    /// inherited from its base presets.
    pub fn load_preset(
        &mut self,
        root: &ConfigRef<'_>,
        name: &str,
    ) -> Result<Vec<KeyLoadIssue>, PresetError> {
        let class = root.get(PRESETS_CLASS);
        if !class.is_class() {
            return Err(PresetError::NoPresets);
        }
        let preset = class.get(name);
        if !preset.is_class() {
            return Err(PresetError::UnknownPreset(name.to_owned()));
        }
        let mappings = preset.get("Mappings");
        if !mappings.is_class() {
            return Err(PresetError::NoMappings(name.to_owned()));
        }
        let mut issues = Vec::new();
        for entry in mappings.entries_with_inherited() {
            if entry.is_array() {
                self.overlay(entry.name(), &entry.array(), &mut issues);
            }
        }
        Ok(issues)
    }

    /// Overlay the player's keybindings from a parsed profile (`key<Action>[] = {...}` entries
    /// at the profile's top level).
    pub fn apply_profile(&mut self, profile: &ConfigRef<'_>) -> Vec<KeyLoadIssue> {
        let mut issues = Vec::new();
        for entry in profile.entries() {
            let name = entry.name();
            let action = match name.get(..3) {
                Some(prefix) if prefix.eq_ignore_ascii_case("key") && name.len() > 3 => &name[3..],
                _ => continue,
            };
            if entry.is_array() {
                self.overlay(action, &entry.array(), &mut issues);
            }
        }
        issues
    }

    fn overlay(&mut self, action: &str, values: &[Value], issues: &mut Vec<KeyLoadIssue>) {
        let mut bindings: Vec<Binding> = self
            .bindings(action)
            .iter()
            .filter(|b| matches!(b.input, InputCode::Gamepad(_)))
            .copied()
            .collect();
        for value in values {
            match binding_from_value(value) {
                Ok(b) if bindings.contains(&b) => {}
                Ok(b) => bindings.push(b),
                Err(error) => issues.push(KeyLoadIssue {
                    action: action.to_owned(),
                    error,
                }),
            }
        }
        self.set_bindings(action, bindings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dik, GamepadInput, InputState, Modifier, MouseAxis, Trigger};
    use a3_config::{ConfigTree, parse_text};

    fn tree(src: &str) -> ConfigTree {
        ConfigTree::from_config(&parse_text(src).expect("valid config"))
    }

    const PRESETS: &str = r#"
        class CfgDefaultKeysPresets {
            class Arma2 {
                displayName = "$STR_A3_CfgDefaultKeysPresets_Arma20";
                default = 0;
                class Mappings {
                    moveForward[] = {17, 200};
                    compassToggle[] = {"256+0x25"};
                    minimapToggle[] = {{157, 50}};
                    fire[] = {{29, 65536}};
                    prevAction[] = {"(0x00100000 +4)", 26};
                    optics[] = {"0x00010000 +128+1", 82};
                    reloadMagazine[] = {19};
                };
            };
            class Arma3: Arma2 {
                displayName = "$STR_A3_CfgDefaultKeysPresets_Arma30";
                default = 1;
                class Mappings: Mappings {
                    reloadMagazine[] = {"256+0x13"};
                    broken[] = {"DIK_W", 31, {1, 2, 3}};
                };
            };
            class Joystick1 { name = "stick"; };
        };
    "#;

    #[test]
    fn lists_presets_and_finds_the_default() {
        let t = tree(PRESETS);
        let list = presets(&t.root());
        let names: Vec<_> = list.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Arma2", "Arma3"]);
        assert_eq!(default_preset(&t.root()).as_deref(), Some("Arma3"));
        assert_eq!(list[0].display_name, "$STR_A3_CfgDefaultKeysPresets_Arma20");
    }

    #[test]
    fn preset_bindings_decode_codes_expressions_and_combos() {
        let t = tree(PRESETS);
        let mut map = ActionMap::new();
        let issues = map.load_preset(&t.root(), "Arma2").unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(
            map.bindings("moveForward"),
            &[Binding::key(Dik::W), Binding::key(Dik::UP)]
        );
        assert_eq!(
            map.bindings("compassToggle"),
            &[Binding::double_tap(InputCode::Key(Dik::K))]
        );
        assert_eq!(
            map.bindings("minimapToggle"),
            &[Binding::key(Dik::M).with_modifier(Modifier::Input(InputCode::Key(Dik::RCONTROL)))]
        );
        assert_eq!(
            map.bindings("fire"),
            &[Binding::press(InputCode::MouseButton(0))
                .with_modifier(Modifier::Input(InputCode::Key(Dik::LCONTROL)))]
        );
        assert_eq!(
            map.bindings("prevAction"),
            &[
                Binding::press(InputCode::MouseAxis(MouseAxis::WheelUp)),
                Binding::key(Dik::LBRACKET)
            ]
        );
        assert_eq!(map.bindings("optics")[0].trigger, Trigger::DoubleTap);
    }

    #[test]
    fn derived_preset_inherits_and_overrides_mappings() {
        let t = tree(PRESETS);
        let mut map = ActionMap::new();
        let issues = map.load_preset(&t.root(), "Arma3").unwrap();
        assert_eq!(
            map.bindings("reloadMagazine"),
            &[Binding::double_tap(InputCode::Key(Dik::R))]
        );
        assert_eq!(map.bindings("moveForward").len(), 2, "inherited from Arma2");
        // The broken entry keeps its good element and reports the two bad ones.
        assert_eq!(map.bindings("broken"), &[Binding::key(Dik::S)]);
        assert_eq!(issues.len(), 2, "{issues:?}");
        assert!(issues.iter().all(|i| i.action == "broken"));
    }

    #[test]
    fn unknown_presets_are_errors() {
        let t = tree(PRESETS);
        let mut map = ActionMap::new();
        assert_eq!(
            map.load_preset(&t.root(), "Nope"),
            Err(PresetError::UnknownPreset("Nope".into()))
        );
        assert_eq!(
            map.load_preset(&t.root(), "Joystick1"),
            Err(PresetError::NoMappings("Joystick1".into()))
        );
        let empty = tree("class Other {};");
        assert_eq!(
            map.load_preset(&empty.root(), "Arma3"),
            Err(PresetError::NoPresets)
        );
    }

    #[test]
    fn profile_overrides_preset_and_decodes_packed_combos() {
        let t = tree(PRESETS);
        let mut map = ActionMap::new();
        map.load_preset(&t.root(), "Arma3").unwrap();
        // LCtrl+X packed as (0x1D << 24) | 0x2D = 486539309.
        let profile = tree(
            "version = 1; keyMoveForward[] = {72}; keyReloadMagazine[] = {486539309}; \
             difficulty = \"Regular\"; keyNothing = 3;",
        );
        assert!(map.apply_profile(&profile.root()).is_empty());
        assert_eq!(map.bindings("moveForward"), &[Binding::key(Dik::NUMPAD8)]);
        assert_eq!(
            map.bindings("reloadMagazine"),
            &[Binding::key(Dik::X).with_modifier(Modifier::Input(InputCode::Key(Dik::LCONTROL)))]
        );
        let mut state = InputState::new();
        state.press(InputCode::Key(Dik::NUMPAD8));
        assert!(map.is_active(&state, "MOVEFORWARD"));
    }

    #[test]
    fn gamepad_bindings_survive_a_keyboard_preset() {
        let t = tree(PRESETS);
        let mut map = ActionMap::new();
        let stick = Binding::press(InputCode::Gamepad(GamepadInput::LeftStickUp));
        map.bind("moveForward", Binding::key(Dik::I));
        map.bind("moveForward", stick);
        map.load_preset(&t.root(), "Arma2").unwrap();
        assert_eq!(
            map.bindings("moveForward"),
            &[stick, Binding::key(Dik::W), Binding::key(Dik::UP)]
        );
    }
}
