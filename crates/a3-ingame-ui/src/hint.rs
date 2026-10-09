//! Hints: `hint` / `hintSilent` text in `RscInGameUI >> RscHint` (IDD 301), shown for
//! `CfgInGameUI >> Hint >> dimmEndTime` seconds and fading over the last
//! `dimmEndTime - dimmStartTime` (`InGameUI::ShowHint` `FUN_1411d1820`/`FUN_1411d1610`,
//! `DisplayHint::SetHint` `FUN_1411cf100`, drawn by `FUN_1411e66a0`).

use a3_config::{ConfigTree, Value};
use a3_ui::{DisplayId, Fonts, Ui};

use crate::unit_info::driven_control;

/// `IDC_HINT` background (resized to the text).
pub const IDC_BACKGROUND: i32 = 101;
/// The hint's structured text.
pub const IDC_HINT: i32 = 102;

/// The longest hint text the engine keeps, in bytes.
pub const MAX_LENGTH: usize = 0x1000;

/// A sound of `CfgInGameUI` (`sound[] = {path, volume, frequency}`).
#[derive(Debug, Clone, PartialEq)]
pub struct SoundRef {
    pub path: String,
    pub volume: f32,
    pub frequency: f32,
}

impl SoundRef {
    /// The sound entry `name` of `class`, unless its path is empty.
    pub fn from_entry(class: &a3_config::ConfigRef<'_>, name: &str) -> Option<SoundRef> {
        let e = class.get(name);
        if !e.is_array() {
            return None;
        }
        let items = e.array();
        let number = |v: Option<&Value>, default: f32| match v {
            Some(Value::Float(f)) => *f,
            Some(Value::Int(i)) => *i as f32,
            _ => default,
        };
        let path = match items.first() {
            Some(Value::String(s)) if !s.is_empty() => s.clone(),
            _ => return None,
        };
        Some(SoundRef {
            path,
            volume: number(items.get(1), 1.0),
            frequency: number(items.get(2), 1.0),
        })
    }
}

/// `CfgInGameUI >> Hint`.
#[derive(Debug, Clone, PartialEq)]
pub struct HintConfig {
    /// Seconds a hint stays fully visible.
    pub dimm_start: f32,
    /// Seconds after which it is gone.
    pub dimm_end: f32,
    /// Played by `hint` (not by `hintSilent`).
    pub sound: Option<SoundRef>,
}

impl Default for HintConfig {
    fn default() -> Self {
        HintConfig {
            dimm_start: 30.0,
            dimm_end: 35.0,
            sound: None,
        }
    }
}

impl HintConfig {
    pub fn from_config(config: &ConfigTree) -> Self {
        let class = config.root().get("CfgInGameUI").get("Hint");
        let mut out = HintConfig::default();
        for (key, slot) in [
            ("dimmStartTime", &mut out.dimm_start),
            ("dimmEndTime", &mut out.dimm_end),
        ] {
            let e = class.get(key);
            if e.is_number() {
                *slot = e.number();
            }
        }
        out.sound = SoundRef::from_entry(&class, "sound");
        out
    }

    /// The hint's opacity with `remaining` seconds left (`FUN_1411e66a0`): 1 while more than
    /// the fade time is left, then the fraction of the fade time left; `None` once gone.
    pub fn alpha(&self, remaining: f32) -> Option<f32> {
        if remaining < 0.0 {
            return None;
        }
        let fade = self.dimm_end - self.dimm_start;
        Some(if fade < remaining {
            1.0
        } else {
            remaining / fade
        })
    }
}

/// The text of a hint.
#[derive(Debug, Clone, PartialEq)]
pub enum HintText {
    /// A string (`hint "..."`): shown as is.
    Plain(String),
    /// Structured text markup (`hint parseText "..."`).
    Structured(String),
}

