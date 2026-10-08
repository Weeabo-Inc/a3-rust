//! Identifiers for individual physical inputs.

use std::fmt;

/// A keyboard key, identified by its DirectInput scancode (`DIK_*`).
///
/// RV stores keyboard bindings as DIK codes, so the engine uses them as its canonical key
/// identity. The platform layer maps OS key events onto these codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Dik(pub u8);

macro_rules! dik_codes {
    ($($name:ident = $code:literal, $label:literal;)*) => {
        impl Dik {
            $(pub const $name: Dik = Dik($code);)*

            /// Short human-readable label of the key (`"LCtrl"`, `"W"`, `"Num 5"`), or `None`
            /// for a scancode without a known name.
            pub fn label(self) -> Option<&'static str> {
                match self.0 {
                    $($code => Some($label),)*
                    _ => None,
                }
            }
        }
    };
}

dik_codes! {
    ESCAPE = 0x01, "Esc";
    KEY_1 = 0x02, "1";
    KEY_2 = 0x03, "2";
    KEY_3 = 0x04, "3";
    KEY_4 = 0x05, "4";
    KEY_5 = 0x06, "5";
    KEY_6 = 0x07, "6";
    KEY_7 = 0x08, "7";
    KEY_8 = 0x09, "8";
    KEY_9 = 0x0A, "9";
    KEY_0 = 0x0B, "0";
    MINUS = 0x0C, "-";
    EQUALS = 0x0D, "=";
    BACK = 0x0E, "Backspace";
    TAB = 0x0F, "Tab";
    Q = 0x10, "Q";
    W = 0x11, "W";
    E = 0x12, "E";
    R = 0x13, "R";
    T = 0x14, "T";
    Y = 0x15, "Y";
    U = 0x16, "U";
    I = 0x17, "I";
    O = 0x18, "O";
    P = 0x19, "P";
    LBRACKET = 0x1A, "[";
    RBRACKET = 0x1B, "]";
    RETURN = 0x1C, "Enter";
    LCONTROL = 0x1D, "LCtrl";
    A = 0x1E, "A";
    S = 0x1F, "S";
    D = 0x20, "D";
    F = 0x21, "F";
    G = 0x22, "G";
    H = 0x23, "H";
    J = 0x24, "J";
    K = 0x25, "K";
    L = 0x26, "L";
    SEMICOLON = 0x27, ";";
    APOSTROPHE = 0x28, "'";
    GRAVE = 0x29, "`";
    LSHIFT = 0x2A, "LShift";
    BACKSLASH = 0x2B, "\\";
    Z = 0x2C, "Z";
    X = 0x2D, "X";
    C = 0x2E, "C";
    V = 0x2F, "V";
    B = 0x30, "B";
    N = 0x31, "N";
    M = 0x32, "M";
    COMMA = 0x33, ",";
    PERIOD = 0x34, ".";
    SLASH = 0x35, "/";
    RSHIFT = 0x36, "RShift";
    MULTIPLY = 0x37, "Num *";
    LMENU = 0x38, "LAlt";
    SPACE = 0x39, "Space";
    CAPITAL = 0x3A, "Caps Lock";
    F1 = 0x3B, "F1";
    F2 = 0x3C, "F2";
    F3 = 0x3D, "F3";
    F4 = 0x3E, "F4";
    F5 = 0x3F, "F5";
    F6 = 0x40, "F6";
    F7 = 0x41, "F7";
    F8 = 0x42, "F8";
    F9 = 0x43, "F9";
    F10 = 0x44, "F10";
    NUMLOCK = 0x45, "Num Lock";
    SCROLL = 0x46, "Scroll Lock";
    NUMPAD7 = 0x47, "Num 7";
    NUMPAD8 = 0x48, "Num 8";
    NUMPAD9 = 0x49, "Num 9";
    SUBTRACT = 0x4A, "Num -";
    NUMPAD4 = 0x4B, "Num 4";
    NUMPAD5 = 0x4C, "Num 5";
    NUMPAD6 = 0x4D, "Num 6";
    ADD = 0x4E, "Num +";
    NUMPAD1 = 0x4F, "Num 1";
    NUMPAD2 = 0x50, "Num 2";
    NUMPAD3 = 0x51, "Num 3";
    NUMPAD0 = 0x52, "Num 0";
    DECIMAL = 0x53, "Num .";
    OEM_102 = 0x56, "<>";
    F11 = 0x57, "F11";
    F12 = 0x58, "F12";
    NUMPADENTER = 0x9C, "Num Enter";
    RCONTROL = 0x9D, "RCtrl";
    DIVIDE = 0xB5, "Num /";
    SYSRQ = 0xB7, "Print Screen";
    RMENU = 0xB8, "RAlt";
    PAUSE = 0xC5, "Pause";
    HOME = 0xC7, "Home";
    UP = 0xC8, "Up";
    PRIOR = 0xC9, "Page Up";
    LEFT = 0xCB, "Left";
    RIGHT = 0xCD, "Right";
    END = 0xCF, "End";
    DOWN = 0xD0, "Down";
    NEXT = 0xD1, "Page Down";
    INSERT = 0xD2, "Insert";
    DELETE = 0xD3, "Delete";
    LWIN = 0xDB, "LWin";
    RWIN = 0xDC, "RWin";
    APPS = 0xDD, "Apps";
}

