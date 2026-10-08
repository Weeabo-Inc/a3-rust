//! Just enough PBO and PAA parsing to find compressed data in a real install. The proper readers
//! live in their own crates; these helpers only serve the codec tests.

#![allow(dead_code)]

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// A file inside a PBO: name, absolute data offset in the PBO, stored size.
pub struct Entry {
    pub name: String,
    pub offset: u64,
    pub size: u32,
}

impl Entry {
    pub fn has_extension(&self, ext: &str) -> bool {
        self.name
            .rsplit_once('.')
            .is_some_and(|(_, x)| x.eq_ignore_ascii_case(ext))
    }
}

/// The `Addons` PBOs of the install at `A3_ROOT`, or `None` (with a note) when it is unset.
pub fn game_pbos() -> Option<Vec<PathBuf>> {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return None;
    };
    Some(pbos(&Path::new(&root).join("Addons")))
}

fn read_cstr(r: &mut impl Read) -> std::io::Result<String> {
    let mut s = Vec::new();
    let mut b = [0u8; 1];
    loop {
        r.read_exact(&mut b)?;
        if b[0] == 0 {
            return Ok(String::from_utf8_lossy(&s).into_owned());
        }
        s.push(b[0]);
    }
}

fn read_u32(r: &mut impl Read) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

/// Minimal PBO header walk: enough to locate entries stored without compression.
pub fn pbo_entries(path: &Path) -> std::io::Result<Vec<Entry>> {
    let mut r = BufReader::new(File::open(path)?);
    let mut raw = Vec::new();
    let mut first = true;
    loop {
        let name = read_cstr(&mut r)?;
        let method = read_u32(&mut r)?;
        let _original = read_u32(&mut r)?;
        let _reserved = read_u32(&mut r)?;
        let _timestamp = read_u32(&mut r)?;
        let size = read_u32(&mut r)?;
        if first && name.is_empty() && method == 0x5665_7273 {
            // Header properties: key/value strings up to an empty key.
            while !read_cstr(&mut r)?.is_empty() {
                read_cstr(&mut r)?;
            }
            first = false;
            continue;
        }
        first = false;
        if name.is_empty() {
            break;
        }
        raw.push((name, size));
    }
    let mut offset = r.stream_position()?;
    Ok(raw
        .into_iter()
        .map(|(name, size)| {
            let e = Entry { name, offset, size };
            offset += u64::from(size);
            e
        })
        .collect())
}

pub fn read_entry(pbo: &Path, e: &Entry) -> Vec<u8> {
    read_entry_prefix(pbo, e, usize::MAX)
}

/// The first `max` bytes of an entry (all of it if shorter).
pub fn read_entry_prefix(pbo: &Path, e: &Entry, max: usize) -> Vec<u8> {
    let mut f = File::open(pbo).unwrap();
    f.seek(SeekFrom::Start(e.offset)).unwrap();
    let mut data = vec![0u8; (e.size as usize).min(max)];
    f.read_exact(&mut data).unwrap();
    data
}

fn pbos(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for item in std::fs::read_dir(&dir).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("pbo"))
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// The first mipmap of a PAA.
pub struct Mip<'a> {
    /// Pixel format tag (`0xFF01` DXT1, `0xFF05` DXT5, `0x4444`, `0x1555`, `0x8080`, ...).
    pub tag: u16,
    /// Raw width field; the top bit flags LZO compression of DXT data.
    pub width: u16,
    pub height: u16,
    pub data: &'a [u8],
}

/// The first mipmap of a PAA, or `None` if `paa` is too short to hold it.
pub fn first_mip(paa: &[u8]) -> Option<Mip<'_>> {
    let (tag, width, height, start, len) = first_mip_header(paa)?;
    Some(Mip {
        tag,
        width,
        height,
        data: paa.get(start..start + len)?,
    })
}

/// `(tag, width, height, data offset, data length)` of the first mipmap; needs only the header.
pub fn first_mip_header(paa: &[u8]) -> Option<(u16, u16, u16, usize, usize)> {
    let u16_at = |p: usize| Some(u16::from_le_bytes(paa.get(p..p + 2)?.try_into().ok()?));
    let tag = u16_at(0)?;
    let mut pos = 2;
    while paa.get(pos..pos + 4)? == b"GGAT" {
        let len = u32::from_le_bytes(paa.get(pos + 8..pos + 12)?.try_into().ok()?) as usize;
        pos += 12 + len;
    }
    let palette = usize::from(u16_at(pos)?);
    pos += 2 + palette * 3;
    let width = u16_at(pos)?;
    let height = u16_at(pos + 2)?;
    let l = paa.get(pos + 4..pos + 7)?;
    let len = u32::from_le_bytes([l[0], l[1], l[2], 0]) as usize;
    Some((tag, width, height, pos + 7, len))
}
