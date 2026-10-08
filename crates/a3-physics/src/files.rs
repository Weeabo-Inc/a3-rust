//! Where models and surface files come from.

use std::collections::HashMap;

/// Read access to game files by in-game path (backslash-separated, case-insensitive).
pub trait FileSource: Send + Sync {
    /// The file's bytes, or `None` if there is no such file.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
}

impl FileSource for a3_vfs::Vfs {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.open(path).ok().map(|b| b.to_vec())
    }
}

/// Files held in memory, for tests and tools.
#[derive(Debug, Clone, Default)]
pub struct MemoryFiles {
    files: HashMap<String, Vec<u8>>,
}

impl MemoryFiles {
    pub fn insert(&mut self, path: &str, bytes: impl Into<Vec<u8>>) {
        self.files.insert(normalize(path), bytes.into());
    }
}

impl FileSource for MemoryFiles {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(&normalize(path)).cloned()
    }
}

/// Lower case, backslashes, no leading backslash: the key under which a path is cached.
pub(crate) fn normalize(path: &str) -> String {
    path.trim_start_matches(['\\', '/'])
        .replace('/', "\\")
        .to_ascii_lowercase()
}
