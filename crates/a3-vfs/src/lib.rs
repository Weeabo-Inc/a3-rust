//! The engine's virtual file system (VFS).
//!
//! A [`Vfs`] is one case-insensitive, backslash-separated file tree built by mounting PBOs at
//! their Prefix and loose OS folders at a chosen virtual path. Paths are [`VfsPath`]s; lookups
//! accept any spelling and normalise it.
//!
//! **Priority.** A later mount overrides an earlier one, file by file. [`Vfs::mount_game`]
//! mounts in the engine's load order: `Dta` (core), the base game `Addons`, the official DLC
//! folders in release order, then the mods given by the caller. See `docs/re/vfs.md` for what
//! is known and uncertain about the original engine's rules.
//!
//! `Vfs` is `Send + Sync`; clones share the same tree.

mod game;
mod glob;

use std::collections::BTreeMap;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

pub use a3_core::VfsPath;
use a3_pbo::Pbo;
use bytes::Bytes;

pub use game::{OFFICIAL_MOD_DIRS, find_addons_dir, optional_mod_dirs};

/// Errors from VFS lookups and mounts.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No file is mounted at the path.
    #[error("file not found in VFS: {0}")]
    NotFound(VfsPath),

    /// A PBO could not be opened or parsed.
    #[error("cannot mount {path}: {source}")]
    Pbo {
        /// The OS path of the archive.
        path: PathBuf,
        /// The underlying PBO error.
        source: a3_pbo::Error,
    },

    /// A mounted PBO entry could not be read (unsupported packing method, encrypted).
    #[error("cannot read {path}: {source}")]
    Entry {
        /// The VFS path of the file.
        path: VfsPath,
        /// The underlying PBO error.
        source: a3_pbo::Error,
    },

    /// An OS file or folder could not be read.
    #[error("I/O error on {path}: {source}")]
    Io {
        /// The OS path involved.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// One child of a VFS directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// The child's name (one path component, lower-case).
    pub name: String,
    /// `true` for a directory, `false` for a file.
    pub is_dir: bool,
}

/// Facts about one mounted file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// Unpacked size in bytes.
    pub size: u64,
    /// The OS file that provides it: the PBO archive or the loose file. `None` for a PBO
    /// mounted from memory.
    pub source: Option<PathBuf>,
}

/// The outcome of mounting a set of archives.
#[derive(Debug, Clone, Default)]
pub struct MountReport {
    /// PBOs mounted.
    pub pbos: usize,
    /// Files added to the tree (counting overrides).
    pub files: usize,
    /// Files that replaced a file mounted earlier at the same path.
    pub overridden: usize,
    /// Encrypted archives (EBOs) skipped, with their declared prefix when readable.
    pub encrypted: Vec<(PathBuf, Option<VfsPath>)>,
    /// Archives that failed to open, with the reason.
    pub failed: Vec<(PathBuf, String)>,
    /// Wall-clock time spent.
    pub elapsed: Duration,
}

impl MountReport {
    fn merge(&mut self, other: MountReport) {
        self.pbos += other.pbos;
        self.files += other.files;
        self.overridden += other.overridden;
        self.encrypted.extend(other.encrypted);
        self.failed.extend(other.failed);
        self.elapsed += other.elapsed;
    }
}

/// One PBO mounted in a [`Vfs`], as listed by [`Vfs::archives`].
#[derive(Clone)]
pub struct MountedArchive {
    /// The virtual path the archive's entries are mounted under.
    pub prefix: VfsPath,
    /// The OS file it was opened from; `None` for a PBO mounted from memory.
    pub source: Option<PathBuf>,
    /// The parsed archive (shared with the VFS).
    pub pbo: Arc<Pbo>,
}

impl std::fmt::Debug for MountedArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MountedArchive")
            .field("prefix", &self.prefix)
            .field("source", &self.source)
            .field("entries", &self.pbo.entries().len())
            .finish()
    }
}

/// The virtual file system. Cheap to clone; clones share one tree.
#[derive(Clone, Default)]
pub struct Vfs {
    inner: Arc<RwLock<Tree>>,
}

