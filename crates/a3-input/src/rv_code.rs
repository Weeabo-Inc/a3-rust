//! Decoding of RV integer key codes as stored in `CfgDefaultKeysPresets` and profiles.
//!
//! See `docs/re/input-keys.md` for the encoding and how sure we are of each part.
//!
//! Layout of a code (a 32-bit integer):
//!
//! | bits   | meaning                                                                  |
//! | ------ | ------------------------------------------------------------------------ |
//! | 24..32 | DIK of a combo modifier key (`LCtrl+X` = `0x1D << 24 \| 0x2D`)          |
//! | 16..24 | device: 0 keyboard, 1 mouse button, 2 joystick button, 4 joystick POV,   |
//! |        | 5 XInput, 0x10 mouse axis                                                |
//! | 0..16  | index within the device; for keyboard: DIK in bits 0..8, `0x100` double  |
//! |        | tap, `0x200`/`0x400`/`0x800` legacy Ctrl/Shift/Alt; for mouse buttons:   |
//! |        | `0x80` double click                                                      |

use thiserror::Error;

use crate::binding::{Binding, Modifier, Trigger};
use crate::code::{Dik, GamepadInput, InputCode, MouseAxis};

const DEVICE_KEYBOARD: u8 = 0x00;
const DEVICE_MOUSE_BUTTON: u8 = 0x01;
const DEVICE_JOYSTICK_BUTTON: u8 = 0x02;
const DEVICE_JOYSTICK_POV: u8 = 0x04;
const DEVICE_XINPUT: u8 = 0x05;
const DEVICE_MOUSE_AXIS: u8 = 0x10;

const KEY_DOUBLE_TAP: u16 = 0x100;
const KEY_CTRL: u16 = 0x200;
const KEY_SHIFT: u16 = 0x400;
const KEY_ALT: u16 = 0x800;
const MOUSE_DOUBLE_CLICK: u16 = 0x80;

const MOUSE_AXES: [MouseAxis; 6] = [
    MouseAxis::Left,
    MouseAxis::Right,
    MouseAxis::Up,
    MouseAxis::Down,
    MouseAxis::WheelUp,
    MouseAxis::WheelDown,
];

/// An RV key code that does not describe a binding.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RvCodeError {
    #[error("key code {0} is outside the 32-bit range")]
    OutOfRange(i64),
    #[error("key code 0x{0:08X} has an unknown index for its device")]
    UnknownIndex(u32),
    #[error("key code 0x{0:08X} combines a combo modifier with legacy modifier flags")]
    ConflictingModifiers(u32),
}

impl Binding {
    /// Decode one RV integer key code (an element of a `CfgDefaultKeysPresets` mapping array or
    /// a profile `key*[]` array).
    pub fn from_rv_code(code: i64) -> Result<Binding, RvCodeError> {
        let code = u32::try_from(code).map_err(|_| RvCodeError::OutOfRange(code))?;
        let combo_key = (code >> 24) as u8;
        let device = (code >> 16) as u8;
        let index = code as u16;

        let mut binding = match device {
            DEVICE_KEYBOARD => decode_keyboard(code, index)?,
            DEVICE_MOUSE_BUTTON => {
                let input = InputCode::MouseButton((index & !MOUSE_DOUBLE_CLICK) as u8);
                if index & !MOUSE_DOUBLE_CLICK > 0xFF {
                    return Err(RvCodeError::UnknownIndex(code));
                }
                if index & MOUSE_DOUBLE_CLICK != 0 {
                    Binding::double_tap(input)
                } else {
                    Binding::press(input)
                }
            }
            DEVICE_JOYSTICK_BUTTON => {
                Binding::press(InputCode::JoystickButton(small(code, index)?))
            }
            DEVICE_JOYSTICK_POV => Binding::press(InputCode::JoystickPov(small(code, index)?)),
            DEVICE_XINPUT => {
                let pad = GamepadInput::from_rv_index(small(code, index)?)
                    .ok_or(RvCodeError::UnknownIndex(code))?;
                Binding::press(InputCode::Gamepad(pad))
            }
            DEVICE_MOUSE_AXIS => {
                let axis = MOUSE_AXES
                    .get(usize::from(index))
                    .ok_or(RvCodeError::UnknownIndex(code))?;
                Binding::press(InputCode::MouseAxis(*axis))
            }
            other => Binding::press(InputCode::Other {
                device: other,
                index,
            }),
        };

        if combo_key != 0 {
            if binding.modifier.is_some() {
                return Err(RvCodeError::ConflictingModifiers(code));
            }
            binding.modifier = Some(Modifier::Input(InputCode::Key(Dik(combo_key))));
        }
        Ok(binding)
    }

