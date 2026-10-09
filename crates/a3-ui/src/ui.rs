//! The UI state: display and control arenas, the display stack, animations.

use a3_config::{ConfigRef, ConfigTree};
use a3_sqf::Value;

use crate::kinds::ControlType;
use crate::metrics::{Screen, UiMetrics};
use crate::model::{Animation, Control, ControlId, Display, DisplayId, EventHandler, Rgba};

/// Evaluates the SQF expressions config values may hold (`x = "safeZoneX + 0.1";`,
/// `colorBackground[] = {"(profileNamespace getVariable ['GUI_BCG_RGB_R', 0.13])", ...}`) and
/// localizes `$STR_` text.
pub trait Eval {
    /// The number `expression` evaluates to, or `None` on error.
    fn number(&mut self, expression: &str) -> Option<f32>;
    /// The text for `text` (`$STR_key` localized; anything else unchanged).
    fn localize(&mut self, text: &str) -> String {
        text.to_owned()
    }
}

/// An [`Eval`] that only parses plain numbers.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoEval;

impl Eval for NoEval {
    fn number(&mut self, expression: &str) -> Option<f32> {
        expression.trim().parse().ok()
    }
}

/// Errors from building displays.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// No config class of that name.
    #[error("display class `{0}` not found")]
    NoClass(String),
}

/// All displays and controls.
#[derive(Debug)]
pub struct Ui {
    pub metrics: UiMetrics,
    displays: Vec<Option<Display>>,
    controls: Vec<Option<Control>>,
    /// Open displays, bottom first.
    stack: Vec<DisplayId>,
    /// UI time in seconds (drives `ctrlCommit` animations).
    time: f64,
    next_handler: i32,
}

/// The config entry `name` of `class` resolved through inheritance.
fn entry<'a>(class: &ConfigRef<'a>, name: &str) -> ConfigRef<'a> {
    class.get(name)
}

/// A number entry: a config number, or a string holding an expression.
pub fn read_number(class: &ConfigRef<'_>, name: &str, eval: &mut dyn Eval) -> Option<f32> {
    let e = entry(class, name);
    if e.is_null() {
        return None;
    }
    if e.is_number() {
        return Some(e.number());
    }
    if e.is_text() {
        let text = e.text();
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        return text.parse().ok().or_else(|| eval.number(text));
    }
    None
}

/// A text entry, localized.
pub fn read_text(class: &ConfigRef<'_>, name: &str, eval: &mut dyn Eval) -> Option<String> {
    let e = entry(class, name);
    if e.is_null() || !e.is_text() {
        return None;
    }
    Some(eval.localize(&e.text()))
}

fn value_number(v: &a3_config::Value, eval: &mut dyn Eval) -> Option<f32> {
    match v {
        a3_config::Value::Float(f) => Some(*f),
        a3_config::Value::Int(i) => Some(*i as f32),
        a3_config::Value::Int64(i) => Some(*i as f32),
        a3_config::Value::String(s) | a3_config::Value::Expression(s) => {
            let s = s.trim();
            s.parse().ok().or_else(|| eval.number(s))
        }
        a3_config::Value::Array(_) => None,
    }
}

/// A colour entry: an array of 4 numbers or expressions (3 numbers get alpha 1).
pub fn read_color(class: &ConfigRef<'_>, name: &str, eval: &mut dyn Eval) -> Option<Rgba> {
    let e = entry(class, name);
    if e.is_null() || !e.is_array() {
        return None;
    }
    let items = e.array();
    let mut out = [0.0, 0.0, 0.0, 1.0];
    if items.len() < 3 {
        return None;
    }
    for (slot, item) in out.iter_mut().zip(&items) {
        *slot = value_number(item, eval).unwrap_or(0.0);
    }
    Some(out)
}

impl Ui {
    /// An empty UI for `screen`.
    pub fn new(screen: Screen) -> Self {
        Self {
            metrics: UiMetrics::new(screen),
            displays: Vec::new(),
            controls: Vec::new(),
            stack: Vec::new(),
            time: 0.0,
            next_handler: 0,
        }
    }

    /// Changes the screen size or interface settings.
    pub fn set_screen(&mut self, screen: Screen) {
        self.metrics = UiMetrics::new(screen);
    }

