//! Where the texture cache gets its file bytes: the mounted game VFS, or any other source.

use std::collections::HashMap;
use std::path::Path;

use a3_vfs::{Vfs, VfsPath};

/// A source of texture files.
///
/// Paths are already normalized by [`a3_ui::draw::normalize_texture_path`]: no leading
/// separator, backslash-separated, lower case. `#(...)` procedural textures are never read
/// from here. `Send + Sync` because the asset source moves into the render feature.
pub trait UiAssets: Send + Sync {
    /// The bytes of `path`, or `None` when it does not exist.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
}

impl UiAssets for Vfs {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.open(path).ok().map(|bytes| bytes.to_vec())
    }
}

impl<T: UiAssets + ?Sized> UiAssets for Box<T> {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        (**self).read(path)
    }
}

impl<T: UiAssets + ?Sized> UiAssets for std::sync::Arc<T> {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        (**self).read(path)
    }
}

/// An in-memory asset set: tests, generated textures, or a preloaded subset of the VFS.
#[derive(Debug, Clone, Default)]
pub struct MemoryAssets {
    files: HashMap<String, Vec<u8>>,
}

impl MemoryAssets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file under its normalized name.
    pub fn insert(&mut self, path: &str, bytes: Vec<u8>) {
        self.files
            .insert(a3_ui::draw::normalize_texture_path(path), bytes);
    }

    /// Adds a file from the loose file `path` on disk (its file name is the key).
    pub fn insert_file(&mut self, path: &Path) -> std::io::Result<()> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.insert(&name, std::fs::read(path)?);
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

impl UiAssets for MemoryAssets {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(path).cloned()
    }
}

/// A VFS with `dir` mounted at its root, for tests and tools that only need loose files.
pub fn vfs_with_dir(dir: &Path) -> Vfs {
    let vfs = Vfs::new();
    let _ = vfs.mount_dir(dir, VfsPath::root());
    vfs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_assets_normalize_names_and_miss_cleanly() {
        let mut assets = MemoryAssets::new();
        assets.insert(r"A3\UI_F\Data\X_CA.paa", vec![1, 2, 3]);
        assert_eq!(assets.read(r"a3\ui_f\data\x_ca.paa"), Some(vec![1, 2, 3]));
        assert_eq!(assets.read("a3\\ui_f\\data\\nope.paa"), None);
    }

    #[test]
    fn a_vfs_serves_a_mounted_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("icon.paa"), b"hello").unwrap();
        let vfs = vfs_with_dir(dir.path());
        assert_eq!(vfs.read("icon.paa"), Some(b"hello".to_vec()));
        assert_eq!(vfs.read("missing.paa"), None);
    }
}
