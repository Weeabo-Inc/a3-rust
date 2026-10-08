//! Well-known user action names and built-in default bindings.
//!
//! Names are RV's own (`CfgDefaultKeysPresets` / `inputAction`). The defaults here stand in
//! until the real presets are loaded from the game config and the player's profile.

use crate::action::ActionMap;
use crate::binding::Binding;
use crate::code::{Dik, GamepadInput, InputCode, MouseAxis};

pub const MOVE_FORWARD: &str = "moveForward";
pub const MOVE_BACK: &str = "moveBack";
pub const MOVE_LEFT: &str = "moveLeft";
pub const MOVE_RIGHT: &str = "moveRight";
pub const TURN_LEFT: &str = "turnLeft";
pub const TURN_RIGHT: &str = "turnRight";
pub const DEFAULT_ACTION: &str = "defaultAction";
pub const RELOAD_MAGAZINE: &str = "reloadMagazine";
pub const INGAME_PAUSE: &str = "ingamePause";

pub const CAMERA_MOVE_FORWARD: &str = "cameraMoveForward";
pub const CAMERA_MOVE_BACKWARD: &str = "cameraMoveBackward";
pub const CAMERA_MOVE_LEFT: &str = "cameraMoveLeft";
pub const CAMERA_MOVE_RIGHT: &str = "cameraMoveRight";
pub const CAMERA_MOVE_UP: &str = "cameraMoveUp";
pub const CAMERA_MOVE_DOWN: &str = "cameraMoveDown";
pub const CAMERA_MOVE_TURBO1: &str = "cameraMoveTurbo1";
pub const CAMERA_MOVE_TURBO2: &str = "cameraMoveTurbo2";
pub const CAMERA_LOOK_UP: &str = "cameraLookUp";
pub const CAMERA_LOOK_DOWN: &str = "cameraLookDown";
pub const CAMERA_LOOK_LEFT: &str = "cameraLookLeft";
pub const CAMERA_LOOK_RIGHT: &str = "cameraLookRight";

/// Built-in bindings for infantry movement and the free (debug/editor) camera.
pub fn default_map() -> ActionMap {
    let key = Binding::key;
    let pad = |g| Binding::press(InputCode::Gamepad(g));
    let mouse = |a| Binding::press(InputCode::MouseAxis(a));
    let mut map = ActionMap::new();
    let table: [(&str, Vec<Binding>); 21] = [
        (MOVE_FORWARD, vec![key(Dik::W), key(Dik::UP)]),
        (MOVE_BACK, vec![key(Dik::S), key(Dik::DOWN)]),
        (MOVE_LEFT, vec![key(Dik::A)]),
        (MOVE_RIGHT, vec![key(Dik::D)]),
        (TURN_LEFT, vec![key(Dik::LEFT), mouse(MouseAxis::Left)]),
        (TURN_RIGHT, vec![key(Dik::RIGHT), mouse(MouseAxis::Right)]),
        (
            DEFAULT_ACTION,
            vec![
                Binding::press(InputCode::MouseButton(0)),
                pad(GamepadInput::RightTrigger),
            ],
        ),
        (RELOAD_MAGAZINE, vec![key(Dik::R), pad(GamepadInput::X)]),
        (
            INGAME_PAUSE,
            vec![key(Dik::ESCAPE), pad(GamepadInput::Start)],
        ),
        (
            CAMERA_MOVE_FORWARD,
            vec![key(Dik::W), pad(GamepadInput::LeftStickUp)],
        ),
        (
            CAMERA_MOVE_BACKWARD,
            vec![key(Dik::S), pad(GamepadInput::LeftStickDown)],
        ),
        (
            CAMERA_MOVE_LEFT,
            vec![key(Dik::A), pad(GamepadInput::LeftStickLeft)],
        ),
        (
            CAMERA_MOVE_RIGHT,
            vec![key(Dik::D), pad(GamepadInput::LeftStickRight)],
        ),
        (
            CAMERA_MOVE_UP,
            vec![key(Dik::Q), pad(GamepadInput::RightBumper)],
        ),
        (
            CAMERA_MOVE_DOWN,
            vec![key(Dik::Z), pad(GamepadInput::LeftBumper)],
        ),
        (
            CAMERA_MOVE_TURBO1,
            vec![key(Dik::LSHIFT), pad(GamepadInput::LeftThumb)],
        ),
        (CAMERA_MOVE_TURBO2, vec![key(Dik::LCONTROL)]),
        (
            CAMERA_LOOK_UP,
            vec![mouse(MouseAxis::Up), pad(GamepadInput::RightStickUp)],
        ),
        (
            CAMERA_LOOK_DOWN,
            vec![mouse(MouseAxis::Down), pad(GamepadInput::RightStickDown)],
        ),
        (
            CAMERA_LOOK_LEFT,
            vec![mouse(MouseAxis::Left), pad(GamepadInput::RightStickLeft)],
        ),
        (
            CAMERA_LOOK_RIGHT,
            vec![mouse(MouseAxis::Right), pad(GamepadInput::RightStickRight)],
        ),
    ];
    for (name, bindings) in table {
        map.set_bindings(name, bindings);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::InputState;

    #[test]
    fn default_map_drives_the_free_camera() {
        let map = default_map();
        let mut s = InputState::new();
        s.press(InputCode::Key(Dik::Q));
        s.mouse_motion(0.0, -3.0);
        assert_eq!(map.value(&s, CAMERA_MOVE_UP), 1.0);
        assert_eq!(map.value(&s, CAMERA_LOOK_UP), 3.0);
        assert_eq!(map.value(&s, CAMERA_LOOK_DOWN), 0.0);
    }
}
