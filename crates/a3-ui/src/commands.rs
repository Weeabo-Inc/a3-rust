//! SQF UI commands (#81) on a [`UiHost`].
//!
//! Displays and controls cross the script boundary as `Value::Handle` of kind `Display` /
//! `Control` whose id is the arena index plus one (0 is `displayNull` / `controlNull`). A
//! handle whose display or control was closed behaves as null.
//!
//! Event handlers (config `on<Event>` text and code added with `ctrlAddEventHandler` /
//! `displayAddEventHandler`) run unscheduled with `_this` set to the event arguments
//! ([`fire_control_event`], [`fire_display_event`]).

use std::sync::Arc;

use a3_config::ConfigTree;
use a3_sqf::registry::Registry;
use a3_sqf::vm::Ctx;
use a3_sqf::{Array, Handle, HandleKind, Host, SqfError, Type, TypeSet, Value, Vm};

use crate::model::{ControlId, DisplayId, EventHandler, ListItem, Rgba};
use crate::text::Fonts;
use crate::ui::{Eval, Ui};

/// A script host with a UI.
pub trait UiHost: Host {
    fn ui(&self) -> &Ui;
    fn ui_mut(&mut self) -> &mut Ui;
    /// `configFile`, for `createDisplay` / `ctrlCreate`.
    fn ui_config(&self) -> Option<Arc<ConfigTree>>;
    /// Fonts, for text measurement (`ctrlTextWidth`, `ctrlTextHeight`).
    fn ui_fonts(&mut self) -> Option<&mut Fonts> {
        None
    }
}

const NUM: TypeSet = TypeSet::NUMBER;
const BOOL: TypeSet = TypeSet::of(Type::Bool);
const STR: TypeSet = TypeSet::of(Type::String);
const ARR: TypeSet = TypeSet::of(Type::Array);
const CODE: TypeSet = TypeSet::of(Type::Code);
const DISP: TypeSet = TypeSet::of(Type::Display);
const CTRL: TypeSet = TypeSet::of(Type::Control);
const TEXT: TypeSet = TypeSet::of(Type::Text);
const NOTHING: TypeSet = TypeSet::of(Type::Nothing);
const ANY: TypeSet = TypeSet::ANYTHING;

/// The handle of a display.
pub fn display_value(id: Option<DisplayId>) -> Value {
    Value::Handle(Handle {
        kind: HandleKind::Display,
        id: id.map_or(0, |d| u64::from(d.0) + 1),
    })
}

/// The handle of a control.
pub fn control_value(id: Option<ControlId>) -> Value {
    Value::Handle(Handle {
        kind: HandleKind::Control,
        id: id.map_or(0, |c| u64::from(c.0) + 1),
    })
}

/// The display a value refers to, if it is open.
pub fn as_display(ui: &Ui, v: &Value) -> Option<DisplayId> {
    match v {
        Value::Handle(Handle {
            kind: HandleKind::Display,
            id,
        }) if *id > 0 => {
            let d = DisplayId((*id - 1) as u32);
            ui.display(d).map(|_| d)
        }
        _ => None,
    }
}

/// The control a value refers to, if it exists.
pub fn as_control(ui: &Ui, v: &Value) -> Option<ControlId> {
    match v {
        Value::Handle(Handle {
            kind: HandleKind::Control,
            id,
        }) if *id > 0 => {
            let c = ControlId((*id - 1) as u32);
            ui.control(c).map(|_| c)
        }
        _ => None,
    }
}

fn num(v: &Value) -> f32 {
    match v {
        Value::Number(n) => *n,
        _ => 0.0,
    }
}

fn string(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        _ => String::new(),
    }
}

fn items(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.borrow().clone(),
        _ => Vec::new(),
    }
}

fn array(values: Vec<Value>) -> Value {
    Value::Array(Array::from_vec(values))
}

fn color(v: &Value) -> Option<Rgba> {
    let items = items(v);
    if items.len() < 3 {
        return None;
    }
    let mut c = [0.0, 0.0, 0.0, 1.0];
    for (slot, item) in c.iter_mut().zip(&items) {
        *slot = num(item);
    }
    Some(c)
}

/// Evaluates config expressions inside a command, on the running VM.
struct CtxEval<'c, 'a, H: UiHost> {
    ctx: &'c mut Ctx<'a, H>,
}

impl<H: UiHost> Eval for CtxEval<'_, '_, H> {
    fn number(&mut self, expression: &str) -> Option<f32> {
        let code = self.ctx.compile("", expression).ok()?;
        match self.ctx.call_unscheduled(&code, None) {
            Ok(Value::Number(n)) => Some(n),
            _ => None,
        }
    }

    fn localize(&mut self, text: &str) -> String {
        localize(&*self.ctx.host, text)
    }
}

/// Evaluates config expressions on a [`Vm`] (outside commands).
pub struct VmEval<'v, H: Host> {
    pub vm: &'v mut Vm<H>,
}

impl<H: Host> Eval for VmEval<'_, H> {
    fn number(&mut self, expression: &str) -> Option<f32> {
        let code = self.vm.compile(expression).ok()?;
        match self.vm.call(&code, None) {
            Ok(Value::Number(n)) => Some(n),
            _ => None,
        }
    }

    fn localize(&mut self, text: &str) -> String {
        localize(&self.vm.host, text)
    }
}

/// `$STR_key` → the host's localized text (unchanged when unknown or not a key).
pub fn localize<H: Host + ?Sized>(host: &H, text: &str) -> String {
    match text
        .strip_prefix('$')
        .filter(|k| k.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("STR_")))
    {
        Some(key) => host.localize(key).unwrap_or_else(|| text.to_owned()),
        None => text.to_owned(),
    }
}

