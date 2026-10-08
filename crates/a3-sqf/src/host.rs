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

    /// `hint` / `hintSilent`.
    fn hint(&mut self, _text: &str) {}

    /// `copyToClipboard`.
    fn copy_to_clipboard(&mut self, _text: &str) {}

    /// The error sink: a script error, already formatted engine-style in
    /// [`ScriptError::report`].
    fn report_error(&mut self, _error: &ScriptError) {}

    /// Reads a file from the game's virtual file system (`loadFile`).
    /// `path` is an in-game path (backslash-separated, case-insensitive).
    fn load_file(&mut self, path: &str) -> Result<String, String> {
        Err(format!("Script {path} not found"))
    }

    /// Reads and preprocesses a script file (`preprocessFile`,
    /// `preprocessFileLineNumbers`, `execVM`). With `line_numbers`, the
    /// output carries `#line` directives. The default loads the file
    /// without preprocessing.
    fn preprocess_file(&mut self, path: &str, _line_numbers: bool) -> Result<String, String> {
        self.load_file(path)
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
}

/// A host with all defaults, for tools and tests.
#[derive(Debug, Default)]
pub struct NullHost;

impl Host for NullHost {}