    /// Decode a two-element combo as written in `CfgDefaultKeysPresets` (`{modifier, key}`,
    /// e.g. `{0x9D, 0x32}` for `RCtrl+M`).
    pub fn from_rv_combo(modifier: i64, key: i64) -> Result<Binding, RvCodeError> {
        let modifier_binding = Binding::from_rv_code(modifier)?;
        let key_binding = Binding::from_rv_code(key)?;
        if key_binding.modifier.is_some() || modifier_binding.modifier.is_some() {
            // Both codes passed the range check above.
            return Err(RvCodeError::ConflictingModifiers(key as u32));
        }
        Ok(key_binding.with_modifier(Modifier::Input(modifier_binding.input)))
    }

    /// Encode this binding as an RV integer key code, or `None` if RV cannot express it
    /// (hold triggers, double taps on devices other than keyboard and mouse buttons, combo
    /// modifiers that are not keyboard keys).
    pub fn to_rv_code(&self) -> Option<i64> {
        let double = self.trigger == Trigger::DoubleTap;
        if matches!(self.trigger, Trigger::Hold { .. }) {
            return None;
        }
        let (device, mut index): (u8, u16) = match self.input {
            InputCode::Key(dik) => (DEVICE_KEYBOARD, u16::from(dik.0)),
            InputCode::MouseButton(n) if n < 0x80 => (DEVICE_MOUSE_BUTTON, u16::from(n)),
            InputCode::MouseButton(_) => return None,
            InputCode::JoystickButton(n) => (DEVICE_JOYSTICK_BUTTON, u16::from(n)),
            InputCode::JoystickPov(n) => (DEVICE_JOYSTICK_POV, u16::from(n)),
            InputCode::Gamepad(g) => (DEVICE_XINPUT, u16::from(g.rv_index())),
            InputCode::MouseAxis(axis) => {
                let i = MOUSE_AXES.iter().position(|&a| a == axis)? as u16;
                (DEVICE_MOUSE_AXIS, i)
            }
            InputCode::Other { device, index } => (device, index),
        };
        if double {
            index |= match device {
                DEVICE_KEYBOARD => KEY_DOUBLE_TAP,
                DEVICE_MOUSE_BUTTON => MOUSE_DOUBLE_CLICK,
                _ => return None,
            };
        }
        let mut combo_key = 0u8;
        match self.modifier {
            None => {}
            Some(Modifier::Input(InputCode::Key(dik))) => combo_key = dik.0,
            Some(Modifier::Input(_)) => return None,
            Some(legacy) if device == DEVICE_KEYBOARD => {
                index |= match legacy {
                    Modifier::AnyCtrl => KEY_CTRL,
                    Modifier::AnyShift => KEY_SHIFT,
                    _ => KEY_ALT,
                }
            }
            Some(_) => return None,
        }
        Some(i64::from(
            (u32::from(combo_key) << 24) | (u32::from(device) << 16) | u32::from(index),
        ))
    }
}

fn decode_keyboard(code: u32, index: u16) -> Result<Binding, RvCodeError> {
    if index & !0x0FFF != 0 {
        return Err(RvCodeError::UnknownIndex(code));
    }
    let input = InputCode::Key(Dik(index as u8));
    let mut binding = if index & KEY_DOUBLE_TAP != 0 {
        Binding::double_tap(input)
    } else {
        Binding::press(input)
    };
    let legacy = [
        (KEY_CTRL, Modifier::AnyCtrl),
        (KEY_SHIFT, Modifier::AnyShift),
        (KEY_ALT, Modifier::AnyAlt),
    ];
    let mut flags = legacy.iter().filter(|(flag, _)| index & flag != 0);
    if let Some((_, modifier)) = flags.next() {
        if flags.next().is_some() {
            return Err(RvCodeError::ConflictingModifiers(code));
        }
        binding.modifier = Some(*modifier);
    }
    Ok(binding)
}