fn handlers_of(ui: &Ui, target: Target, event: &str) -> Vec<Value> {
    let list = match target {
        Target::Display(d) => ui.display(d).map(|d| d.events.clone()),
        Target::Control(c) => ui.control(c).map(|c| c.events.clone()),
    };
    list.unwrap_or_default()
        .into_iter()
        .filter(|h| h.event == event)
        .map(|h| h.code)
        .collect()
}

#[derive(Clone, Copy)]
enum Target {
    Display(DisplayId),
    Control(ControlId),
}

fn run_handlers_ctx<H: UiHost>(
    ctx: &mut Ctx<'_, H>,
    target: Target,
    event: &str,
    args: Vec<Value>,
) -> Vec<Value> {
    let mut results = Vec::new();
    for code in handlers_of(ctx.host.ui(), target, event) {
        let code = match code {
            Value::Code(c) => c,
            Value::String(s) => match ctx.compile("", &s) {
                Ok(c) => c,
                Err(_) => continue,
            },
            _ => continue,
        };
        if let Ok(v) = ctx.call_unscheduled(&code, Some(array(args.clone()))) {
            results.push(v);
        }
    }
    results
}

/// Runs the `event` handlers of `display` (e.g. `"load"`) with `_this = [display, args...]`.
pub fn fire_display_event<H: UiHost>(
    vm: &mut Vm<H>,
    display: DisplayId,
    event: &str,
    extra: Vec<Value>,
) -> Vec<Value> {
    let mut args = vec![display_value(Some(display))];
    args.extend(extra);
    run_handlers_vm(vm, Target::Display(display), event, args)
}

/// Runs the `event` handlers of `control` (e.g. `"buttonclick"`) with
/// `_this = [control, args...]`.
pub fn fire_control_event<H: UiHost>(
    vm: &mut Vm<H>,
    control: ControlId,
    event: &str,
    extra: Vec<Value>,
) -> Vec<Value> {
    let mut args = vec![control_value(Some(control))];
    args.extend(extra);
    run_handlers_vm(vm, Target::Control(control), event, args)
}

fn run_handlers_vm<H: UiHost>(
    vm: &mut Vm<H>,
    target: Target,
    event: &str,
    args: Vec<Value>,
) -> Vec<Value> {
    let mut results = Vec::new();
    for code in handlers_of(vm.host.ui(), target, event) {
        let code = match code {
            Value::Code(c) => c,
            Value::String(s) => match vm.compile(&s) {
                Ok(c) => c,
                Err(_) => continue,
            },
            _ => continue,
        };
        if let Ok(v) = vm.call(&code, Some(array(args.clone()))) {
            results.push(v);
        }
    }
    results
}

/// Runs `onLoad` of every control of `display` (with `[control]`), then of the display (with
/// `[display]`), as the engine does after building it.
pub fn fire_load_events<H: UiHost>(vm: &mut Vm<H>, display: DisplayId) {
    for c in vm.host.ui().controls_in_order(display) {
        fire_control_event(vm, c, "load", Vec::new());
    }
    fire_display_event(vm, display, "load", Vec::new());
}

fn fire_load_events_ctx<H: UiHost>(ctx: &mut Ctx<'_, H>, display: DisplayId) {
    for c in ctx.host.ui().controls_in_order(display) {
        run_handlers_ctx(
            ctx,
            Target::Control(c),
            "load",
            vec![control_value(Some(c))],
        );
    }
    run_handlers_ctx(
        ctx,
        Target::Display(display),
        "load",
        vec![display_value(Some(display))],
    );
}

fn create_display_ctx<H: UiHost>(
    ctx: &mut Ctx<'_, H>,
    class: &str,
    parent: Option<DisplayId>,
    dialog: bool,
) -> Value {
    let Some(config) = ctx.host.ui_config() else {
        return display_value(None);
    };
    // Config expressions run on the VM while the display is built, so the UI is moved out of
    // the host meanwhile; a stand-in with the same screen answers the layout commands
    // (`safeZoneX`, `pixelGrid`, ...).
    let screen = ctx.host.ui().metrics.screen;
    let result = {
        let mut eval = CtxEval { ctx };
        let mut ui = std::mem::replace(eval.ctx.host.ui_mut(), Ui::new(screen));
        let result = ui.create_display(&config, class, parent, dialog, &mut eval);
        *eval.ctx.host.ui_mut() = ui;
        result
    };
    match result {
        Ok(d) => {
            fire_load_events_ctx(ctx, d);
            display_value(Some(d))
        }
        Err(_) => display_value(None),
    }
}

/// Creates `class` on top of the stack from outside the VM and runs its load events.
pub fn open_display<H: UiHost>(vm: &mut Vm<H>, class: &str) -> Option<DisplayId> {
    let config = vm.host.ui_config()?;
    let screen = vm.host.ui().metrics.screen;
    let mut ui = std::mem::replace(vm.host.ui_mut(), Ui::new(screen));
    let parent = ui.top();
    let result = ui.create_display(&config, class, parent, false, &mut VmEval { vm });
    *vm.host.ui_mut() = ui;
    let d = result.ok()?;
    fire_load_events(vm, d);
    Some(d)
}

fn set_position(ui: &mut Ui, c: ControlId, values: &[Value]) {
    if let Some(ctrl) = ui.control_mut(c) {
        for (i, v) in values.iter().take(4).enumerate() {
            ctrl.pending_position[i] = num(v);
        }
    }
}