impl fmt::Display for Dik {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.label() {
            Some(label) => f.write_str(label),
            None => write!(f, "DIK 0x{:02X}", self.0),
        }
    }
}

/// One half of a relative mouse axis, or a wheel direction.
///
/// RV binds each direction separately (`aimLeft` to "mouse left", `aimRight` to "mouse right").
/// Values are per-frame motion amounts, not positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MouseAxis {
    Left,
    Right,
    Up,
    Down,
    WheelUp,
    WheelDown,
}

/// A gamepad input in XInput layout, numbered as RV's `KEY_XBOX_*` codes.
///
/// Stick and trigger directions are separate half-axes with an absolute value in `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GamepadInput {
    A,
    B,
    X,
    Y,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    Start,
    Back,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    LeftThumb,
    RightThumb,
    LeftStickRight,
    LeftStickUp,
    RightStickRight,
    RightStickUp,
    LeftStickLeft,
    LeftStickDown,
    RightStickLeft,
    RightStickDown,
}

impl GamepadInput {
    /// All inputs in RV `KEY_XBOX_*` index order (`KEY_XINPUT + index`).
    pub const ALL: [GamepadInput; 24] = [
        GamepadInput::A,
        GamepadInput::B,
        GamepadInput::X,
        GamepadInput::Y,
        GamepadInput::DPadUp,
        GamepadInput::DPadDown,
        GamepadInput::DPadLeft,
        GamepadInput::DPadRight,
        GamepadInput::Start,
        GamepadInput::Back,
        GamepadInput::LeftBumper,
        GamepadInput::RightBumper,
        GamepadInput::LeftTrigger,
        GamepadInput::RightTrigger,
        GamepadInput::LeftThumb,
        GamepadInput::RightThumb,
        GamepadInput::LeftStickRight,
        GamepadInput::LeftStickUp,
        GamepadInput::RightStickRight,
        GamepadInput::RightStickUp,
        GamepadInput::LeftStickLeft,
        GamepadInput::LeftStickDown,
        GamepadInput::RightStickLeft,
        GamepadInput::RightStickDown,
    ];

    /// Index of this input in RV's `KEY_XBOX_*` numbering.
    pub fn rv_index(self) -> u8 {
        GamepadInput::ALL
            .iter()
            .position(|&g| g == self)
            .expect("ALL lists every variant") as u8
    }

    /// The input with the given RV `KEY_XBOX_*` index.
    pub fn from_rv_index(index: u8) -> Option<GamepadInput> {
        GamepadInput::ALL.get(usize::from(index)).copied()
    }
}

/// One physical input the engine can bind an action to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InputCode {
    /// Keyboard key by DIK scancode.
    Key(Dik),
    /// Mouse button; 0 left, 1 right, 2 middle, then extra buttons.
    MouseButton(u8),
    /// Relative mouse motion or wheel in one direction.
    MouseAxis(MouseAxis),
    /// Gamepad (XInput layout) button or half-axis.
    Gamepad(GamepadInput),
    /// DirectInput joystick button.
    JoystickButton(u8),
    /// DirectInput joystick POV hat direction (0 up, 2 right, 4 down, 6 left; odd = diagonals).
    JoystickPov(u8),
    /// A device RV knows that we do not model yet (`device` is bits 16..24 of the RV code).
    Other { device: u8, index: u16 },
}

impl InputCode {
    /// Whether this input reports relative motion (summed per frame) rather than a level.
    pub fn is_relative(self) -> bool {
        matches!(self, InputCode::MouseAxis(_))
    }
}

impl fmt::Display for InputCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InputCode::Key(dik) => dik.fmt(f),
            InputCode::MouseButton(0) => f.write_str("LMB"),
            InputCode::MouseButton(1) => f.write_str("RMB"),
            InputCode::MouseButton(2) => f.write_str("MMB"),
            InputCode::MouseButton(n) => write!(f, "Mouse {}", n + 1),
            InputCode::MouseAxis(axis) => write!(f, "Mouse {axis:?}"),
            InputCode::Gamepad(g) => write!(f, "Pad {g:?}"),
            InputCode::JoystickButton(n) => write!(f, "Joy {}", n + 1),
            InputCode::JoystickPov(n) => write!(f, "Joy POV {n}"),
            InputCode::Other { device, index } => write!(f, "Device 0x{device:02X} #{index}"),
        }
    }
}