#[derive(Default)]
struct Tree {
    mounts: Vec<Mount>,
    files: BTreeMap<VfsPath, FileRef>,
}

enum Mount {
    Pbo {
        pbo: Arc<Pbo>,
        path: Option<PathBuf>,
        prefix: VfsPath,
    },
    Dir {
        root: PathBuf,
    },
}

#[derive(Clone)]
enum FileRef {
    Entry {
        mount: u32,
        entry: u32,
    },
    Loose {
        mount: u32,
        rel: Box<Path>,
        size: u64,
    },
}

/// What a lookup resolved to, detached from the lock.
enum Resolved {
    Entry(Arc<Pbo>, u32, Option<PathBuf>),
    Loose(PathBuf, u64),
}

impl Vfs {
    /// An empty VFS.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mounts a parsed PBO at `prefix`, or at its own `prefix` property when `None` (the root
    /// if it has none). Returns the prefix used.
    pub fn mount_pbo(&self, pbo: Pbo, prefix: Option<VfsPath>) -> VfsPath {
        self.mount_parsed(pbo, prefix, None).0
    }

    /// Opens and mounts the PBO at `path` at its `prefix` property, or at its lower-cased file
    /// stem when it declares none. Returns the prefix used.
    pub fn mount_pbo_file(&self, path: &Path) -> Result<VfsPath> {
        let pbo = open_pbo(path)?;
        let prefix = pbo.prefix().unwrap_or_else(|| stem_prefix(path));
        Ok(self
            .mount_parsed(pbo, Some(prefix), Some(path.to_owned()))
            .0)
    }

    /// Mounts every file below the OS folder `dir` at `prefix`. Returns the number of files.
    ///
    /// The folder is scanned once, now; files added to it later are not seen.
    pub fn mount_dir(&self, dir: &Path, prefix: VfsPath) -> Result<usize> {
        let mut found = Vec::new();
        scan_dir(dir, Path::new(""), &mut found)?;
        let count = found.len();
        let mut tree = self.write();
        let mount = tree.mounts.len() as u32;
        tree.mounts.push(Mount::Dir {
            root: dir.to_owned(),
        });
        for (rel, size) in found {
            let path = prefix.join(&rel.to_string_lossy());
            let rel = rel.into_boxed_path();
            tree.files.insert(path, FileRef::Loose { mount, rel, size });
        }
        Ok(count)
    }

    /// Mounts every PBO in the `addons` folder of the mod folder `mod_dir` (the folder name is
    /// matched case-insensitively), in file-name order. Encrypted EBOs are skipped and archives
    /// that fail to parse are reported; neither stops the mount.
    pub fn mount_mod(&self, mod_dir: &Path) -> MountReport {
        match find_addons_dir(mod_dir) {
            Some(addons) => self.mount_archives(&addons),
            None => MountReport::default(),
        }
    }

    /// Mounts every PBO and EBO directly inside the OS folder `dir`, in file-name order.
    pub fn mount_archives(&self, dir: &Path) -> MountReport {
        let start = Instant::now();
        let mut report = MountReport::default();
        for path in game::archives_in(dir) {
            if game::is_ebo(&path) {
                let prefix = a3_pbo::read_properties(&path)
                    .ok()
                    .and_then(|props| props.prefix());
                report.encrypted.push((path, prefix));
                continue;
            }
            match open_pbo(&path) {
                Ok(pbo) => {
                    let prefix = pbo.prefix().unwrap_or_else(|| stem_prefix(&path));
                    let (_, files, overridden) = self.mount_parsed(pbo, Some(prefix), Some(path));
                    report.pbos += 1;
                    report.files += files;
                    report.overridden += overridden;
                }
                Err(e) => report.failed.push((path, e.to_string())),
            }
        }
        report.elapsed = start.elapsed();
        report
    }

