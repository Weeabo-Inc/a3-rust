//! Locating `#include`d files.
//!
//! The preprocessor never touches a file system itself; it asks an [`IncludeResolver`]. The game
//! plugs in the VFS; tests use [`MemoryResolver`] or [`FsResolver`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::IncludeError;

/// A file found by an [`IncludeResolver`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInclude {
    /// The canonical virtual path of the file (backslash-separated, starting with `\`). Used for
    /// `__FILE__`, nested relative includes, the source map and `#line` directives.
    pub path: String,
    /// The file's text.
    pub source: String,
}

/// Finds the files named by `#include` and `__has_include`.
pub trait IncludeResolver {
    /// Resolves `include` (as written in the directive, without quotes) seen in `current_file`.
    ///
    /// A path starting with `\` is absolute in the VFS; any other path is relative to the
    /// directory of `current_file`, and may contain `..`. Use [`join_virtual_path`] for the
    /// standard rules. `current_file` is empty when the preprocessor loads its root file.
    fn resolve(&self, current_file: &str, include: &str) -> Result<ResolvedInclude, IncludeError>;

    /// Whether `include` exists, for `__has_include`. The default resolves and discards the file.
    fn exists(&self, current_file: &str, include: &str) -> bool {
        self.resolve(current_file, include).is_ok()
    }
}

/// Joins an include path to the file containing the directive, the way the engine does.
///
/// Forward slashes become backslashes, `.` and `..` segments are collapsed (`..` above the root
/// is dropped), and the result always starts with a single `\`. Case is preserved; virtual paths
/// compare case-insensitively.
///
/// ```
/// use a3_preproc::join_virtual_path;
/// assert_eq!(join_virtual_path("\\a3\\ui_f\\config.cpp", "hpp\\defines.hpp"),
///            "\\a3\\ui_f\\hpp\\defines.hpp");
/// assert_eq!(join_virtual_path("\\a3\\ui_f\\config.cpp", "..\\data_f\\x.h"), "\\a3\\data_f\\x.h");
/// assert_eq!(join_virtual_path("\\a3\\ui_f\\config.cpp", "\\x\\y.h"), "\\x\\y.h");
/// ```
pub fn join_virtual_path(current_file: &str, include: &str) -> String {
    if include.starts_with(['\\', '/']) {
        return normalize(include);
    }
    let dir = current_file
        .rfind(['\\', '/'])
        .map_or("", |end| &current_file[..end]);
    normalize(&format!("{dir}\\{include}"))
}

/// Collapses `.`/`..`/empty segments and adds the leading backslash.
fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for segment in path.split(['\\', '/']) {
        match segment {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    let mut result = String::with_capacity(path.len() + 1);
    for segment in out {
        result.push('\\');
        result.push_str(segment);
    }
    if result.is_empty() {
        result.push('\\');
    }
    result
}

/// An in-memory set of files keyed by virtual path (case-insensitive).
#[derive(Debug, Clone, Default)]
pub struct MemoryResolver {
    files: HashMap<String, ResolvedInclude>,
}

impl MemoryResolver {
    /// An empty resolver.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or replaces) a file. `path` is normalized with [`join_virtual_path`] rules.
    pub fn insert(&mut self, path: &str, source: impl Into<String>) -> &mut Self {
        let path = normalize(path);
        self.files.insert(
            path.to_ascii_lowercase(),
            ResolvedInclude {
                path,
                source: source.into(),
            },
        );
        self
    }

    /// Builder form of [`Self::insert`].
    pub fn with_file(mut self, path: &str, source: impl Into<String>) -> Self {
        self.insert(path, source);
        self
    }
}

impl IncludeResolver for MemoryResolver {
    fn resolve(&self, current_file: &str, include: &str) -> Result<ResolvedInclude, IncludeError> {
        let path = join_virtual_path(current_file, include);
        self.files
            .get(&path.to_ascii_lowercase())
            .cloned()
            .ok_or(IncludeError::NotFound(path))
    }
}

/// Resolves virtual paths against a directory on disk: `\a\b.hpp` maps to `<root>/a/b.hpp`.
///
/// Lookup is exact-case first, then case-insensitive per path segment, so it behaves like the VFS
/// on case-sensitive file systems too. File contents are decoded as UTF-8, replacing invalid
/// sequences.
#[derive(Debug, Clone)]
pub struct FsResolver {
    root: PathBuf,
}

impl FsResolver {
    /// A resolver rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn locate(&self, virtual_path: &str) -> Option<PathBuf> {
        let mut current = self.root.clone();
        for segment in virtual_path.split('\\').filter(|s| !s.is_empty()) {
            let exact = current.join(segment);
            if exact.exists() {
                current = exact;
                continue;
            }
            current = find_case_insensitive(&current, segment)?;
        }
        current.is_file().then_some(current)
    }
}

fn find_case_insensitive(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(name)
        })
        .map(|entry| entry.path())
}

impl IncludeResolver for FsResolver {
    fn resolve(&self, current_file: &str, include: &str) -> Result<ResolvedInclude, IncludeError> {
        let path = join_virtual_path(current_file, include);
        let os_path = self
            .locate(&path)
            .ok_or_else(|| IncludeError::NotFound(path.clone()))?;
        let bytes = std::fs::read(&os_path).map_err(|e| IncludeError::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
        Ok(ResolvedInclude {
            path,
            source: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }

    fn exists(&self, current_file: &str, include: &str) -> bool {
        self.locate(&join_virtual_path(current_file, include))
            .is_some()
    }
}