fn small(code: u32, index: u16) -> Result<u8, RvCodeError> {
    u8::try_from(index).map_err(|_| RvCodeError::UnknownIndex(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(code: i64) -> Binding {
        Binding::from_rv_code(code).unwrap()
    }

    #[test]
    fn plain_dik_code_is_a_key_press() {
        assert_eq!(decode(0x11), Binding::press(InputCode::Key(Dik::W)));
    }

    #[test]
    fn plus_256_is_a_double_tap() {
        // compassToggle[] = {"256+0x25"} in the shipped default preset: 2xK.
        assert_eq!(
            decode(256 + 0x25),
            Binding::double_tap(InputCode::Key(Dik::K))
        );
    }

    #[test]
    fn combo_modifier_lives_in_the_top_byte() {
        // LCtrl+X.
        let b = decode((0x1D << 24) | 0x2D);
        assert_eq!(
            b,
            Binding::key(Dik::X).with_modifier(Modifier::Input(InputCode::Key(Dik::LCONTROL)))
        );
        assert_eq!(b.to_string(), "LCtrl+X");
    }

    #[test]
    fn legacy_ctrl_offset_means_either_ctrl() {
        // INPUT_CTRL_OFFSET = 512.
        assert_eq!(
            decode(512 + 0x2E),
            Binding::key(Dik::C).with_modifier(Modifier::AnyCtrl)
        );
    }

    #[test]
    fn mouse_buttons_and_double_click() {
        // holdBreath[] = {"0x00010000 + 1"}: RMB; optics[] = {"0x00010000 +128+1", ...}.
        assert_eq!(
            decode(0x0001_0000 + 1),
            Binding::press(InputCode::MouseButton(1))
        );
        assert_eq!(
            decode(0x0001_0000 + 128 + 1),
            Binding::double_tap(InputCode::MouseButton(1))
        );
    }

    #[test]
    fn mouse_axes_and_wheel() {
        // aimHeadUp = 0x00100000 + 2, prevAction = + 4 (wheel up), nextAction = + 5.
        assert_eq!(
            decode(0x0010_0000 + 2),
            Binding::press(InputCode::MouseAxis(MouseAxis::Up))
        );
        assert_eq!(
            decode(0x0010_0000 + 5),
            Binding::press(InputCode::MouseAxis(MouseAxis::WheelDown))
        );
        assert!(Binding::from_rv_code(0x0010_0000 + 6).is_err());
    }

    #[test]
    fn xinput_codes_follow_key_xbox_numbering() {
        // KEY_XINPUT = 0x00050000; KEY_XBOX_LeftTrigger = KEY_XINPUT + 12.
        assert_eq!(
            decode(0x0005_0000 + 12),
            Binding::press(InputCode::Gamepad(GamepadInput::LeftTrigger))
        );
        assert_eq!(
            decode(0x0005_0000 + 21),
            Binding::press(InputCode::Gamepad(GamepadInput::LeftStickDown))
        );
    }

    #[test]
    fn joystick_buttons_and_pov() {
        assert_eq!(
            decode(0x0002_0000 + 9),
            Binding::press(InputCode::JoystickButton(9))
        );
        assert_eq!(
            decode(0x0004_0000 + 4),
            Binding::press(InputCode::JoystickPov(4))
        );
    }

    #[test]
    fn unknown_devices_are_kept_opaque() {
        assert_eq!(
            decode(0x0008_0000 + 3),
            Binding::press(InputCode::Other {
                device: 0x08,
                index: 3
            })
        );
    }

    #[test]
    fn preset_combo_array_is_modifier_then_key() {
        // minimapToggle[] = {{0x9D, 0x32}}: RCtrl+M.
        assert_eq!(
            Binding::from_rv_combo(0x9D, 0x32).unwrap(),
            Binding::key(Dik::M).with_modifier(Modifier::Input(InputCode::Key(Dik::RCONTROL)))
        );
    }

    #[test]
    fn invalid_codes_are_rejected() {
        assert_eq!(Binding::from_rv_code(-1), Err(RvCodeError::OutOfRange(-1)));
        assert!(Binding::from_rv_code(0x1000).is_err());
        assert!(Binding::from_rv_code(512 + 1024 + 0x10).is_err());
    }

    #[test]
    fn encodable_bindings_round_trip() {
        let codes: [i64; 10] = [
            0x11,
            256 + 0x25,
            (0x1D << 24) | 0x2D,
            512 + 0x2E,
            0x0001_0000 + 1,
            0x0001_0000 + 129,
            0x0010_0000 + 4,
            0x0005_0000 + 23,
            0x0002_0000 + 31,
            0x0008_0000 + 3,
        ];
        for code in codes {
            assert_eq!(decode(code).to_rv_code(), Some(code), "code 0x{code:X}");
        }
    }

    #[test]
    fn hold_has_no_rv_code() {
        assert_eq!(
            Binding::hold(InputCode::Key(Dik::SPACE), 0.5).to_rv_code(),
            None
        );
    }
}