    /// Mounts a game install in load order: `Dta`, the base game `Addons`, each official DLC
    /// folder of [`OFFICIAL_MOD_DIRS`] that exists, then `mods` in the given order.
    pub fn mount_game(&self, root: &Path, mods: &[PathBuf]) -> MountReport {
        let mut report = MountReport::default();
        if let Some(dta) = game::find_child_dir(root, "dta") {
            report.merge(self.mount_archives(&dta));
        }
        report.merge(self.mount_mod(root));
        for name in OFFICIAL_MOD_DIRS {
            if let Some(dir) = game::find_child_dir(root, name) {
                report.merge(self.mount_mod(&dir));
            }
        }
        for dir in mods {
            report.merge(self.mount_mod(dir));
        }
        report
    }

    /// Reads the whole file at `path`. Files inside PBOs are returned without copying.
    pub fn open(&self, path: &str) -> Result<Bytes> {
        let path = VfsPath::new(path);
        match self.resolve(&path) {
            None => Err(Error::NotFound(path)),
            Some(Resolved::Entry(pbo, entry, _)) => pbo
                .read_entry(&pbo.entries()[entry as usize])
                .map_err(|source| Error::Entry { path, source }),
            Some(Resolved::Loose(os_path, _)) => {
                std::fs::read(&os_path)
                    .map(Bytes::from)
                    .map_err(|source| Error::Io {
                        path: os_path,
                        source,
                    })
            }
        }
    }

    /// Returns `true` when a file is mounted at `path`.
    pub fn exists(&self, path: &str) -> bool {
        self.read().files.contains_key(VfsPath::new(path).as_str())
    }

    /// Returns `true` when some file is mounted below `path`.
    pub fn is_dir(&self, path: &str) -> bool {
        let dir = VfsPath::new(path);
        let tree = self.read();
        tree.below(&dir).next().is_some()
    }

    /// Size and origin of the file at `path`.
    pub fn stat(&self, path: &str) -> Option<FileInfo> {
        Some(match self.resolve(&VfsPath::new(path))? {
            Resolved::Entry(pbo, entry, source) => FileInfo {
                size: u64::from(pbo.entries()[entry as usize].size()),
                source,
            },
            Resolved::Loose(os_path, size) => FileInfo {
                size,
                source: Some(os_path),
            },
        })
    }

