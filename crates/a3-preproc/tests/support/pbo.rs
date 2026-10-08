//! A minimal, test-only PBO reader: header entries plus raw reads of uncompressed entries.
//! The real reader lives in `a3-pbo`; this exists so the preprocessor can be tested on game data
//! without depending on it.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Packing method of a compressed entry (`Cprs`).
const MIME_COMPRESSED: u32 = 0x4370_7273;
/// Packing method of the header-properties entry (`Vers`).
const MIME_VERSION: u32 = 0x5665_7273;

#[derive(Debug, Clone)]
pub struct Entry {
    /// Index into [`Vfs::pbos`].
    pub pbo: usize,
    pub offset: u64,
    pub size: u32,
    pub compressed: bool,
}

/// Every PBO under a directory, mounted by prefix. Keys are lowercase virtual paths with a
/// leading backslash.
#[derive(Debug, Default)]
pub struct Vfs {
    pub pbos: Vec<PathBuf>,
    pub files: HashMap<String, Entry>,
    /// PBOs whose header could not be read.
    pub broken: Vec<PathBuf>,
}

fn read_cstring(r: &mut impl Read) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut b = [0u8; 1];
    loop {
        r.read_exact(&mut b)?;
        if b[0] == 0 {
            break;
        }
        bytes.push(b[0]);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_u32(r: &mut impl Read) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

struct RawEntry {
    name: String,
    mime: u32,
    original_size: u32,
    data_size: u32,
}

fn read_header(path: &Path) -> std::io::Result<(Option<String>, Vec<RawEntry>, u64)> {
    let mut r = BufReader::new(File::open(path)?);
    let mut prefix = None;
    let mut entries = Vec::new();
    loop {
        let name = read_cstring(&mut r)?;
        let mime = read_u32(&mut r)?;
        let original_size = read_u32(&mut r)?;
        let _reserved = read_u32(&mut r)?;
        let _timestamp = read_u32(&mut r)?;
        let data_size = read_u32(&mut r)?;
        if name.is_empty() && mime == MIME_VERSION {
            loop {
                let key = read_cstring(&mut r)?;
                if key.is_empty() {
                    break;
                }
                let value = read_cstring(&mut r)?;
                if key.eq_ignore_ascii_case("prefix") {
                    prefix = Some(value);
                }
            }
            continue;
        }
        if name.is_empty() {
            break;
        }
        entries.push(RawEntry {
            name,
            mime,
            original_size,
            data_size,
        });
    }
    let data_start = r.stream_position()?;
    Ok((prefix, entries, data_start))
}

fn collect_pbos(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_pbos(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pbo"))
        {
            out.push(path);
        }
    }
}

impl Vfs {
    /// Mounts every `*.pbo` below `root`. Later PBOs (in sorted path order) override earlier ones.
    pub fn mount(root: &Path) -> Self {
        let mut pbo_paths = Vec::new();
        collect_pbos(root, &mut pbo_paths);
        let mut vfs = Vfs::default();
        for path in pbo_paths {
            let Ok((prefix, entries, data_start)) = read_header(&path) else {
                vfs.broken.push(path);
                continue;
            };
            let index = vfs.pbos.len();
            let prefix = prefix.unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            let prefix = prefix.trim_matches('\\').replace('/', "\\");
            let mut offset = data_start;
            for entry in entries {
                let virtual_path = if prefix.is_empty() {
                    format!("\\{}", entry.name)
                } else {
                    format!("\\{prefix}\\{}", entry.name)
                };
                vfs.files.insert(
                    virtual_path.to_ascii_lowercase(),
                    Entry {
                        pbo: index,
                        offset,
                        size: entry.data_size,
                        compressed: entry.mime == MIME_COMPRESSED
                            || (entry.original_size != 0 && entry.original_size != entry.data_size),
                    },
                );
                offset += u64::from(entry.data_size);
            }
            vfs.pbos.push(path);
        }
        vfs
    }

    /// Reads an uncompressed entry. `None` for compressed entries or I/O failures.
    pub fn read(&self, entry: &Entry) -> Option<Vec<u8>> {
        if entry.compressed {
            return None;
        }
        let mut file = File::open(&self.pbos[entry.pbo]).ok()?;
        file.seek(SeekFrom::Start(entry.offset)).ok()?;
        let mut data = vec![0; entry.size as usize];
        file.read_exact(&mut data).ok()?;
        Some(data)
    }
}
