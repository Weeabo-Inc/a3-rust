//! The script host of the in-game UI layer: its own [`Ui`], `configFile`, the game files and the
//! stringtables. Config expressions of the HUD displays (`safeZoneX`, `profileNamespace
//! getVariable [...]`) and their `onLoad` handlers run on a VM over it.

use std::rc::Rc;
use std::sync::Arc;

use a3_config::ConfigTree;
use a3_gamedata::{ConfigHost, Localizer, SqfConfigs, read_text, script_registry};
use a3_sqf::{Handle, HandleKind, Host, ScriptError, Value, Vm};
use a3_ui::{Screen, Ui, UiHost, register_ui_commands};
use a3_vfs::Vfs;

/// The host of the in-game UI's VM.
pub struct IguiHost {
    pub ui: Ui,
    config: Arc<ConfigTree>,
    configs: SqfConfigs,
    vfs: Option<Vfs>,
    localizer: Option<Arc<dyn Localizer>>,
    /// Script errors reported so far (missing functions in `onLoad`, ...).
    pub errors: Vec<String>,
}

impl std::fmt::Debug for IguiHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IguiHost")
            .field("ui", &self.ui)
            .field("vfs", &self.vfs.is_some())
            .field("localizer", &self.localizer.is_some())
            .field("errors", &self.errors.len())
            .finish()
    }
}

impl IguiHost {
    pub fn new(
        config: Arc<ConfigTree>,
        vfs: Option<Vfs>,
        localizer: Option<Arc<dyn Localizer>>,
        screen: Screen,
    ) -> Self {
        IguiHost {
            ui: Ui::new(screen),
            configs: SqfConfigs::new(Arc::clone(&config)),
            config,
            vfs,
            localizer,
            errors: Vec::new(),
        }
    }

    /// `configFile`.
    pub fn config(&self) -> &Arc<ConfigTree> {
        &self.config
    }

    /// The localized text of `key` (`STR_...`, without `$`).
    pub fn localize_key(&self, key: &str) -> Option<String> {
        self.localizer.as_ref()?.localize(key)
    }
}

impl Host for IguiHost {
    fn localize(&self, key: &str) -> Option<String> {
        self.localize_key(key)
    }

    fn report_error(&mut self, error: &ScriptError) {
        self.errors.push(error.report.clone());
    }

    fn load_file(&mut self, path: &str) -> Result<String, String> {
        let vfs = self
            .vfs
            .as_ref()
            .ok_or_else(|| format!("Script {path} not found"))?;
        read_text(vfs, path).map_err(|_| format!("Script {path} not found"))
    }

    fn format_handle(&self, handle: Handle) -> String {
        if handle.kind == HandleKind::Config {
            self.configs.format(handle)
        } else {
            handle.to_string()
        }
    }
}

impl ConfigHost for IguiHost {
    fn configs(&self) -> &SqfConfigs {
        &self.configs
    }

    fn configs_mut(&mut self) -> &mut SqfConfigs {
        &mut self.configs
    }
}

impl UiHost for IguiHost {
    fn ui(&self) -> &Ui {
        &self.ui
    }

    fn ui_mut(&mut self) -> &mut Ui {
        &mut self.ui
    }

    fn ui_config(&self) -> Option<Arc<ConfigTree>> {
        Some(Arc::clone(&self.config))
    }
}

/// A VM over `host` with the game's script commands, the config commands and the UI commands.
pub fn igui_vm(host: IguiHost) -> Vm<IguiHost> {
    let mut registry = script_registry::<IguiHost>();
    // After the headless registry, so the real UI commands (`findDisplay`, ...) win.
    register_ui_commands(&mut registry);
    Vm::with_registry(host, Rc::new(registry))
}

/// The script that keeps the profile's UI colours (`IGUI_TEXT_RGB_R`, ...) in step with
/// `CfgUIColors` (its default presets for a new profile).
pub const DISPLAY_COLOR_GET: &str = r"a3\functions_f\gui\fn_displayColorGet.sqf";

/// Runs `[true] call BIS_fnc_displayColorGet`, which writes every `<TAG>_<VAR>_R/G/B/A`
/// profile variable of `CfgUIColors` that is missing or no longer matches its preset. The game
/// runs it before the HUD colours are read. Returns `false` when the script is not in the game
/// files or fails.
pub fn init_profile_colors(vm: &mut Vm<IguiHost>) -> bool {
    let Ok(source) = vm.host.preprocess_file(DISPLAY_COLOR_GET, true) else {
        return false;
    };
    let Ok(code) = vm.compile_file(DISPLAY_COLOR_GET, &source) else {
        return false;
    };
    let args = Value::Array(a3_sqf::Array::from_vec(vec![Value::Bool(true)]));
    vm.call(&code, Some(args)).is_ok()
}