    /// UI time in seconds.
    pub fn time(&self) -> f64 {
        self.time
    }

    /// Advances UI time to `time` and steps `ctrlCommit` animations.
    pub fn update(&mut self, time: f64) {
        self.time = time;
        for control in self.controls.iter_mut().flatten() {
            let Some(anim) = control.animation else {
                continue;
            };
            let t = if anim.duration <= 0.0 {
                1.0
            } else {
                ((time - anim.start) / anim.duration).clamp(0.0, 1.0) as f32
            };
            for i in 0..4 {
                control.position[i] =
                    anim.from_position[i] + (anim.to_position[i] - anim.from_position[i]) * t;
            }
            control.fade = anim.from_fade + (anim.to_fade - anim.from_fade) * t;
            if t >= 1.0 {
                control.animation = None;
            }
        }
    }

    // --- arenas -------------------------------------------------------------------------------

    pub fn display(&self, id: DisplayId) -> Option<&Display> {
        self.displays.get(id.0 as usize)?.as_ref()
    }

    pub fn display_mut(&mut self, id: DisplayId) -> Option<&mut Display> {
        self.displays.get_mut(id.0 as usize)?.as_mut()
    }

    pub fn control(&self, id: ControlId) -> Option<&Control> {
        self.controls.get(id.0 as usize)?.as_ref()
    }

    pub fn control_mut(&mut self, id: ControlId) -> Option<&mut Control> {
        self.controls.get_mut(id.0 as usize)?.as_mut()
    }

    /// Open displays, bottom first.
    pub fn stack(&self) -> &[DisplayId] {
        &self.stack
    }

    /// The topmost open display.
    pub fn top(&self) -> Option<DisplayId> {
        self.stack.last().copied()
    }

    /// The open display with `idd`, topmost first (`findDisplay`).
    pub fn find_display(&self, idd: i32) -> Option<DisplayId> {
        self.stack
            .iter()
            .rev()
            .copied()
            .find(|&d| self.display(d).is_some_and(|d| d.idd == idd))
    }

    /// The control of `display` with `idc` (`displayCtrl`), searching controls groups too.
    pub fn find_control(&self, display: DisplayId, idc: i32) -> Option<ControlId> {
        let d = self.display(display)?;
        let mut queue: Vec<ControlId> = d
            .controls
            .iter()
            .chain(&d.background)
            .chain(&d.objects)
            .copied()
            .collect();
        let mut i = 0;
        while i < queue.len() {
            let c = self.control(queue[i])?;
            if c.idc == idc {
                return Some(c.id);
            }
            queue.extend(c.children.iter().copied());
            i += 1;
        }
        None
    }

    /// Every control of `display` in drawing order (background first, children after their
    /// group).
    pub fn controls_in_order(&self, display: DisplayId) -> Vec<ControlId> {
        let mut out = Vec::new();
        let Some(d) = self.display(display) else {
            return out;
        };
        fn walk(ui: &Ui, ids: &[ControlId], out: &mut Vec<ControlId>) {
            for &id in ids {
                out.push(id);
                if let Some(c) = ui.control(id) {
                    walk(ui, &c.children, out);
                }
            }
        }
        walk(self, &d.background, &mut out);
        walk(self, &d.controls, &mut out);
        out
    }

    fn alloc_control(&mut self, display: DisplayId) -> ControlId {
        let id = ControlId(self.controls.len() as u32);
        self.controls.push(Some(Control::new(id, display)));
        id
    }

    /// A new handler id for `ctrlAddEventHandler` / `displayAddEventHandler`.
    pub fn next_handler_id(&mut self) -> i32 {
        let id = self.next_handler;
        self.next_handler += 1;
        id
    }

    // --- building from config -----------------------------------------------------------------

    /// Builds the display `class` (a root config class) and pushes it on the stack. Event
    /// handlers (including `onLoad`) are not run; the caller runs them (see
    /// [`crate::commands`]).
    pub fn create_display(
        &mut self,
        config: &ConfigTree,
        class: &str,
        parent: Option<DisplayId>,
        dialog: bool,
        eval: &mut dyn Eval,
    ) -> Result<DisplayId, Error> {
        self.create_display_at(config, &[class], parent, dialog, eval)
    }