fn add_handler<H: UiHost>(ctx: &mut Ctx<'_, H>, target: Target, event: &str, code: Value) -> i32 {
    let ui = ctx.host.ui_mut();
    let id = ui.next_handler_id();
    let handler = EventHandler {
        event: event.to_ascii_lowercase(),
        id,
        code,
    };
    match target {
        Target::Display(d) => {
            if let Some(d) = ui.display_mut(d) {
                d.events.push(handler);
                return id;
            }
        }
        Target::Control(c) => {
            if let Some(c) = ui.control_mut(c) {
                c.events.push(handler);
                return id;
            }
        }
    }
    -1
}

fn text_metrics<H: UiHost>(
    ctx: &mut Ctx<'_, H>,
    c: ControlId,
) -> Option<(String, f32, String, f32)> {
    let ui = ctx.host.ui();
    let ctrl = ui.control(c)?;
    let pixels = ctrl.size_ex * ui.metrics.viewport_h;
    let width_px = ctrl.position[2] * ui.metrics.viewport_w;
    Some((ctrl.font.clone(), pixels, ctrl.text.clone(), width_px))
}

/// Registers the UI commands.
pub fn register_ui_commands<H: UiHost>(r: &mut Registry<H>) {
    register_layout(r);
    register_displays(r);
    register_controls(r);
    register_lists(r);
    register_events(r);
}

fn register_layout<H: UiHost>(r: &mut Registry<H>) {
    r.nular("safeZoneX", NUM, |ctx| {
        Ok(Value::Number(ctx.host.ui().metrics.safe_zone_x))
    });
    r.nular("safeZoneY", NUM, |ctx| {
        Ok(Value::Number(ctx.host.ui().metrics.safe_zone_y))
    });
    r.nular("safeZoneW", NUM, |ctx| {
        Ok(Value::Number(ctx.host.ui().metrics.safe_zone_w))
    });
    r.nular("safeZoneH", NUM, |ctx| {
        Ok(Value::Number(ctx.host.ui().metrics.safe_zone_h))
    });
    // Single monitor: the absolute variants equal the plain ones.
    r.nular("safeZoneXAbs", NUM, |ctx| {
        Ok(Value::Number(ctx.host.ui().metrics.safe_zone_x))
    });
    r.nular("safeZoneWAbs", NUM, |ctx| {
        Ok(Value::Number(ctx.host.ui().metrics.safe_zone_w))
    });
    r.nular("pixelW", NUM, |ctx| {
        Ok(Value::Number(ctx.host.ui().metrics.pixel_w))
    });
    r.nular("pixelH", NUM, |ctx| {
        Ok(Value::Number(ctx.host.ui().metrics.pixel_h))
    });
    r.nular("pixelGrid", NUM, |ctx| {
        Ok(Value::Number(ctx.host.ui().metrics.pixel_grid))
    });
    r.nular("pixelGridBase", NUM, |ctx| {
        Ok(Value::Number(ctx.host.ui().metrics.pixel_grid_base))
    });
    r.nular("pixelGridNoUIScale", NUM, |ctx| {
        Ok(Value::Number(ctx.host.ui().metrics.pixel_grid_no_ui_scale))
    });
    r.nular("getResolution", ARR, |ctx| {
        let m = ctx.host.ui().metrics;
        Ok(array(vec![
            Value::Number(m.screen.width as f32),
            Value::Number(m.screen.height as f32),
            Value::Number(m.viewport_w),
            Value::Number(m.viewport_h),
            Value::Number(m.aspect()),
            Value::Number(m.screen.ui_scale),
            Value::Number(0.75),
            Value::Number(0.75 * m.aspect()),
            Value::Bool(false),
            Value::Number(1.0),
        ]))
    });
    // The engine marks UI-holding scripts as not serializable; nothing to do without saves.
    r.nular("disableSerialization", NOTHING, |_| Ok(Value::Nothing));
}

fn register_displays<H: UiHost>(r: &mut Registry<H>) {
    r.unary("findDisplay", NUM, DISP, |ctx, a| {
        Ok(display_value(ctx.host.ui().find_display(num(&a) as i32)))
    });
    r.nular("allDisplays", ARR, |ctx| {
        let ui = ctx.host.ui();
        Ok(array(
            ui.stack().iter().map(|&d| display_value(Some(d))).collect(),
        ))
    });
    r.binary("createDisplay", DISP, STR, DISP, |ctx, a, b| {
        let parent = as_display(ctx.host.ui(), &a);
        if parent.is_none() {
            return Ok(display_value(None));
        }
        Ok(create_display_ctx(ctx, &string(&b), parent, false))
    });
    r.unary("createDialog", STR, BOOL, |ctx, a| {
        let parent = ctx.host.ui().top();
        let d = create_display_ctx(ctx, &string(&a), parent, true);
        Ok(Value::Bool(as_display(ctx.host.ui(), &d).is_some()))
    });
    r.binary("closeDisplay", DISP, NUM, NOTHING, |ctx, a, b| {
        if let Some(d) = as_display(ctx.host.ui(), &a) {
            let code = num(&b);
            run_handlers_ctx(
                ctx,
                Target::Display(d),
                "unload",
                vec![display_value(Some(d)), Value::Number(code)],
            );
            ctx.host.ui_mut().close_display(d);
        }
        Ok(Value::Nothing)
    });
    r.unary("closeDialog", NUM, NOTHING, |ctx, _| {
        let ui = ctx.host.ui();
        if let Some(d) = ui
            .stack()
            .iter()
            .rev()
            .copied()
            .find(|&d| ui.display(d).is_some_and(|d| d.dialog))
        {
            ctx.host.ui_mut().close_display(d);
        }
        Ok(Value::Nothing)
    });
    r.nular("dialog", BOOL, |ctx| {
        let ui = ctx.host.ui();
        Ok(Value::Bool(
            ui.stack()
                .iter()
                .any(|&d| ui.display(d).is_some_and(|d| d.dialog)),
        ))
    });
    r.unary("ctrlIDD", DISP, NUM, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(Value::Number(
            as_display(ui, &a)
                .and_then(|d| ui.display(d))
                .map_or(-1.0, |d| d.idd as f32),
        ))
    });
    r.binary("displayCtrl", DISP, NUM, CTRL, |ctx, a, b| {
        let ui = ctx.host.ui();
        Ok(control_value(
            as_display(ui, &a).and_then(|d| ui.find_control(d, num(&b) as i32)),
        ))
    });
    r.unary("allControls", DISP, ARR, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(array(
            as_display(ui, &a)
                .map(|d| ui.controls_in_order(d))
                .unwrap_or_default()
                .into_iter()
                .map(|c| control_value(Some(c)))
                .collect(),
        ))
    });
    r.unary("displayParent", DISP, DISP, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(display_value(
            as_display(ui, &a)
                .and_then(|d| ui.display(d))
                .and_then(|d| d.parent),
        ))
    });
    r.binary("setVariable", DISP, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        if let (Some(d), Some(name)) = (as_display(ctx.host.ui(), &a), args.first()) {
            let name = string(name).to_ascii_lowercase();
            let value = args.get(1).cloned().unwrap_or(Value::Nil);
            if let Some(d) = ctx.host.ui_mut().display_mut(d) {
                d.variables.retain(|(n, _)| *n != name);
                d.variables.push((name, value));
            }
        }
        Ok(Value::Nothing)
    });
    r.binary("getVariable", DISP, STR | ARR, ANY, |ctx, a, b| {
        let (name, default) = match &b {
            Value::Array(_) => {
                let args = items(&b);
                (
                    args.first().map(string).unwrap_or_default(),
                    args.get(1).cloned().unwrap_or(Value::Nil),
                )
            }
            other => (string(other), Value::Nil),
        };
        let ui = ctx.host.ui();
        let name = name.to_ascii_lowercase();
        Ok(as_display(ui, &a)
            .and_then(|d| ui.display(d))
            .and_then(|d| d.variables.iter().find(|(n, _)| *n == name))
            .map_or(default, |(_, v)| v.clone()))
    });
}

