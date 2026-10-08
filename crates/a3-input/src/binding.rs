//! A single input-to-action binding.

use std::fmt;

use crate::code::{Dik, InputCode};

/// A modifier that must be held for a [`Binding`] to fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modifier {
    /// One specific input, e.g. `LCtrl` in `LCtrl+X` or `RMB` in `RMB+LMB`.
    Input(InputCode),
    /// Either Ctrl key (RV's legacy `INPUT_CTRL_OFFSET` bindings).
    AnyCtrl,
    /// Either Shift key (legacy `INPUT_SHIFT_OFFSET`).
    AnyShift,
    /// Either Alt key (legacy `INPUT_ALT_OFFSET`).
    AnyAlt,
}

impl Modifier {
    /// The physical inputs any of which satisfies this modifier.
    pub fn inputs(self) -> &'static [InputCode] {
        const CTRL: [InputCode; 2] = [InputCode::Key(Dik::LCONTROL), InputCode::Key(Dik::RCONTROL)];
        const SHIFT: [InputCode; 2] = [InputCode::Key(Dik::LSHIFT), InputCode::Key(Dik::RSHIFT)];
        const ALT: [InputCode; 2] = [InputCode::Key(Dik::LMENU), InputCode::Key(Dik::RMENU)];
        match self {
            Modifier::Input(_) => &[],
            Modifier::AnyCtrl => &CTRL,
            Modifier::AnyShift => &SHIFT,
            Modifier::AnyAlt => &ALT,
        }
    }

    /// Whether `input` is (one of) this modifier's keys.
    pub fn matches(self, input: InputCode) -> bool {
        match self {
            Modifier::Input(code) => code == input,
            other => other.inputs().contains(&input),
        }
    }
}

impl fmt::Display for Modifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Modifier::Input(code) => code.fmt(f),
            Modifier::AnyCtrl => f.write_str("Ctrl"),
            Modifier::AnyShift => f.write_str("Shift"),
            Modifier::AnyAlt => f.write_str("Alt"),
        }
    }
}

/// How the bound input must be actuated for the binding to be active.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Trigger {
    /// Active while the input is down (or, for relative axes, while it moves).
    Press,
    /// Active from the second press of a quick double tap until that press is released
    /// (RV `256 + DIK`, shown as `2xKey`).
    DoubleTap,
    /// Active once the input has been held for at least `seconds`.
    Hold { seconds: f32 },
}

/// One way to trigger an action: an input, an optional modifier and a [`Trigger`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Binding {
    pub input: InputCode,
    pub modifier: Option<Modifier>,
    pub trigger: Trigger,
}

impl Binding {
    /// Plain press of `input`.
    pub fn press(input: InputCode) -> Binding {
        Binding {
            input,
            modifier: None,
            trigger: Trigger::Press,
        }
    }

    /// Plain press of a keyboard key.
    pub fn key(dik: Dik) -> Binding {
        Binding::press(InputCode::Key(dik))
    }

    /// Double tap of `input`.
    pub fn double_tap(input: InputCode) -> Binding {
        Binding {
            trigger: Trigger::DoubleTap,
            ..Binding::press(input)
        }
    }

    /// Hold `input` for at least `seconds`.
    pub fn hold(input: InputCode, seconds: f32) -> Binding {
        Binding {
            trigger: Trigger::Hold { seconds },
            ..Binding::press(input)
        }
    }

    /// The same binding, requiring `modifier` to be held.
    pub fn with_modifier(self, modifier: Modifier) -> Binding {
        Binding {
            modifier: Some(modifier),
            ..self
        }
    }
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(m) = self.modifier {
            write!(f, "{m}+")?;
        }
        match self.trigger {
            Trigger::Press => write!(f, "{}", self.input),
            Trigger::DoubleTap => write!(f, "2x{}", self.input),
            Trigger::Hold { .. } => write!(f, "Hold {}", self.input),
        }
    }
}