    /// Like [`Ui::create_display`], for a display class nested in other classes, named by its
    /// path from the config root (`["RscInGameUI", "RscUnitInfoSoldier"]`).
    pub fn create_display_at(
        &mut self,
        config: &ConfigTree,
        class_path: &[&str],
        parent: Option<DisplayId>,
        dialog: bool,
        eval: &mut dyn Eval,
    ) -> Result<DisplayId, Error> {
        let mut cfg = config.root();
        let mut path = Vec::with_capacity(class_path.len());
        for name in class_path {
            cfg = cfg.get(name);
            if !cfg.is_class() {
                return Err(Error::NoClass(class_path.join("/")));
            }
            path.push(cfg.name().to_owned());
        }
        if path.is_empty() {
            return Err(Error::NoClass(String::new()));
        }
        let id = DisplayId(self.displays.len() as u32);
        let mut display = Display {
            id,
            idd: read_number(&cfg, "idd", eval).unwrap_or(-1.0) as i32,
            class_name: cfg.name().to_owned(),
            config_path: path.clone(),
            parent,
            dialog,
            background: Vec::new(),
            controls: Vec::new(),
            objects: Vec::new(),
            events: config_events(&cfg),
            variables: Vec::new(),
            alpha: 1.0,
        };
        self.displays.push(None);
        // The engine loads `controls`, `objects` and `controlsBackground`, in that order.
        for list_name in ["controls", "objects", "controlsBackground"] {
            let list = cfg.get(list_name);
            let ids = if list.is_class() {
                let mut list_path = path.clone();
                list_path.push(list.name().to_owned());
                self.load_children(&list, &list_path, id, None, eval)
            } else if list.is_array() {
                // `controls[] = {"A", "B"};`: class names looked up in the display class.
                self.load_named(&cfg, &list, &path, id, eval)
            } else {
                continue;
            };
            match list_name {
                "controls" => display.controls = ids,
                "objects" => display.objects = ids,
                _ => display.background = ids,
            }
        }
        self.displays[id.0 as usize] = Some(display);
        self.stack.push(id);
        Ok(id)
    }

    /// Loads the control classes declared directly in `list` (the engine enumerates own
    /// entries only, resolving each by name through inheritance).
    fn load_children(
        &mut self,
        list: &ConfigRef<'_>,
        list_path: &[String],
        display: DisplayId,
        parent: Option<ControlId>,
        eval: &mut dyn Eval,
    ) -> Vec<ControlId> {
        let mut ids = Vec::new();
        for item in list.entries() {
            if !item.is_class() {
                continue;
            }
            let class = list.get(item.name());
            let mut path = list_path.to_vec();
            path.push(class.name().to_owned());
            ids.push(self.load_control(&class, path, display, parent, eval));
        }
        ids
    }

    /// Loads the controls an array-form list names (`controls[] = {"A", "B"};`): each name is
    /// looked up in `display_cfg` through inheritance, in list order; a name that is not a
    /// class is skipped (`FUN_141448350`, see `docs/re/ui.md`).
    fn load_named(
        &mut self,
        display_cfg: &ConfigRef<'_>,
        list: &ConfigRef<'_>,
        display_path: &[String],
        display: DisplayId,
        eval: &mut dyn Eval,
    ) -> Vec<ControlId> {
        let mut ids = Vec::new();
        for name in list.array() {
            let a3_config::Value::String(name) = name else {
                continue;
            };
            let class = display_cfg.get(&name);
            if !class.is_class() {
                continue;
            }
            let mut path = display_path.to_vec();
            path.push(class.name().to_owned());
            ids.push(self.load_control(&class, path, display, None, eval));
        }
        ids
    }