fn register_controls<H: UiHost>(r: &mut Registry<H>) {
    r.unary("ctrlIDC", CTRL, NUM, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(Value::Number(
            as_control(ui, &a)
                .and_then(|c| ui.control(c))
                .map_or(-1.0, |c| c.idc as f32),
        ))
    });
    r.unary("ctrlParent", CTRL, DISP, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(display_value(
            as_control(ui, &a)
                .and_then(|c| ui.control(c))
                .map(|c| c.display),
        ))
    });
    r.unary("ctrlParentControlsGroup", CTRL, CTRL, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(control_value(
            as_control(ui, &a)
                .and_then(|c| ui.control(c))
                .and_then(|c| c.parent),
        ))
    });
    r.unary("ctrlClassName", CTRL, STR, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(Value::from(
            as_control(ui, &a)
                .and_then(|c| ui.control(c))
                .map(|c| c.class_name.clone())
                .unwrap_or_default(),
        ))
    });
    r.unary("ctrlType", CTRL, NUM, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(Value::Number(
            as_control(ui, &a)
                .and_then(|c| ui.control(c))
                .map_or(-1.0, |c| c.kind.code() as f32),
        ))
    });
    r.unary("ctrlStyle", CTRL, NUM, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(Value::Number(
            as_control(ui, &a)
                .and_then(|c| ui.control(c))
                .map_or(-1.0, |c| c.style as f32),
        ))
    });
    r.unary("ctrlText", CTRL, STR, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(Value::from(
            as_control(ui, &a)
                .and_then(|c| ui.control(c))
                .map(|c| c.text.clone())
                .unwrap_or_default(),
        ))
    });
    r.unary("ctrlText", NUM, STR, |ctx, a| {
        let ui = ctx.host.ui();
        let text = ui
            .top()
            .and_then(|d| ui.find_control(d, num(&a) as i32))
            .and_then(|c| ui.control(c))
            .map(|c| c.text.clone())
            .unwrap_or_default();
        Ok(Value::from(text))
    });
    r.binary("ctrlSetText", CTRL, STR, NOTHING, |ctx, a, b| {
        let text = localize(&*ctx.host, &string(&b));
        let ui = ctx.host.ui_mut();
        if let Some(c) = as_control(ui, &a).and_then(|c| ui.control_mut(c)) {
            c.text = text;
        }
        Ok(Value::Nothing)
    });
    r.binary(
        "ctrlSetStructuredText",
        CTRL,
        TEXT | STR,
        NOTHING,
        |ctx, a, b| {
            let text = match &b {
                Value::String(s) => s.to_string(),
                other => ctx.to_display_string(other),
            };
            let ui = ctx.host.ui_mut();
            if let Some(c) = as_control(ui, &a).and_then(|c| ui.control_mut(c)) {
                c.text = text;
            }
            Ok(Value::Nothing)
        },
    );
    r.binary("ctrlSetTooltip", CTRL, STR, NOTHING, |ctx, a, b| {
        let text = localize(&*ctx.host, &string(&b));
        let ui = ctx.host.ui_mut();
        if let Some(c) = as_control(ui, &a).and_then(|c| ui.control_mut(c)) {
            c.tooltip = text;
        }
        Ok(Value::Nothing)
    });
    r.unary("ctrlTooltip", CTRL, STR, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(Value::from(
            as_control(ui, &a)
                .and_then(|c| ui.control(c))
                .map(|c| c.tooltip.clone())
                .unwrap_or_default(),
        ))
    });
    r.binary("ctrlShow", CTRL, BOOL, NOTHING, |ctx, a, b| {
        let ui = ctx.host.ui_mut();
        if let Some(c) = as_control(ui, &a).and_then(|c| ui.control_mut(c)) {
            c.show = matches!(b, Value::Bool(true));
        }
        Ok(Value::Nothing)
    });
    r.unary("ctrlShown", CTRL, BOOL, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(Value::Bool(
            as_control(ui, &a)
                .and_then(|c| ui.control(c))
                .is_some_and(|c| c.show),
        ))
    });
    r.unary("ctrlVisible", NUM, BOOL, |ctx, a| {
        let ui = ctx.host.ui();
        let visible = ui
            .top()
            .and_then(|d| ui.find_control(d, num(&a) as i32))
            .is_some_and(|c| ui.is_visible(c));
        Ok(Value::Bool(visible))
    });
    r.binary("ctrlEnable", CTRL, BOOL, NOTHING, |ctx, a, b| {
        let ui = ctx.host.ui_mut();
        if let Some(c) = as_control(ui, &a).and_then(|c| ui.control_mut(c)) {
            c.enabled = matches!(b, Value::Bool(true));
        }
        Ok(Value::Nothing)
    });
    r.unary("ctrlEnabled", CTRL, BOOL, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(Value::Bool(
            as_control(ui, &a)
                .and_then(|c| ui.control(c))
                .is_some_and(|c| c.enabled),
        ))
    });
    r.binary("ctrlSetFade", CTRL, NUM, NOTHING, |ctx, a, b| {
        let ui = ctx.host.ui_mut();
        if let Some(c) = as_control(ui, &a).and_then(|c| ui.control_mut(c)) {
            c.pending_fade = num(&b).clamp(0.0, 1.0);
        }
        Ok(Value::Nothing)
    });
    r.unary("ctrlFade", CTRL, NUM, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(Value::Number(
            as_control(ui, &a)
                .and_then(|c| ui.control(c))
                .map_or(0.0, |c| c.fade),
        ))
    });
    r.binary("ctrlSetPosition", CTRL, ARR, NOTHING, |ctx, a, b| {
        let ui = ctx.host.ui_mut();
        if let Some(c) = as_control(ui, &a) {
            set_position(ui, c, &items(&b));
        }
        Ok(Value::Nothing)
    });
    for (name, index) in [
        ("ctrlSetPositionX", 0usize),
        ("ctrlSetPositionY", 1),
        ("ctrlSetPositionW", 2),
        ("ctrlSetPositionH", 3),
    ] {
        let f: a3_sqf::registry::BinaryFn<H> = match index {
            0 => |ctx, a, b| set_axis(ctx, &a, 0, num(&b)),
            1 => |ctx, a, b| set_axis(ctx, &a, 1, num(&b)),
            2 => |ctx, a, b| set_axis(ctx, &a, 2, num(&b)),
            _ => |ctx, a, b| set_axis(ctx, &a, 3, num(&b)),
        };
        r.binary(name, CTRL, NUM, NOTHING, f);
    }
    r.unary("ctrlPosition", CTRL, ARR, |ctx, a| {
        let ui = ctx.host.ui();
        let pos = as_control(ui, &a)
            .and_then(|c| ui.control(c))
            .map(|c| c.position);
        Ok(array(
            pos.unwrap_or([0.0; 4])
                .iter()
                .map(|&v| Value::Number(v))
                .collect(),
        ))
    });
    r.binary("ctrlCommit", CTRL, NUM, NOTHING, |ctx, a, b| {
        let ui = ctx.host.ui_mut();
        if let Some(c) = as_control(ui, &a) {
            ui.commit(c, num(&b));
        }
        Ok(Value::Nothing)
    });
    r.unary("ctrlCommitted", CTRL, BOOL, |ctx, a| {
        let ui = ctx.host.ui();
        Ok(Value::Bool(
            as_control(ui, &a).is_none_or(|c| ui.committed(c)),
        ))
    });
    r.binary("ctrlSetTextColor", CTRL, ARR, NOTHING, |ctx, a, b| {
        let ui = ctx.host.ui_mut();
        if let (Some(c), Some(col)) = (
            as_control(ui, &a).and_then(|c| ui.control_mut(c)),
            color(&b),
        ) {
            c.color_text = col;
        }
        Ok(Value::Nothing)
    });
    r.binary("ctrlSetBackgroundColor", CTRL, ARR, NOTHING, |ctx, a, b| {
        let ui = ctx.host.ui_mut();
        if let (Some(c), Some(col)) = (
            as_control(ui, &a).and_then(|c| ui.control_mut(c)),
            color(&b),
        ) {
            c.color_background = col;
        }
        Ok(Value::Nothing)
    });
    r.binary("ctrlSetForegroundColor", CTRL, ARR, NOTHING, |ctx, a, b| {
        let ui = ctx.host.ui_mut();
        if let (Some(c), Some(col)) = (
            as_control(ui, &a).and_then(|c| ui.control_mut(c)),
            color(&b),
        ) {
            c.color_text = col;
        }
        Ok(Value::Nothing)
    });
    r.binary("ctrlSetActiveColor", CTRL, ARR, NOTHING, |ctx, a, b| {
        let ui = ctx.host.ui_mut();
        if let (Some(c), Some(col)) = (
            as_control(ui, &a).and_then(|c| ui.control_mut(c)),
            color(&b),
        ) {
            c.color_active = col;
        }
        Ok(Value::Nothing)
    });
    r.binary("ctrlSetFontHeight", CTRL, NUM, NOTHING, |ctx, a, b| {
        let ui = ctx.host.ui_mut();
        if let Some(c) = as_control(ui, &a).and_then(|c| ui.control_mut(c)) {
            c.size_ex = num(&b);
        }
        Ok(Value::Nothing)
    });
    r.binary("ctrlSetAngle", CTRL, ARR, NOTHING, |ctx, a, b| {
        let angle = items(&b).first().map_or(0.0, num);
        let ui = ctx.host.ui_mut();
        if let Some(c) = as_control(ui, &a).and_then(|c| ui.control_mut(c)) {
            c.angle = angle;
        }
        Ok(Value::Nothing)
    });
    r.binary("ctrlSetFocus", CTRL, ANY, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.unary("ctrlSetFocus", CTRL, NOTHING, |_, _| Ok(Value::Nothing));
    r.binary("ctrlSetURL", CTRL, STR, NOTHING, |ctx, a, b| {
        set_variable(ctx.host.ui_mut(), &a, "#url", Value::from(string(&b)));
        Ok(Value::Nothing)
    });
    r.unary("ctrlURL", CTRL, STR, |ctx, a| {
        Ok(get_variable(ctx.host.ui(), &a, "#url").unwrap_or_else(|| Value::from("")))
    });
    r.binary("ctrlSetURLOverlayMode", CTRL, BOOL, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.unary("ctrlTextHeight", CTRL, NUM, |ctx, a| {
        let Some(c) = as_control(ctx.host.ui(), &a) else {
            return Ok(Value::Number(0.0));
        };
        let Some((font, pixels, text, width)) = text_metrics(ctx, c) else {
            return Ok(Value::Number(0.0));
        };
        let viewport_h = ctx.host.ui().metrics.viewport_h;
        let lines = match ctx.host.ui_fonts() {
            Some(fonts) => {
                let (plain, _) = crate::draw::plain_structured_text(&text);
                fonts.wrap(&font, pixels, &plain, width).len()
            }
            None => 1,
        };
        Ok(Value::Number(lines as f32 * pixels / viewport_h))
    });
    r.unary("ctrlTextWidth", CTRL, NUM, |ctx, a| {
        let Some(c) = as_control(ctx.host.ui(), &a) else {
            return Ok(Value::Number(0.0));
        };
        let Some((font, pixels, text, _)) = text_metrics(ctx, c) else {
            return Ok(Value::Number(0.0));
        };
        let viewport_w = ctx.host.ui().metrics.viewport_w;
        let width = ctx
            .host
            .ui_fonts()
            .map_or(0.0, |f| f.measure(&font, pixels, &text));
        Ok(Value::Number(width / viewport_w))
    });
    r.binary("ctrlCreate", DISP, ARR, CTRL, |ctx, a, b| {
        let args = items(&b);
        let class = args.first().map(string).unwrap_or_default();
        let idc = args.get(1).map_or(-1.0, num) as i32;
        let ui = ctx.host.ui();
        let display = as_display(ui, &a);
        let group = args.get(2).and_then(|g| as_control(ui, g));
        let Some(display) = display else {
            return Ok(control_value(None));
        };
        Ok(control_value(create_control_ctx(
            ctx, &class, idc, display, group,
        )))
    });
    r.binary("ctrlDelete", ANY, CTRL, BOOL, |ctx, _, b| {
        let ui = ctx.host.ui_mut();
        Ok(Value::Bool(
            as_control(ui, &b).is_some_and(|c| ui.delete_control(c)),
        ))
    });
    r.unary("ctrlDelete", CTRL, BOOL, |ctx, a| {
        let ui = ctx.host.ui_mut();
        Ok(Value::Bool(
            as_control(ui, &a).is_some_and(|c| ui.delete_control(c)),
        ))
    });
    r.binary("setVariable", CTRL, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        if let Some(name) = args.first() {
            let value = args.get(1).cloned().unwrap_or(Value::Nil);
            set_variable(ctx.host.ui_mut(), &a, &string(name), value);
        }
        Ok(Value::Nothing)
    });
    r.binary("getVariable", CTRL, STR | ARR, ANY, |ctx, a, b| {
        let (name, default) = match &b {
            Value::Array(_) => {
                let args = items(&b);
                (
                    args.first().map(string).unwrap_or_default(),
                    args.get(1).cloned().unwrap_or(Value::Nil),
                )
            }
            other => (string(other), Value::Nil),
        };
        Ok(get_variable(ctx.host.ui(), &a, &name).unwrap_or(default))
    });
    r.binary("ctrlActivate", ANY, CTRL, NOTHING, |ctx, _, b| {
        activate(ctx, &b)
    });
    r.unary("ctrlActivate", CTRL, NOTHING, |ctx, a| activate(ctx, &a));
    r.nular("getMousePosition", ARR, |_| {
        Ok(array(vec![Value::Number(0.5), Value::Number(0.5)]))
    });
}

