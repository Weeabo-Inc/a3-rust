//! [`InGameUi`]: the HUD layer, its displays and the per-frame update.

use std::sync::Arc;

use a3_config::ConfigTree;
use a3_gamedata::Localizer;
use a3_sqf::Vm;
use a3_ui::commands::{VmEval, fire_load_events};
use a3_ui::{DisplayId, DrawList, Fonts, Screen, Ui, build_draw_list};
use a3_vfs::Vfs;

use crate::colors::IguiColors;
use crate::host::{IguiHost, igui_vm, init_profile_colors};
use crate::idc;
use crate::stance::{Stance, StanceAdjust, StanceState, StanceTextures};
use crate::unit_info::{FillContext, Formats, ModeTextures, UnitInfo, driven_control, fill};

/// The difficulty options the HUD follows (`CfgDifficultyPresets >> <preset> >> Options`):
/// 0 never, 1 fade, 2 always.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudOptions {
    /// `weaponInfo`: the unit info displays.
    pub weapon_info: u8,
    /// `stanceIndicator`.
    pub stance_indicator: u8,
}

impl Default for HudOptions {
    /// The `Regular` preset.
    fn default() -> Self {
        HudOptions {
            weapon_info: 2,
            stance_indicator: 2,
        }
    }
}

impl HudOptions {
    /// The options of `preset`, or of `CfgDifficultyPresets >> defaultPreset` when `None`.
    pub fn from_config(config: &ConfigTree, preset: Option<&str>) -> Self {
        let presets = config.root().get("CfgDifficultyPresets");
        let name = match preset {
            Some(p) => p.to_owned(),
            None => presets.get("defaultPreset").text(),
        };
        let options = presets.get(&name).get("Options");
        let mut out = HudOptions::default();
        for (key, slot) in [
            ("weaponInfo", &mut out.weapon_info),
            ("stanceIndicator", &mut out.stance_indicator),
        ] {
            let e = options.get(key);
            if e.is_number() {
                *slot = e.number().clamp(0.0, 2.0) as u8;
            }
        }
        out
    }
}

/// How long after a change a fading HUD element stays, then fades
/// (`CfgInGameUI >> PlayerInfo >> dimmStartTime`, `dimmEndTime`; `FUN_140a0c990`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dimm {
    pub start: f32,
    pub end: f32,
}

impl Default for Dimm {
    fn default() -> Self {
        Dimm {
            start: 5.0,
            end: 10.0,
        }
    }
}

impl Dimm {
    pub fn from_config(config: &ConfigTree) -> Self {
        let info = config.root().get("CfgInGameUI").get("PlayerInfo");
        let mut d = Dimm::default();
        for (key, slot) in [("dimmStartTime", &mut d.start), ("dimmEndTime", &mut d.end)] {
            let e = info.get(key);
            if e.is_number() {
                *slot = e.number();
            }
        }
        d
    }

    /// The opacity `t` seconds after the last change: 1 until `start`, then linearly down to 0
    /// at `end`.
    pub fn alpha(&self, t: f32) -> f32 {
        if (0.0..self.start).contains(&t) {
            1.0
        } else if (0.0..self.end).contains(&t) {
            (self.end - t) / (self.end - self.start)
        } else {
            0.0
        }
    }
}

/// The in-game UI: the unit info displays, the stance indicator and the state they follow.
pub struct InGameUi {
    vm: Vm<IguiHost>,
    ctx: FillContext,
    stance_textures: StanceTextures,
    pub options: HudOptions,
    pub dimm: Dimm,
    /// One slot per `unitInfoType` entry; `None` for a class `RscInGameUI` lacks.
    unit_info: Vec<(String, Option<DisplayId>)>,
    stance: Option<DisplayId>,
    last_stance: Option<(Stance, StanceState, StanceAdjust)>,
    stance_changed: f64,
}

impl std::fmt::Debug for InGameUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InGameUi")
            .field("unit_info", &self.unit_info)
            .field("stance", &self.stance)
            .field("options", &self.options)
            .finish()
    }
}

impl InGameUi {
    /// The in-game UI over `configFile`, reading scripts from `vfs` and texts through
    /// `localizer`. Brings the profile's UI colours up to date (`BIS_fnc_displayColorGet`)
    /// and reads the HUD colours and textures, as `InGameUI::Init` does.
    pub fn new(
        config: Arc<ConfigTree>,
        vfs: Option<Vfs>,
        localizer: Option<Arc<dyn Localizer>>,
        screen: Screen,
    ) -> Self {
        let host = IguiHost::new(Arc::clone(&config), vfs, localizer, screen);
        let mut vm = igui_vm(host);
        init_profile_colors(&mut vm);
        let colors = IguiColors::from_config(&config, &mut VmEval { vm: &mut vm });
        let formats = {
            let host = &vm.host;
            Formats::localized(|key| host.localize_key(key))
        };
        InGameUi {
            ctx: FillContext {
                colors,
                mode_textures: ModeTextures::from_config(&config),
                formats,
            },
            stance_textures: StanceTextures::from_config(&config),
            options: HudOptions::from_config(&config, None),
            dimm: Dimm::from_config(&config),
            vm,
            unit_info: Vec::new(),
            stance: None,
            last_stance: None,
            stance_changed: 0.0,
        }
    }