    /// Builds one control from its config class.
    pub fn load_control(
        &mut self,
        cfg: &ConfigRef<'_>,
        path: Vec<String>,
        display: DisplayId,
        parent: Option<ControlId>,
        eval: &mut dyn Eval,
    ) -> ControlId {
        let id = self.alloc_control(display);
        let mut c = Control::new(id, display);
        c.parent = parent;
        c.class_name = cfg.name().to_owned();
        c.config_path = path.clone();
        c.idc = read_number(cfg, "idc", eval).unwrap_or(-1.0) as i32;
        c.kind = ControlType::from_code(read_number(cfg, "type", eval).unwrap_or(0.0) as i32);
        c.style = read_number(cfg, "style", eval).unwrap_or(0.0) as i32 as u32;
        let pos = [
            read_number(cfg, "x", eval).unwrap_or(0.0),
            read_number(cfg, "y", eval).unwrap_or(0.0),
            read_number(cfg, "w", eval).unwrap_or(0.0),
            read_number(cfg, "h", eval).unwrap_or(0.0),
        ];
        c.position = pos;
        c.pending_position = pos;
        c.text = read_text(cfg, "text", eval).unwrap_or_default();
        c.tooltip = read_text(cfg, "tooltip", eval).unwrap_or_default();
        let color = |name: &str, eval: &mut dyn Eval| read_color(cfg, name, eval);
        if let Some(v) = color("colorText", eval).or_else(|| color("color", eval)) {
            c.color_text = v;
        }
        if let Some(v) = color("colorBackground", eval) {
            c.color_background = v;
        }
        if let Some(v) = color("colorBackgroundActive", eval).or_else(|| color("colorActive", eval))
        {
            c.color_active = v;
        }
        if let Some(v) = color("colorDisabled", eval) {
            c.color_disabled = v;
        }
        if let Some(v) = color("colorShadow", eval) {
            c.color_shadow = v;
        }
        if let Some(v) = color("colorFocused", eval) {
            c.color_focused = v;
        }
        if let Some(v) = color("colorBorder", eval) {
            c.color_border = v;
        }
        c.font = read_text(cfg, "font", eval).unwrap_or_default();
        // Structured text takes its base size from `size` (`sizeEx` is a plain-text entry
        // its classes often inherit, e.g. `RscHint >> Hint`); other controls from `sizeEx`.
        let (first, second) = if c.kind == ControlType::StructuredText {
            ("size", "sizeEx")
        } else {
            ("sizeEx", "size")
        };
        c.size_ex = read_number(cfg, first, eval)
            .or_else(|| read_number(cfg, second, eval))
            .unwrap_or(c.size_ex);
        c.shadow = read_number(cfg, "shadow", eval).unwrap_or(0.0) as i32;
        c.line_spacing = read_number(cfg, "lineSpacing", eval).unwrap_or(1.0);
        c.angle = read_number(cfg, "angle", eval).unwrap_or(0.0);
        c.picture = read_text(cfg, "textureNoShortcut", eval)
            .or_else(|| read_text(cfg, "picture", eval))
            .unwrap_or_default();
        c.texture_normal = read_text(cfg, "animTextureNormal", eval).unwrap_or_default();
        let text_pos = cfg.get("TextPos");
        if text_pos.is_class() {
            c.text_pos = Some([
                read_number(&text_pos, "left", eval).unwrap_or(0.0),
                read_number(&text_pos, "top", eval).unwrap_or(0.0),
                read_number(&text_pos, "right", eval).unwrap_or(0.0),
                read_number(&text_pos, "bottom", eval).unwrap_or(0.0),
            ]);
        }
        let attributes = cfg.get("Attributes");
        if c.kind == ControlType::StructuredText && attributes.is_class() {
            c.text_align = read_text(&attributes, "align", eval).and_then(|a| {
                match a.to_ascii_lowercase().as_str() {
                    "left" => Some(crate::kinds::style::LEFT),
                    "center" => Some(crate::kinds::style::CENTER),
                    "right" => Some(crate::kinds::style::RIGHT),
                    _ => None,
                }
            });
        }
        c.show = read_number(cfg, "show", eval).is_none_or(|v| v != 0.0);
        c.enabled = read_number(cfg, "enable", eval).is_none_or(|v| v != 0.0);
        c.fade = read_number(cfg, "fade", eval).unwrap_or(0.0);
        c.pending_fade = c.fade;
        c.action = read_text(cfg, "action", eval).unwrap_or_default();
        c.events = config_events(cfg);
        if matches!(
            c.kind,
            ControlType::Slider | ControlType::XSlider | ControlType::Progress
        ) {
            if c.kind == ControlType::Progress {
                // A progress bar draws its bar in `colorBar` and its outline in `colorFrame`.
                if let Some(v) = read_color(cfg, "colorBar", eval) {
                    c.color_text = v;
                }
                if let Some(v) = read_color(cfg, "colorFrame", eval) {
                    c.color_border = v;
                }
            }
            c.value = read_number(cfg, "sliderPosition", eval).unwrap_or(0.0);
            c.range = [
                read_number(cfg, "sliderRangeMin", eval).unwrap_or(0.0),
                read_number(cfg, "sliderRangeMax", eval).unwrap_or(1.0),
            ];
        }
        self.controls[id.0 as usize] = Some(c);
        if ControlType::from_code(read_number(cfg, "type", eval).unwrap_or(0.0) as i32).is_group() {
            let mut children = Vec::new();
            for list_name in ["ControlsBackground", "Controls"] {
                let list = cfg.get(list_name);
                if !list.is_class() {
                    continue;
                }
                let mut list_path = path.clone();
                list_path.push(list.name().to_owned());
                children.extend(self.load_children(&list, &list_path, display, Some(id), eval));
            }
            if let Some(c) = self.control_mut(id) {
                c.children = children;
            }
        }
        id
    }

