//! Translation of winit and gilrs identifiers to engine input codes.

use a3_input::{Dik, GamepadInput, InputCode};
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// The DIK scancode of a physical key, or `None` for keys RV has no code for.
pub fn dik_from_keycode(key: KeyCode) -> Option<Dik> {
    use KeyCode as K;
    Some(match key {
        K::Escape => Dik::ESCAPE,
        K::Digit1 => Dik::KEY_1,
        K::Digit2 => Dik::KEY_2,
        K::Digit3 => Dik::KEY_3,
        K::Digit4 => Dik::KEY_4,
        K::Digit5 => Dik::KEY_5,
        K::Digit6 => Dik::KEY_6,
        K::Digit7 => Dik::KEY_7,
        K::Digit8 => Dik::KEY_8,
        K::Digit9 => Dik::KEY_9,
        K::Digit0 => Dik::KEY_0,
        K::Minus => Dik::MINUS,
        K::Equal => Dik::EQUALS,
        K::Backspace => Dik::BACK,
        K::Tab => Dik::TAB,
        K::KeyQ => Dik::Q,
        K::KeyW => Dik::W,
        K::KeyE => Dik::E,
        K::KeyR => Dik::R,
        K::KeyT => Dik::T,
        K::KeyY => Dik::Y,
        K::KeyU => Dik::U,
        K::KeyI => Dik::I,
        K::KeyO => Dik::O,
        K::KeyP => Dik::P,
        K::BracketLeft => Dik::LBRACKET,
        K::BracketRight => Dik::RBRACKET,
        K::Enter => Dik::RETURN,
        K::ControlLeft => Dik::LCONTROL,
        K::KeyA => Dik::A,
        K::KeyS => Dik::S,
        K::KeyD => Dik::D,
        K::KeyF => Dik::F,
        K::KeyG => Dik::G,
        K::KeyH => Dik::H,
        K::KeyJ => Dik::J,
        K::KeyK => Dik::K,
        K::KeyL => Dik::L,
        K::Semicolon => Dik::SEMICOLON,
        K::Quote => Dik::APOSTROPHE,
        K::Backquote => Dik::GRAVE,
        K::ShiftLeft => Dik::LSHIFT,
        K::Backslash => Dik::BACKSLASH,
        K::KeyZ => Dik::Z,
        K::KeyX => Dik::X,
        K::KeyC => Dik::C,
        K::KeyV => Dik::V,
        K::KeyB => Dik::B,
        K::KeyN => Dik::N,
        K::KeyM => Dik::M,
        K::Comma => Dik::COMMA,
        K::Period => Dik::PERIOD,
        K::Slash => Dik::SLASH,
        K::ShiftRight => Dik::RSHIFT,
        K::NumpadMultiply => Dik::MULTIPLY,
        K::AltLeft => Dik::LMENU,
        K::Space => Dik::SPACE,
        K::CapsLock => Dik::CAPITAL,
        K::F1 => Dik::F1,
        K::F2 => Dik::F2,
        K::F3 => Dik::F3,
        K::F4 => Dik::F4,
        K::F5 => Dik::F5,
        K::F6 => Dik::F6,
        K::F7 => Dik::F7,
        K::F8 => Dik::F8,
        K::F9 => Dik::F9,
        K::F10 => Dik::F10,
        K::NumLock => Dik::NUMLOCK,
        K::ScrollLock => Dik::SCROLL,
        K::Numpad7 => Dik::NUMPAD7,
        K::Numpad8 => Dik::NUMPAD8,
        K::Numpad9 => Dik::NUMPAD9,
        K::NumpadSubtract => Dik::SUBTRACT,
        K::Numpad4 => Dik::NUMPAD4,
        K::Numpad5 => Dik::NUMPAD5,
        K::Numpad6 => Dik::NUMPAD6,
        K::NumpadAdd => Dik::ADD,
        K::Numpad1 => Dik::NUMPAD1,
        K::Numpad2 => Dik::NUMPAD2,
        K::Numpad3 => Dik::NUMPAD3,
        K::Numpad0 => Dik::NUMPAD0,
        K::NumpadDecimal => Dik::DECIMAL,
        K::IntlBackslash => Dik::OEM_102,
        K::F11 => Dik::F11,
        K::F12 => Dik::F12,
        K::NumpadEnter => Dik::NUMPADENTER,
        K::ControlRight => Dik::RCONTROL,
        K::NumpadDivide => Dik::DIVIDE,
        K::PrintScreen => Dik::SYSRQ,
        K::AltRight => Dik::RMENU,
        K::Pause => Dik::PAUSE,
        K::Home => Dik::HOME,
        K::ArrowUp => Dik::UP,
        K::PageUp => Dik::PRIOR,
        K::ArrowLeft => Dik::LEFT,
        K::ArrowRight => Dik::RIGHT,
        K::End => Dik::END,
        K::ArrowDown => Dik::DOWN,
        K::PageDown => Dik::NEXT,
        K::Insert => Dik::INSERT,
        K::Delete => Dik::DELETE,
        K::SuperLeft => Dik::LWIN,
        K::SuperRight => Dik::RWIN,
        K::ContextMenu => Dik::APPS,
        _ => return None,
    })
}

