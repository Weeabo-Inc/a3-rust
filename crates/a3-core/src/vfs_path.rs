//! In-game paths: backslash-separated, case-insensitive paths inside the VFS.
//!
//! The engine treats paths inside PBOs and the VFS as case-insensitive and backslash-separated.
//! [`VfsPath`] holds a path in canonical form so that equal paths compare equal byte for byte:
//!
//! - ASCII letters are lower-cased (non-ASCII text is kept as is),
//! - `/` becomes `\`,
//! - leading, trailing and repeated separators are removed.
//!
//! The empty path is the VFS root. A `VfsPath` is never an OS path; convert explicitly.

use std::borrow::Borrow;
use std::fmt;

/// The in-game path separator.
pub const SEPARATOR: char = '\\';

/// A normalised, case-insensitive path inside the VFS (for example `a3\data_f\config.bin`).
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VfsPath(String);

impl VfsPath {
    /// The VFS root (the empty path).
    pub const fn root() -> Self {
        Self(String::new())
    }

    /// Normalises `path` into canonical form.
    pub fn new(path: &str) -> Self {
        let mut out = String::with_capacity(path.len());
        for component in path.split(['\\', '/']).filter(|c| !c.is_empty()) {
            if !out.is_empty() {
                out.push(SEPARATOR);
            }
            out.extend(component.chars().map(|c| c.to_ascii_lowercase()));
        }
        Self(out)
    }

    /// The canonical text of this path.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns `true` for the VFS root.
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Appends `child` (normalised) to this path.
    pub fn join(&self, child: &str) -> Self {
        let child = Self::new(child);
        match (self.is_root(), child.is_root()) {
            (true, _) => child,
            (false, true) => self.clone(),
            (false, false) => Self(format!("{}{SEPARATOR}{}", self.0, child.0)),
        }
    }

    /// The path without its last component, or `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        Some(match self.0.rfind(SEPARATOR) {
            Some(i) => Self(self.0[..i].to_owned()),
            None => Self::root(),
        })
    }

    /// The last component, or `None` for the root.
    pub fn file_name(&self) -> Option<&str> {
        if self.is_root() {
            return None;
        }
        self.0.rsplit(SEPARATOR).next()
    }

    /// The text after the last `.` of the file name, if any.
    pub fn extension(&self) -> Option<&str> {
        let name = self.file_name()?;
        let dot = name.rfind('.')?;
        (dot > 0).then(|| &name[dot + 1..])
    }

    /// Returns `true` when `self` equals `dir` or lies below it, component-wise.
    pub fn starts_with(&self, dir: &VfsPath) -> bool {
        if dir.is_root() {
            return true;
        }
        match self.0.strip_prefix(&dir.0) {
            Some(rest) => rest.is_empty() || rest.starts_with(SEPARATOR),
            None => false,
        }
    }

    /// The components of this path; empty for the root.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split(SEPARATOR).filter(|c| !c.is_empty())
    }

    /// Consumes the path and returns its canonical text.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for VfsPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for VfsPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VfsPath({:?})", self.0)
    }
}

impl From<&str> for VfsPath {
    fn from(path: &str) -> Self {
        Self::new(path)
    }
}

impl From<&String> for VfsPath {
    fn from(path: &String) -> Self {
        Self::new(path)
    }
}

impl AsRef<str> for VfsPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for VfsPath {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_case_separators_and_leading_backslash() {
        let path = VfsPath::new("/A3/Data_F\\\\Config.BIN\\");
        assert_eq!(path.as_str(), r"a3\data_f\config.bin");
    }

    #[test]
    fn paths_differing_only_in_case_and_separators_are_equal() {
        assert_eq!(VfsPath::new(r"A3\UI_F\Data"), VfsPath::new("a3/ui_f/data/"));
    }

    #[test]
    fn empty_and_separator_only_paths_are_the_root() {
        assert!(VfsPath::new("").is_root());
        assert!(VfsPath::new(r"\\/").is_root());
        assert_eq!(VfsPath::new(r"\"), VfsPath::root());
    }

    #[test]
    fn non_ascii_text_is_kept() {
        assert_eq!(VfsPath::new("Ünïcode\\Ä").as_str(), "Ünïcode\\Ä");
    }

    #[test]
    fn join_appends_a_normalised_child() {
        let dir = VfsPath::new(r"a3\data_f");
        assert_eq!(
            dir.join("/Images/X.paa").as_str(),
            r"a3\data_f\images\x.paa"
        );
        assert_eq!(VfsPath::root().join("A").as_str(), "a");
        assert_eq!(dir.join("").as_str(), r"a3\data_f");
    }

    #[test]
    fn parent_and_file_name_split_the_last_component() {
        let path = VfsPath::new(r"a3\data_f\config.bin");
        assert_eq!(path.parent(), Some(VfsPath::new(r"a3\data_f")));
        assert_eq!(path.file_name(), Some("config.bin"));
        assert_eq!(VfsPath::new("a3").parent(), Some(VfsPath::root()));
        assert_eq!(VfsPath::root().parent(), None);
        assert_eq!(VfsPath::root().file_name(), None);
    }

    #[test]
    fn extension_is_text_after_the_last_dot_of_the_file_name() {
        assert_eq!(VfsPath::new(r"a\b.c\x.p3d").extension(), Some("p3d"));
        assert_eq!(VfsPath::new(r"a\b.c\x").extension(), None);
        assert_eq!(VfsPath::new(r"a\.hidden").extension(), None);
    }

    #[test]
    fn starts_with_matches_whole_components_only() {
        let path = VfsPath::new(r"a3\data_f\config.bin");
        assert!(path.starts_with(&VfsPath::new(r"a3\data_f")));
        assert!(path.starts_with(&VfsPath::new("a3")));
        assert!(path.starts_with(&VfsPath::root()));
        assert!(path.starts_with(&path));
        assert!(!path.starts_with(&VfsPath::new(r"a3\data")));
    }

    #[test]
    fn components_iterates_the_parts() {
        let path = VfsPath::new(r"a3\data_f\config.bin");
        let parts: Vec<_> = path.components().collect();
        assert_eq!(parts, ["a3", "data_f", "config.bin"]);
        assert_eq!(VfsPath::root().components().count(), 0);
    }
}