impl HintText {
    /// The markup the hint control shows (plain text escaped), cut to [`MAX_LENGTH`] bytes.
    pub fn markup(&self) -> String {
        match self {
            HintText::Plain(s) => {
                let mut end = s.len().min(MAX_LENGTH);
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                s[..end]
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            }
            HintText::Structured(s) => s.clone(),
        }
    }
}

/// The geometry the config gives the hint controls, which the engine resizes and moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Base {
    pub background: [f32; 4],
    pub hint: [f32; 4],
}

/// The state of the hint display.
#[derive(Debug, Clone, Default)]
pub(crate) struct HintState {
    pub display: Option<DisplayId>,
    pub base: Option<Base>,
    pub markup: String,
    /// Seconds left (`InGameUI+0xb6c`), counting down every frame.
    pub remaining: f32,
    /// The text changed: the boxes are resized at the next draw (needs fonts).
    pub dirty: bool,
}

impl HintState {
    /// Remembers the config geometry of a freshly built hint display.
    pub fn attach(&mut self, ui: &Ui, display: DisplayId) {
        let pos = |idc| {
            driven_control(ui, display, idc)
                .and_then(|c| ui.control(c))
                .map(|c| c.position)
        };
        self.display = Some(display);
        self.base = match (pos(IDC_BACKGROUND), pos(IDC_HINT)) {
            (Some(background), Some(hint)) => Some(Base { background, hint }),
            _ => None,
        };
        self.dirty = true;
    }

    /// Moves the box so the background's top is at `top` (`FUN_1411d1170`).
    pub fn place(&self, ui: &mut Ui, top: f32) {
        let (Some(d), Some(base)) = (self.display, self.base) else {
            return;
        };
        let offset = base.hint[1] - base.background[1];
        for (idc, y) in [(IDC_BACKGROUND, top), (IDC_HINT, top + offset)] {
            if let Some(c) = driven_control(ui, d, idc).and_then(|c| ui.control_mut(c)) {
                c.position[1] = y;
                c.pending_position[1] = y;
            }
        }
    }

    /// Sets the text and fits the text box to it, the background keeping its margin
    /// (`FUN_1411cf100`).
    pub fn layout(&mut self, ui: &mut Ui, fonts: &mut Fonts) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let (Some(d), Some(base)) = (self.display, self.base) else {
            return;
        };
        let Some(id) = driven_control(ui, d, IDC_HINT) else {
            return;
        };
        let m = ui.metrics;
        let Some(c) = ui.control_mut(id) else { return };
        c.text = self.markup.clone();
        let pixels = c.size_ex * m.viewport_h;
        let (plain, _) = a3_ui::draw::plain_structured_text(&self.markup);
        let lines = if plain.is_empty() {
            0
        } else {
            fonts
                .wrap(&c.font, pixels, &plain, base.hint[2] * m.viewport_w)
                .len()
        };
        let height = lines as f32 * pixels / m.viewport_h;
        c.position[3] = height;
        c.pending_position[3] = height;
        if let Some(bg) = driven_control(ui, d, IDC_BACKGROUND).and_then(|c| ui.control_mut(c)) {
            let h = base.background[3] - base.hint[3] + height;
            bg.position[3] = h;
            bg.pending_position[3] = h;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_stays_then_fades() {
        let c = HintConfig::default();
        assert_eq!(c.alpha(35.0), Some(1.0));
        assert_eq!(c.alpha(5.5), Some(1.0));
        assert_eq!(c.alpha(2.5), Some(0.5));
        assert_eq!(c.alpha(0.0), Some(0.0));
        assert_eq!(c.alpha(-0.1), None);
    }

    #[test]
    fn plain_hints_are_escaped_and_cut() {
        assert_eq!(
            HintText::Plain("a<b> & c".into()).markup(),
            "a&lt;b&gt; &amp; c"
        );
        assert_eq!(HintText::Plain("x".repeat(5000)).markup().len(), MAX_LENGTH);
        assert_eq!(HintText::Structured("<t>x</t>".into()).markup(), "<t>x</t>");
    }
}