    /// The immediate children of the directory `dir`, sorted by name. Empty when `dir` holds
    /// nothing (or is a file).
    pub fn list_dir(&self, dir: &str) -> Vec<DirEntry> {
        let dir = VfsPath::new(dir);
        let skip = if dir.is_root() {
            0
        } else {
            dir.as_str().len() + 1
        };
        let tree = self.read();
        let mut out: Vec<DirEntry> = Vec::new();
        for path in tree.below(&dir) {
            let rest = &path.as_str()[skip..];
            let (name, is_dir) = match rest.split_once('\\') {
                Some((name, _)) => (name, true),
                None => (rest, false),
            };
            match out.last_mut() {
                Some(last) if last.name == name => last.is_dir |= is_dir,
                _ => out.push(DirEntry {
                    name: name.to_owned(),
                    is_dir,
                }),
            }
        }
        // Children of one directory are contiguous, but `\` sorts after characters such as
        // `.` and `0`, so `b0.txt` precedes the files of `b\`. Re-sort by name.
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Every file below `dir` (recursively), sorted.
    pub fn walk(&self, dir: &str) -> Vec<VfsPath> {
        let dir = VfsPath::new(dir);
        self.read().below(&dir).cloned().collect()
    }

    /// Every file whose path matches `pattern`, sorted. In the pattern, `?` matches one
    /// character and `*` any run of characters within a path component; a `**` component
    /// matches any number of components. Matching ignores case.
    pub fn glob(&self, pattern: &str) -> Vec<VfsPath> {
        let pattern = glob::Pattern::new(pattern);
        let base = pattern.literal_base();
        self.read()
            .below(&base)
            .filter(|path| pattern.matches(path))
            .cloned()
            .collect()
    }

    /// Every mounted PBO in mount order (lowest priority first). Loose folders are not listed.
    ///
    /// Engine subsystems that process whole archives use this, e.g. loading each addon's
    /// `config.bin` in discovery order.
    pub fn archives(&self) -> Vec<MountedArchive> {
        self.read()
            .mounts
            .iter()
            .filter_map(|mount| match mount {
                Mount::Pbo { pbo, path, prefix } => Some(MountedArchive {
                    prefix: prefix.clone(),
                    source: path.clone(),
                    pbo: Arc::clone(pbo),
                }),
                Mount::Dir { .. } => None,
            })
            .collect()
    }

    /// Number of files in the tree.
    pub fn len(&self) -> usize {
        self.read().files.len()
    }

    /// Returns `true` when nothing is mounted.
    pub fn is_empty(&self) -> bool {
        self.read().files.is_empty()
    }

    fn mount_parsed(
        &self,
        pbo: Pbo,
        prefix: Option<VfsPath>,
        path: Option<PathBuf>,
    ) -> (VfsPath, usize, usize) {
        let prefix = prefix.or_else(|| pbo.prefix()).unwrap_or_default();
        let keys: Vec<VfsPath> = pbo
            .entries()
            .iter()
            .map(|e| prefix.join(e.name()))
            .collect();
        let mut tree = self.write();
        let mount = tree.mounts.len() as u32;
        tree.mounts.push(Mount::Pbo {
            pbo: Arc::new(pbo),
            path,
            prefix: prefix.clone(),
        });
        let mut overridden = 0;
        let files = keys.len();
        // Within one PBO the first entry of a duplicated name wins, so insert in reverse.
        for (entry, key) in keys.into_iter().enumerate().rev() {
            let file = FileRef::Entry {
                mount,
                entry: entry as u32,
            };
            if let Some(old) = tree.files.insert(key, file) {
                if old.mount() != mount {
                    overridden += 1;
                }
            }
        }
        (prefix, files, overridden)
    }

    fn resolve(&self, path: &VfsPath) -> Option<Resolved> {
        let tree = self.read();
        let file = tree.files.get(path.as_str())?;
        Some(match (file, &tree.mounts[file.mount() as usize]) {
            (FileRef::Entry { entry, .. }, Mount::Pbo { pbo, path, .. }) => {
                Resolved::Entry(Arc::clone(pbo), *entry, path.clone())
            }
            (FileRef::Loose { rel, size, .. }, Mount::Dir { root }) => {
                Resolved::Loose(root.join(rel), *size)
            }
            _ => unreachable!("file reference does not match its mount kind"),
        })
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Tree> {
        self.inner.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Tree> {
        self.inner.write().unwrap_or_else(PoisonError::into_inner)
    }
}

impl std::fmt::Debug for Vfs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tree = self.read();
        f.debug_struct("Vfs")
            .field("mounts", &tree.mounts.len())
            .field("files", &tree.files.len())
            .finish()
    }
}

impl Tree {
    /// Files at or below `dir`, in order. For the root, every file.
    fn below<'a>(&'a self, dir: &'a VfsPath) -> impl Iterator<Item = &'a VfsPath> + 'a {
        let start = if dir.is_root() {
            String::new()
        } else {
            format!("{dir}\\")
        };
        self.files
            .range::<str, _>((Bound::Included(start.as_str()), Bound::Unbounded))
            .map(|(path, _)| path)
            .take_while(move |path| path.as_str().starts_with(&start))
    }
}

impl FileRef {
    fn mount(&self) -> u32 {
        match self {
            Self::Entry { mount, .. } | Self::Loose { mount, .. } => *mount,
        }
    }
}

fn open_pbo(path: &Path) -> Result<Pbo> {
    Pbo::open(path).map_err(|source| Error::Pbo {
        path: path.to_owned(),
        source,
    })
}

fn stem_prefix(path: &Path) -> VfsPath {
    VfsPath::new(&path.file_stem().unwrap_or_default().to_string_lossy())
}

fn scan_dir(root: &Path, rel: &Path, out: &mut Vec<(PathBuf, u64)>) -> Result<()> {
    let dir = root.join(rel);
    let io = |source| Error::Io {
        path: dir.clone(),
        source,
    };
    for entry in std::fs::read_dir(&dir).map_err(io)? {
        let entry = entry.map_err(io)?;
        let child = rel.join(entry.file_name());
        let meta = entry.metadata().map_err(io)?;
        if meta.is_dir() {
            scan_dir(root, &child, out)?;
        } else {
            out.push((child, meta.len()));
        }
    }
    Ok(())
}