    /// The layer's displays and controls.
    pub fn ui(&self) -> &Ui {
        &self.vm.host.ui
    }

    /// The VM the layer's scripts run on.
    pub fn vm(&mut self) -> &mut Vm<IguiHost> {
        &mut self.vm
    }

    /// The colours, textures and formats the unit info is filled with.
    pub fn context(&self) -> &FillContext {
        &self.ctx
    }

    /// The unit info displays with their `RscInGameUI` class names (`None` for a class that
    /// does not exist).
    pub fn unit_info_displays(&self) -> &[(String, Option<DisplayId>)] {
        &self.unit_info
    }

    /// The stance indicator display, once shown.
    pub fn stance_display(&self) -> Option<DisplayId> {
        self.stance
    }

    /// Changes the screen. The displays are rebuilt, since their layout expressions depend on
    /// it.
    pub fn set_screen(&mut self, screen: Screen) {
        if self.vm.host.ui.metrics.screen == screen {
            return;
        }
        for (_, d) in std::mem::take(&mut self.unit_info) {
            if let Some(d) = d {
                self.vm.host.ui.remove_display(d);
            }
        }
        if let Some(d) = self.stance.take() {
            self.vm.host.ui.remove_display(d);
        }
        self.vm.host.ui.set_screen(screen);
    }

    /// Builds `RscInGameUI >> class` and runs its load handlers.
    fn open(&mut self, class: &str) -> Option<DisplayId> {
        let config = Arc::clone(self.vm.host.config());
        let screen = self.vm.host.ui.metrics.screen;
        let mut ui = std::mem::replace(&mut self.vm.host.ui, Ui::new(screen));
        let result = ui.create_display_at(
            &config,
            &["RscInGameUI", class],
            None,
            false,
            &mut VmEval { vm: &mut self.vm },
        );
        self.vm.host.ui = ui;
        let d = result.ok()?;
        fire_load_events(&mut self.vm, d);
        Some(d)
    }

    /// Brings the displays up to date with `info` at UI time `time` (seconds).
    pub fn update(&mut self, info: &UnitInfo, time: f64) {
        self.vm.host.ui.update(time);
        self.sync_unit_info(&info.unit_info_types);
        let unit_alpha = if self.options.weapon_info == 0 && info.on_foot {
            0.0
        } else {
            1.0
        };
        for i in 0..self.unit_info.len() {
            let Some(d) = self.unit_info[i].1 else {
                continue;
            };
            let ui = &mut self.vm.host.ui;
            fill(ui, d, info, &self.ctx);
            if let Some(display) = ui.display_mut(d) {
                display.alpha = unit_alpha;
            }
        }
        self.update_stance(info, time);
    }

    /// Keeps one display per `unitInfoType` entry: when the list changes, the displays are
    /// rebuilt from config (the engine reloads each slot whose class name changed).
    fn sync_unit_info(&mut self, types: &[String]) {
        if self.unit_info.iter().map(|(name, _)| name).eq(types.iter()) {
            return;
        }
        for (_, d) in std::mem::take(&mut self.unit_info) {
            if let Some(d) = d {
                self.vm.host.ui.remove_display(d);
            }
        }
        for class in types {
            // A class missing from `RscInGameUI` shows nothing.
            let d = self.open(class);
            self.unit_info.push((class.clone(), d));
        }
    }

    fn update_stance(&mut self, info: &UnitInfo, time: f64) {
        if self.stance.is_none() {
            self.stance = self.open("RscStanceInfo");
            self.stance_changed = time;
        }
        let Some(d) = self.stance else { return };
        let key = (info.stance, info.stance_state, info.stance_adjust);
        if self.last_stance != Some(key) {
            self.last_stance = Some(key);
            self.stance_changed = time;
        }
        let alpha = if !info.on_foot {
            0.0
        } else {
            match self.options.stance_indicator {
                0 => 0.0,
                1 => self.dimm.alpha((time - self.stance_changed) as f32),
                _ => 1.0,
            }
        };
        let texture = self
            .stance_textures
            .texture(info.stance_state, info.stance, info.stance_adjust)
            .unwrap_or_default()
            .to_owned();
        let ui = &mut self.vm.host.ui;
        if let Some(c) = driven_control(ui, d, idc::STANCE_INDICATOR)
            && let Some(c) = ui.control_mut(c)
        {
            c.text = texture;
        }
        if let Some(display) = ui.display_mut(d) {
            display.alpha = alpha;
        }
    }

    /// The quads of the layer, in drawing order.
    pub fn draw(&self, fonts: &mut Fonts) -> DrawList {
        build_draw_list(&self.vm.host.ui, fonts)
    }
}