    /// Creates a control of config class `class` (root class, or under `display`'s class) in
    /// `display` (`ctrlCreate`).
    pub fn create_control(
        &mut self,
        config: &ConfigTree,
        class: &str,
        idc: i32,
        display: DisplayId,
        group: Option<ControlId>,
        eval: &mut dyn Eval,
    ) -> Option<ControlId> {
        let display_path = self.display(display)?.config_path.clone();
        let mut cfg = config.root();
        for name in &display_path {
            cfg = cfg.get(name);
        }
        let nested = cfg.get(class);
        let (cfg, path) = if nested.is_class() {
            let mut p = display_path;
            p.push(nested.name().to_owned());
            (nested, p)
        } else {
            let root = config.root() >> class;
            if !root.is_class() {
                return None;
            }
            (root.clone(), vec![root.name().to_owned()])
        };
        let id = self.load_control(&cfg, path, display, group, eval);
        if let Some(c) = self.control_mut(id) {
            c.idc = idc;
        }
        match group {
            Some(g) => self.control_mut(g)?.children.push(id),
            None => self.display_mut(display)?.controls.push(id),
        }
        Some(id)
    }

    /// Removes `display` (and the displays opened over it) from the stack and the arena.
    /// Returns the closed displays, topmost first.
    pub fn close_display(&mut self, display: DisplayId) -> Vec<DisplayId> {
        let mut closed = Vec::new();
        if let Some(pos) = self.stack.iter().position(|&d| d == display) {
            while self.stack.len() > pos {
                let d = self.stack.pop().expect("non-empty");
                closed.push(d);
            }
        }
        for &d in &closed {
            self.free_display(d);
        }
        closed
    }

    /// Removes only `display` from the stack and the arena, leaving the displays above it
    /// open: for layers of independent displays such as the in-game HUD. Returns whether it
    /// was open.
    pub fn remove_display(&mut self, display: DisplayId) -> bool {
        let Some(pos) = self.stack.iter().position(|&d| d == display) else {
            return false;
        };
        self.stack.remove(pos);
        self.free_display(display);
        true
    }

    fn free_display(&mut self, d: DisplayId) {
        let mut ids = self.controls_in_order(d);
        let mut stack: Vec<ControlId> = self
            .display(d)
            .map(|d| d.objects.clone())
            .unwrap_or_default();
        while let Some(id) = stack.pop() {
            ids.push(id);
            if let Some(c) = self.control(id) {
                stack.extend(c.children.iter().copied());
            }
        }
        for c in ids {
            self.controls[c.0 as usize] = None;
        }
        self.displays[d.0 as usize] = None;
    }

    /// Deletes a control and its children (`ctrlDelete`).
    pub fn delete_control(&mut self, control: ControlId) -> bool {
        let Some(c) = self.control(control) else {
            return false;
        };
        let (display, parent) = (c.display, c.parent);
        let mut stack = vec![control];
        while let Some(id) = stack.pop() {
            if let Some(c) = self.controls.get_mut(id.0 as usize).and_then(Option::take) {
                stack.extend(c.children);
            }
        }
        match parent {
            Some(p) => {
                if let Some(p) = self.control_mut(p) {
                    p.children.retain(|&c| c != control);
                }
            }
            None => {
                if let Some(d) = self.display_mut(display) {
                    d.controls.retain(|&c| c != control);
                    d.background.retain(|&c| c != control);
                    d.objects.retain(|&c| c != control);
                }
            }
        }
        true
    }

