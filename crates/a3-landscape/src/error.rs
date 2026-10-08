//! Errors of this crate.

/// Errors from reading world configs and layer materials.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `CfgWorlds` has no class of that name.
    #[error("CfgWorlds has no world class {0:?}")]
    NoWorld(String),

    /// A layer material cannot be read from the VFS.
    #[error("cannot read material {path}: {source}")]
    Vfs {
        /// The rvmat path.
        path: String,
        /// The VFS error.
        source: a3_vfs::Error,
    },

    /// A layer material is not a valid config.
    #[error("cannot parse material {path}: {detail}")]
    Parse {
        /// The rvmat path.
        path: String,
        /// The parser's message.
        detail: String,
    },

    /// A layer material lacks something every terrain material has.
    #[error("material {path}: {detail}")]
    Material {
        /// The rvmat path.
        path: String,
        /// What is missing.
        detail: String,
    },
}