fn activate<H: UiHost>(ctx: &mut Ctx<'_, H>, v: &Value) -> Result<Value, SqfError> {
    let Some(c) = as_control(ctx.host.ui(), v) else {
        return Ok(Value::Nothing);
    };
    run_handlers_ctx(
        ctx,
        Target::Control(c),
        "buttonclick",
        vec![control_value(Some(c))],
    );
    let action = ctx
        .host
        .ui()
        .control(c)
        .map(|c| c.action.clone())
        .unwrap_or_default();
    if !action.trim().is_empty() {
        if let Ok(code) = ctx.compile("", &action) {
            let _ = ctx.call_unscheduled(&code, None);
        }
    }
    Ok(Value::Nothing)
}

fn set_axis<H: UiHost>(
    ctx: &mut Ctx<'_, H>,
    a: &Value,
    axis: usize,
    v: f32,
) -> Result<Value, SqfError> {
    let ui = ctx.host.ui_mut();
    if let Some(c) = as_control(ui, a).and_then(|c| ui.control_mut(c)) {
        c.pending_position[axis] = v;
    }
    Ok(Value::Nothing)
}

fn set_variable(ui: &mut Ui, target: &Value, name: &str, value: Value) {
    let name = name.to_ascii_lowercase();
    if let Some(c) = as_control(ui, target).and_then(|c| ui.control_mut(c)) {
        c.variables.retain(|(n, _)| *n != name);
        c.variables.push((name, value));
    }
}

