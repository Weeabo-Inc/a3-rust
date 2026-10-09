//! Displays and controls.

use a3_sqf::Value;

use crate::kinds::ControlType;

/// RGBA colour, components 0..1 (as written in config).
pub type Rgba = [f32; 4];

/// White, the default text colour.
pub const WHITE: Rgba = [1.0, 1.0, 1.0, 1.0];
/// Fully transparent.
pub const TRANSPARENT: Rgba = [0.0, 0.0, 0.0, 0.0];

/// A display in the [`crate::Ui`]'s arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DisplayId(pub u32);

/// A control in the [`crate::Ui`]'s arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ControlId(pub u32);

/// A UI event handler: config text or code added by `ctrlAddEventHandler` /
/// `displayAddEventHandler`.
#[derive(Clone, Debug)]
pub struct EventHandler {
    /// Event name in lower case without the `on` prefix (`buttonclick`, `load`, ...).
    pub event: String,
    /// The handler id (`ctrlAddEventHandler` result); config handlers use -1.
    pub id: i32,
    /// A `String` (compiled on use) or `Code`.
    pub code: Value,
}

/// One row of a list control (`lbAdd`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListItem {
    pub text: String,
    pub data: String,
    pub value: f32,
    pub picture: String,
    pub picture_right: String,
    pub tooltip: String,
    pub color: Option<Rgba>,
}

/// An animation started by `ctrlCommit`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Animation {
    pub from_position: [f32; 4],
    pub to_position: [f32; 4],
    pub from_fade: f32,
    pub to_fade: f32,
    pub start: f64,
    pub duration: f64,
}

/// A control: one element of a display, built from a config class.
#[derive(Debug, Clone)]
pub struct Control {
    pub id: ControlId,
    pub display: DisplayId,
    /// The controls group holding this control, if any.
    pub parent: Option<ControlId>,
    pub idc: i32,
    pub class_name: String,
    /// Config path of the class (names from the root), for reading further attributes.
    pub config_path: Vec<String>,
    pub kind: ControlType,
    pub style: u32,
    /// Current position `[x, y, w, h]` in UI units, relative to the parent group.
    pub position: [f32; 4],
    /// Position set by `ctrlSetPosition`, applied by `ctrlCommit`.
    pub pending_position: [f32; 4],
    pub text: String,
    pub tooltip: String,
    pub color_text: Rgba,
    pub color_background: Rgba,
    pub color_active: Rgba,
    pub color_disabled: Rgba,
    pub color_shadow: Rgba,
    pub color_focused: Rgba,
    pub color_border: Rgba,
    pub font: String,
    pub size_ex: f32,
    pub shadow: i32,
    pub line_spacing: f32,
    pub angle: f32,
    /// Picture for shortcut buttons, active text, ... (`textureNoShortcut`, `picture`).
    pub picture: String,
    /// Normal-state texture of shortcut/menu buttons (`animTextureNormal`).
    pub texture_normal: String,
    /// Text inset of shortcut/menu buttons (`class TextPos`: left, top, right, bottom).
    pub text_pos: Option<[f32; 4]>,
    pub show: bool,
    pub enabled: bool,
    /// Current fade (0 visible, 1 invisible).
    pub fade: f32,
    /// Fade set by `ctrlSetFade`, applied by `ctrlCommit`.
    pub pending_fade: f32,
    pub animation: Option<Animation>,
    pub children: Vec<ControlId>,
    pub events: Vec<EventHandler>,
    pub action: String,
    pub items: Vec<ListItem>,
    pub cur_sel: i32,
    /// Slider / progress position.
    pub value: f32,
    pub range: [f32; 2],
    /// Scroll offset of a controls group, in UI units.
    pub scroll: [f32; 2],
    /// Variables set with `setVariable` on the control.
    pub variables: Vec<(String, Value)>,
}

impl Control {
    /// A control with engine defaults (a blank static).
    pub fn new(id: ControlId, display: DisplayId) -> Self {
        Self {
            id,
            display,
            parent: None,
            idc: -1,
            class_name: String::new(),
            config_path: Vec::new(),
            kind: ControlType::Static,
            style: 0,
            position: [0.0; 4],
            pending_position: [0.0; 4],
            text: String::new(),
            tooltip: String::new(),
            color_text: WHITE,
            color_background: TRANSPARENT,
            color_active: WHITE,
            color_disabled: [1.0, 1.0, 1.0, 0.25],
            color_shadow: [0.0, 0.0, 0.0, 0.5],
            color_focused: TRANSPARENT,
            color_border: TRANSPARENT,
            font: String::new(),
            size_ex: 0.04,
            shadow: 0,
            line_spacing: 1.0,
            angle: 0.0,
            picture: String::new(),
            texture_normal: String::new(),
            text_pos: None,
            show: true,
            enabled: true,
            fade: 0.0,
            pending_fade: 0.0,
            animation: None,
            children: Vec::new(),
            events: Vec::new(),
            action: String::new(),
            items: Vec::new(),
            cur_sel: -1,
            value: 0.0,
            range: [0.0, 1.0],
            scroll: [0.0, 0.0],
            variables: Vec::new(),
        }
    }
}

/// A display (screen or dialog), built from an `RscDisplay*` config class.
#[derive(Debug, Clone)]
pub struct Display {
    pub id: DisplayId,
    pub idd: i32,
    pub class_name: String,
    pub config_path: Vec<String>,
    /// The display it was created over (`createDisplay` / `createDialog` parent).
    pub parent: Option<DisplayId>,
    /// `createDialog` (modal) rather than `createDisplay`.
    pub dialog: bool,
    /// `controlsBackground`, drawn first.
    pub background: Vec<ControlId>,
    /// `controls`.
    pub controls: Vec<ControlId>,
    /// `objects`: 3D object controls (`RscObject`), loaded after `controls`; drawn in 3D, not
    /// by [`crate::build_draw_list`].
    pub objects: Vec<ControlId>,
    pub events: Vec<EventHandler>,
    pub variables: Vec<(String, Value)>,
    /// Opacity the whole display draws with, 0..1 (the engine's in-game displays fade as a
    /// unit, `Display::DrawHUD(alpha)`); 1 for ordinary displays.
    pub alpha: f32,
}