    // --- geometry -----------------------------------------------------------------------------

    /// Absolute position `[x, y, w, h]` of a control in UI units (adds the positions and
    /// scrolling of enclosing groups).
    pub fn absolute_position(&self, control: ControlId) -> Option<[f32; 4]> {
        let c = self.control(control)?;
        let mut pos = c.position;
        let mut parent = c.parent;
        while let Some(p) = parent {
            let g = self.control(p)?;
            pos[0] += g.position[0] - g.scroll[0];
            pos[1] += g.position[1] - g.scroll[1];
            parent = g.parent;
        }
        Some(pos)
    }

    /// The clip rectangle of a control (the intersection of its groups' areas), UI units.
    pub fn clip_rect(&self, control: ControlId) -> Option<[f32; 4]> {
        let mut clip: Option<[f32; 4]> = None;
        let mut parent = self.control(control)?.parent;
        while let Some(p) = parent {
            let r = self.absolute_position(p)?;
            clip = Some(match clip {
                None => r,
                Some(c) => intersect(c, r),
            });
            parent = self.control(p)?.parent;
        }
        clip
    }

    /// Whether the control and every enclosing group are shown.
    pub fn is_visible(&self, control: ControlId) -> bool {
        let mut cur = Some(control);
        while let Some(id) = cur {
            let Some(c) = self.control(id) else {
                return false;
            };
            if !c.show || c.fade >= 1.0 {
                return false;
            }
            cur = c.parent;
        }
        true
    }

    /// Combined opacity (1 - fade) of the control and its groups.
    pub fn opacity(&self, control: ControlId) -> f32 {
        let mut alpha = 1.0;
        let mut cur = Some(control);
        while let Some(id) = cur {
            let Some(c) = self.control(id) else {
                break;
            };
            alpha *= 1.0 - c.fade.clamp(0.0, 1.0);
            cur = c.parent;
        }
        alpha
    }

    /// The topmost visible, enabled control of `display` under UI point `(x, y)`.
    pub fn control_at(&self, display: DisplayId, x: f32, y: f32) -> Option<ControlId> {
        self.controls_in_order(display)
            .into_iter()
            .rev()
            .find(|&id| {
                if !self.is_visible(id) {
                    return false;
                }
                let Some(r) = self.absolute_position(id) else {
                    return false;
                };
                let inside = x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3];
                let clipped = self.clip_rect(id).is_some_and(|c| {
                    !(x >= c[0] && y >= c[1] && x < c[0] + c[2] && y < c[1] + c[3])
                });
                inside && !clipped
            })
    }

    // --- animation commands -------------------------------------------------------------------

    /// `ctrlCommit`: animates position and fade from their current values to the pending ones
    /// over `seconds`.
    pub fn commit(&mut self, control: ControlId, seconds: f32) {
        let time = self.time;
        if let Some(c) = self.control_mut(control) {
            c.animation = Some(Animation {
                from_position: c.position,
                to_position: c.pending_position,
                from_fade: c.fade,
                to_fade: c.pending_fade,
                start: time,
                duration: f64::from(seconds.max(0.0)),
            });
            if seconds <= 0.0 {
                c.position = c.pending_position;
                c.fade = c.pending_fade;
                c.animation = None;
            }
        }
    }

    /// `ctrlCommitted`.
    pub fn committed(&self, control: ControlId) -> bool {
        self.control(control).is_none_or(|c| c.animation.is_none())
    }
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    let x1 = (a[0] + a[2]).min(b[0] + b[2]);
    let y1 = (a[1] + a[3]).min(b[1] + b[3]);
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

/// Config event handlers: text entries named `on<Event>`.
fn config_events(cfg: &ConfigRef<'_>) -> Vec<EventHandler> {
    cfg.entries_with_inherited()
        .into_iter()
        .filter(|e| e.is_text())
        .filter_map(|e| {
            let name = e.name();
            let event = name
                .get(..2)?
                .eq_ignore_ascii_case("on")
                .then(|| &name[2..])?;
            let code = e.text();
            (!code.trim().is_empty()).then(|| EventHandler {
                event: event.to_ascii_lowercase(),
                id: -1,
                code: Value::from(code.as_str()),
            })
        })
        .collect()
}