fn get_variable(ui: &Ui, target: &Value, name: &str) -> Option<Value> {
    let name = name.to_ascii_lowercase();
    as_control(ui, target)
        .and_then(|c| ui.control(c))
        .and_then(|c| c.variables.iter().find(|(n, _)| *n == name))
        .map(|(_, v)| v.clone())
}

fn create_control_ctx<H: UiHost>(
    ctx: &mut Ctx<'_, H>,
    class: &str,
    idc: i32,
    display: DisplayId,
    group: Option<ControlId>,
) -> Option<ControlId> {
    let config = ctx.host.ui_config()?;
    let screen = ctx.host.ui().metrics.screen;
    let mut eval = CtxEval { ctx };
    let mut ui = std::mem::replace(eval.ctx.host.ui_mut(), Ui::new(screen));
    let result = ui.create_control(&config, class, idc, display, group, &mut eval);
    *eval.ctx.host.ui_mut() = ui;
    result
}

fn register_lists<H: UiHost>(r: &mut Registry<H>) {
    fn with_list<H: UiHost, R>(
        ctx: &mut Ctx<'_, H>,
        v: &Value,
        f: impl FnOnce(&mut crate::model::Control) -> R,
    ) -> Option<R> {
        let ui = ctx.host.ui_mut();
        let c = as_control(ui, v)?;
        ui.control_mut(c).map(f)
    }
    fn row(items: &[Value]) -> usize {
        items.first().map_or(0.0, num).max(0.0) as usize
    }
    r.binary("lbAdd", CTRL, STR, NUM, |ctx, a, b| {
        let text = localize(&*ctx.host, &string(&b));
        Ok(Value::Number(
            with_list(ctx, &a, |c| {
                c.items.push(ListItem {
                    text,
                    ..ListItem::default()
                });
                (c.items.len() - 1) as f32
            })
            .unwrap_or(-1.0),
        ))
    });
    r.unary("lbClear", CTRL, NOTHING, |ctx, a| {
        with_list(ctx, &a, |c| {
            c.items.clear();
            c.cur_sel = -1;
        });
        Ok(Value::Nothing)
    });
    r.unary("lbSize", CTRL, NUM, |ctx, a| {
        Ok(Value::Number(
            with_list(ctx, &a, |c| c.items.len() as f32).unwrap_or(0.0),
        ))
    });
    r.unary("lbCurSel", CTRL, NUM, |ctx, a| {
        Ok(Value::Number(
            with_list(ctx, &a, |c| c.cur_sel as f32).unwrap_or(-1.0),
        ))
    });
    r.binary("lbSetCurSel", CTRL, NUM, NOTHING, |ctx, a, b| {
        with_list(ctx, &a, |c| c.cur_sel = num(&b) as i32);
        Ok(Value::Nothing)
    });
    r.binary("lbText", CTRL, NUM, STR, |ctx, a, b| {
        let i = num(&b).max(0.0) as usize;
        Ok(Value::from(
            with_list(ctx, &a, |c| c.items.get(i).map(|it| it.text.clone()))
                .flatten()
                .unwrap_or_default(),
        ))
    });
    r.binary("lbData", CTRL, NUM, STR, |ctx, a, b| {
        let i = num(&b).max(0.0) as usize;
        Ok(Value::from(
            with_list(ctx, &a, |c| c.items.get(i).map(|it| it.data.clone()))
                .flatten()
                .unwrap_or_default(),
        ))
    });
    r.binary("lbValue", CTRL, NUM, NUM, |ctx, a, b| {
        let i = num(&b).max(0.0) as usize;
        Ok(Value::Number(
            with_list(ctx, &a, |c| c.items.get(i).map(|it| it.value))
                .flatten()
                .unwrap_or(0.0),
        ))
    });
    r.binary("lbSetData", CTRL, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        let (i, v) = (row(&args), args.get(1).map(string).unwrap_or_default());
        with_list(ctx, &a, |c| {
            if let Some(it) = c.items.get_mut(i) {
                it.data = v;
            }
        });
        Ok(Value::Nothing)
    });
    r.binary("lbSetValue", CTRL, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        let (i, v) = (row(&args), args.get(1).map_or(0.0, num));
        with_list(ctx, &a, |c| {
            if let Some(it) = c.items.get_mut(i) {
                it.value = v;
            }
        });
        Ok(Value::Nothing)
    });
    r.binary("lbSetText", CTRL, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        let (i, v) = (row(&args), args.get(1).map(string).unwrap_or_default());
        with_list(ctx, &a, |c| {
            if let Some(it) = c.items.get_mut(i) {
                it.text = v;
            }
        });
        Ok(Value::Nothing)
    });
    r.binary("lbSetPicture", CTRL, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        let (i, v) = (row(&args), args.get(1).map(string).unwrap_or_default());
        with_list(ctx, &a, |c| {
            if let Some(it) = c.items.get_mut(i) {
                it.picture = v;
            }
        });
        Ok(Value::Nothing)
    });
    r.binary("lbSetTooltip", CTRL, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        let (i, v) = (row(&args), args.get(1).map(string).unwrap_or_default());
        with_list(ctx, &a, |c| {
            if let Some(it) = c.items.get_mut(i) {
                it.tooltip = v;
            }
        });
        Ok(Value::Nothing)
    });
    r.binary("lbSetColor", CTRL, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        let (i, col) = (row(&args), args.get(1).and_then(color));
        with_list(ctx, &a, |c| {
            if let Some(it) = c.items.get_mut(i) {
                it.color = col;
            }
        });
        Ok(Value::Nothing)
    });
    r.binary("lbDelete", CTRL, NUM, NOTHING, |ctx, a, b| {
        let i = num(&b).max(0.0) as usize;
        with_list(ctx, &a, |c| {
            if i < c.items.len() {
                c.items.remove(i);
            }
        });
        Ok(Value::Nothing)
    });
    r.binary("sliderSetRange", CTRL, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        let range = [args.first().map_or(0.0, num), args.get(1).map_or(1.0, num)];
        with_list(ctx, &a, |c| c.range = range);
        Ok(Value::Nothing)
    });
    r.binary("sliderSetPosition", CTRL, NUM, NOTHING, |ctx, a, b| {
        with_list(ctx, &a, |c| c.value = num(&b));
        Ok(Value::Nothing)
    });
    r.unary("sliderPosition", CTRL, NUM, |ctx, a| {
        Ok(Value::Number(
            with_list(ctx, &a, |c| c.value).unwrap_or(0.0),
        ))
    });
    r.binary("progressSetPosition", CTRL, NUM, NOTHING, |ctx, a, b| {
        with_list(ctx, &a, |c| c.value = num(&b));
        Ok(Value::Nothing)
    });
    r.unary("progressPosition", CTRL, NUM, |ctx, a| {
        Ok(Value::Number(
            with_list(ctx, &a, |c| c.value).unwrap_or(0.0),
        ))
    });
}

