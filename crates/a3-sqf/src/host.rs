//! Services the VM needs from the engine that embeds it.
//!
//! The VM is generic over a [`Host`]. Every method has a default, so a test
//! host only overrides what it needs. Crates that add world, UI or config
//! commands require more of the host with their own traits:
//!
//! ```ignore
//! // in a3-world
//! pub trait WorldHost: a3_sqf::Host {
//!     fn world(&mut self) -> &mut World;
//! }
//! pub fn register<H: WorldHost>(reg: &mut a3_sqf::Registry<H>) {
//!     reg.nular("player", TypeSet::of(Type::Object), |ctx| { /* ctx.host.world() */ });
//! }
//! ```

use crate::error::ScriptError;
use crate::value::Handle;

/// Where `publicVariable` and its family send a variable (`publicVariable`
/// 0x547370, `publicVariableServer` 0x5474b0, `publicVariableClient`
/// 0x5473f0: all three call the network manager's `PublicVariable(name,
/// target)` vtable slot `+0x870`, with target 0, 2 or a client id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PublicTarget {
    /// Every machine, the server and all clients (`publicVariable`).
    All,
    /// The server only (`publicVariableServer`).
    Server,
    /// One client, by the id `owner` returns (`publicVariableClient`).
    Client(i32),
}

/// Engine services available to script commands.
pub trait Host: 'static {
    /// Mission time in seconds (`time`). Drives `sleep`.
    fn time(&self) -> f32 {
        0.0
    }

    /// Server time in seconds (`serverTime`).
    fn server_time(&self) -> f32 {
        self.time()
    }

    /// Real time in seconds since the engine started (`diag_tickTime`).
    /// Drives `uiSleep`.
    fn tick_time(&self) -> f32 {
        0.0
    }

    /// Number of frames rendered so far (`diag_frameNo`).
    fn frame_no(&self) -> u32 {
        0
    }

    /// `diag_log`: a line for the RPT log.
    fn diag_log(&mut self, _text: &str) {}

    /// `systemChat`: a line in the chat area.
    fn system_chat(&mut self, _text: &str) {}

    /// `hint`.
    fn hint(&mut self, _text: &str) {}

    /// `hintSilent`: a hint without the hint sound. Defaults to [`Host::hint`].
    fn hint_silent(&mut self, text: &str) {
        self.hint(text);
    }

    /// `copyToClipboard`.
    fn copy_to_clipboard(&mut self, _text: &str) {}

    /// `copyFromClipboard`.
    fn clipboard(&mut self) -> String {
        String::new()
    }

    /// `diag_fps` (frames per second, averaged by the engine).
    fn fps(&self) -> f32 {
        0.0
    }

    /// `diag_fpsmin`.
    fn fps_min(&self) -> f32 {
        self.fps()
    }

    /// `diag_deltaTime`: real time of the last frame in seconds.
    fn delta_time(&self) -> f32 {
        0.0
    }

    /// `fileExists`: whether a script-visible file exists.
    fn file_exists(&mut self, path: &str) -> bool {
        self.load_file(path).is_ok()
    }

    /// `saveProfileNamespace`: persist `profileNamespace`.
    fn save_profile_namespace(&mut self, _vars: &crate::vm::Variables) {}

    /// `saveMissionProfileNamespace`; returns whether it was saved.
    fn save_mission_profile_namespace(&mut self, _vars: &crate::vm::Variables) -> bool {
        false
    }

    /// The error sink: a script error, already formatted engine-style in
    /// [`ScriptError::report`].
    fn report_error(&mut self, _error: &ScriptError) {}

    /// Reads a file from the game's virtual file system (`loadFile`).
    /// `path` is an in-game path (backslash-separated, case-insensitive).
    fn load_file(&mut self, path: &str) -> Result<String, String> {
        Err(format!("Script {path} not found"))
    }

    /// Reads and preprocesses a script file (`preprocessFile`,
    /// `preprocessFileLineNumbers`, `execVM`, `compileScript`). With
    /// `line_numbers`, the output carries `#line` directives. The default
    /// runs the RV preprocessor ([`crate::preprocess::preprocess_with_host`]),
    /// reading the file and its includes with [`Host::load_file`].
    fn preprocess_file(&mut self, path: &str, line_numbers: bool) -> Result<String, String> {
        crate::preprocess::preprocess_with_host(self, path, line_numbers)
    }

    /// The stringtable text for `key` (`STR_...`, without `$`, any case), or `None` when the key
    /// is unknown (`localize`). The default knows no keys.
    fn localize(&self, _key: &str) -> Option<String> {
        None
    }

    /// The text `str` gives for a host handle.
    fn format_handle(&self, handle: Handle) -> String {
        handle.to_string()
    }

    /// Whether a handle refers to nothing (`isNull`). The default treats
    /// only id 0 as null; a world host also reports deleted objects.
    fn is_null(&self, handle: Handle) -> bool {
        handle.is_null()
    }

    /// `publicVariable`, `publicVariableServer` and `publicVariableClient`:
    /// hand the current value of the `missionNamespace` variable to the
    /// network. The engine calls its network manager here
    /// (`FUN_140547370`, `FUN_1405474b0`, `FUN_1405473f0` all end in the
    /// `+0x870` vtable slot with the name and a target). There is no
    /// transport yet (#131), so the default records nothing and sends
    /// nothing: headless the original has no client to send to either, and
    /// neither raises an error.
    ///
    /// The value is *not* looked up or validated here: the original's
    /// manager resolves the name itself, which is why publishing an
    /// undefined variable is not an error (server oracle,
    /// `tools/oracle/probes/15_publicvariable.probes`). An empty name is
    /// rejected by the command before this is called.
    fn publish_variable(&mut self, name: &str, target: PublicTarget) {
        let _ = (name, target);
    }
}

/// A host with all defaults, for tools and tests.
#[derive(Debug, Default)]
pub struct NullHost;

impl Host for NullHost {}