/// The engine code of a winit mouse button (RV numbering: 0 left, 1 right, 2 middle, ...).
pub fn mouse_button_code(button: MouseButton) -> Option<InputCode> {
    let index = match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(n) => u8::try_from(n).ok().filter(|n| *n < 0x80)?,
    };
    Some(InputCode::MouseButton(index))
}

/// The gamepad input of a gilrs button. Triggers are reported as analog values.
pub fn gamepad_button(button: gilrs::Button) -> Option<GamepadInput> {
    use gilrs::Button as B;
    Some(match button {
        B::South => GamepadInput::A,
        B::East => GamepadInput::B,
        B::West => GamepadInput::X,
        B::North => GamepadInput::Y,
        B::DPadUp => GamepadInput::DPadUp,
        B::DPadDown => GamepadInput::DPadDown,
        B::DPadLeft => GamepadInput::DPadLeft,
        B::DPadRight => GamepadInput::DPadRight,
        B::Start => GamepadInput::Start,
        B::Select => GamepadInput::Back,
        B::LeftTrigger => GamepadInput::LeftBumper,
        B::RightTrigger => GamepadInput::RightBumper,
        B::LeftTrigger2 => GamepadInput::LeftTrigger,
        B::RightTrigger2 => GamepadInput::RightTrigger,
        B::LeftThumb => GamepadInput::LeftThumb,
        B::RightThumb => GamepadInput::RightThumb,
        _ => return None,
    })
}

/// The (negative, positive) half-axes of a gilrs stick axis. gilrs reports Y positive up.
pub fn gamepad_axis(axis: gilrs::Axis) -> Option<(GamepadInput, GamepadInput)> {
    use gilrs::Axis as A;
    Some(match axis {
        A::LeftStickX => (GamepadInput::LeftStickLeft, GamepadInput::LeftStickRight),
        A::LeftStickY => (GamepadInput::LeftStickDown, GamepadInput::LeftStickUp),
        A::RightStickX => (GamepadInput::RightStickLeft, GamepadInput::RightStickRight),
        A::RightStickY => (GamepadInput::RightStickDown, GamepadInput::RightStickUp),
        _ => return None,
    })
}

/// Split a signed axis value in `-1..=1` into (negative half, positive half) levels in `0..=1`,
/// with a dead zone rescaled so the live range still reaches 1.
pub fn split_axis(value: f32, dead_zone: f32) -> (f32, f32) {
    let magnitude = value.abs();
    let live = if magnitude <= dead_zone {
        0.0
    } else {
        ((magnitude - dead_zone) / (1.0 - dead_zone)).min(1.0)
    };
    if value < 0.0 {
        (live, 0.0)
    } else {
        (0.0, live)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_and_extended_keys_map_to_dik() {
        assert_eq!(dik_from_keycode(KeyCode::KeyW), Some(Dik::W));
        assert_eq!(dik_from_keycode(KeyCode::ControlRight), Some(Dik::RCONTROL));
        assert_eq!(dik_from_keycode(KeyCode::ArrowLeft), Some(Dik::LEFT));
        assert_eq!(dik_from_keycode(KeyCode::F13), None);
    }

    #[test]
    fn mouse_buttons_use_rv_numbering() {
        assert_eq!(
            mouse_button_code(MouseButton::Right),
            Some(InputCode::MouseButton(1))
        );
        assert_eq!(mouse_button_code(MouseButton::Other(500)), None);
    }

    #[test]
    fn gamepad_bumpers_and_triggers_are_not_confused() {
        assert_eq!(
            gamepad_button(gilrs::Button::LeftTrigger),
            Some(GamepadInput::LeftBumper)
        );
        assert_eq!(
            gamepad_button(gilrs::Button::LeftTrigger2),
            Some(GamepadInput::LeftTrigger)
        );
    }

    #[test]
    fn axis_split_applies_dead_zone() {
        assert_eq!(split_axis(0.1, 0.2), (0.0, 0.0));
        assert_eq!(split_axis(-1.0, 0.2), (1.0, 0.0));
        let (neg, pos) = split_axis(0.6, 0.2);
        assert_eq!(neg, 0.0);
        assert!((pos - 0.5).abs() < 1e-6);
    }
}