fn register_events<H: UiHost>(r: &mut Registry<H>) {
    r.binary("ctrlAddEventHandler", CTRL, ARR, NUM, |ctx, a, b| {
        let args = items(&b);
        let Some(c) = as_control(ctx.host.ui(), &a) else {
            return Ok(Value::Number(-1.0));
        };
        let event = args.first().map(string).unwrap_or_default();
        let code = args.get(1).cloned().unwrap_or(Value::Nil);
        Ok(Value::Number(
            add_handler(ctx, Target::Control(c), &event, code) as f32,
        ))
    });
    r.binary("displayAddEventHandler", DISP, ARR, NUM, |ctx, a, b| {
        let args = items(&b);
        let Some(d) = as_display(ctx.host.ui(), &a) else {
            return Ok(Value::Number(-1.0));
        };
        let event = args.first().map(string).unwrap_or_default();
        let code = args.get(1).cloned().unwrap_or(Value::Nil);
        Ok(Value::Number(
            add_handler(ctx, Target::Display(d), &event, code) as f32,
        ))
    });
    r.binary("ctrlSetEventHandler", CTRL, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        let Some(c) = as_control(ctx.host.ui(), &a) else {
            return Ok(Value::Nothing);
        };
        let event = args
            .first()
            .map(string)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let code = args.get(1).cloned().unwrap_or(Value::Nil);
        if let Some(ctrl) = ctx.host.ui_mut().control_mut(c) {
            ctrl.events.retain(|h| h.event != event || h.id >= 0);
        }
        if !string(&code).is_empty() {
            add_handler(ctx, Target::Control(c), &event, code);
        }
        Ok(Value::Nothing)
    });
    r.binary("displaySetEventHandler", DISP, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        let Some(d) = as_display(ctx.host.ui(), &a) else {
            return Ok(Value::Nothing);
        };
        let event = args
            .first()
            .map(string)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let code = args.get(1).cloned().unwrap_or(Value::Nil);
        if let Some(disp) = ctx.host.ui_mut().display_mut(d) {
            disp.events.retain(|h| h.event != event || h.id >= 0);
        }
        if !string(&code).is_empty() {
            add_handler(ctx, Target::Display(d), &event, code);
        }
        Ok(Value::Nothing)
    });
    r.binary("ctrlRemoveEventHandler", CTRL, ARR, NOTHING, |ctx, a, b| {
        let args = items(&b);
        let event = args
            .first()
            .map(string)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let id = args.get(1).map_or(-2.0, num) as i32;
        let ui = ctx.host.ui_mut();
        if let Some(c) = as_control(ui, &a).and_then(|c| ui.control_mut(c)) {
            c.events.retain(|h| !(h.event == event && h.id == id));
        }
        Ok(Value::Nothing)
    });
    r.binary(
        "ctrlRemoveAllEventHandlers",
        CTRL,
        STR,
        NOTHING,
        |ctx, a, b| {
            let event = string(&b).to_ascii_lowercase();
            let ui = ctx.host.ui_mut();
            if let Some(c) = as_control(ui, &a).and_then(|c| ui.control_mut(c)) {
                c.events.retain(|h| h.event != event || h.id < 0);
            }
            Ok(Value::Nothing)
        },
    );
    r.binary(
        "displayRemoveEventHandler",
        DISP,
        ARR,
        NOTHING,
        |ctx, a, b| {
            let args = items(&b);
            let event = args
                .first()
                .map(string)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let id = args.get(1).map_or(-2.0, num) as i32;
            let ui = ctx.host.ui_mut();
            if let Some(d) = as_display(ui, &a).and_then(|d| ui.display_mut(d)) {
                d.events.retain(|h| !(h.event == event && h.id == id));
            }
            Ok(Value::Nothing)
        },
    );
    r.binary(
        "displayRemoveAllEventHandlers",
        DISP,
        STR,
        NOTHING,
        |ctx, a, b| {
            let event = string(&b).to_ascii_lowercase();
            let ui = ctx.host.ui_mut();
            if let Some(d) = as_display(ui, &a).and_then(|d| ui.display_mut(d)) {
                d.events.retain(|h| h.event != event || h.id < 0);
            }
            Ok(Value::Nothing)
        },
    );
    let _ = CODE;
}
