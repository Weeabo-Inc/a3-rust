use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::OnceLock;

use a3_core::VfsPath;
use bytes::Bytes;
use sha1::{Digest, Sha1};

use crate::entry::{Entry, METHOD_PROPERTIES, PackingMethod};
use crate::{Error, Properties, Result};

/// Size of the trailer after the data block: one zero byte and a SHA-1 digest.
const TRAILER_LEN: usize = 21;
/// Size of the five little-endian `u32` fields that follow every entry name.
const RECORD_LEN: usize = 20;

/// A parsed PBO archive with random access to its entries.
///
/// The archive bytes are held as [`Bytes`]: a memory map for [`Pbo::open`], so only the pages
/// that are read are loaded, or an owned buffer for [`Pbo::from_bytes`]. Reading a stored entry
/// is zero-copy. `Pbo` is `Send + Sync`.
#[derive(Debug)]
pub struct Pbo {
    data: Bytes,
    properties: Properties,
    entries: Vec<Entry>,
    header_len: u64,
    data_end: u64,
    stored_hash: Option<[u8; 20]>,
    index: OnceLock<HashMap<VfsPath, usize>>,
}

impl Pbo {
    /// Memory-maps and parses the PBO at `path`.
    ///
    /// Files with the `.ebo` extension are encrypted and fail with [`Error::Encrypted`];
    /// use [`read_properties`] to read their properties.
    ///
    /// The file must not be modified or truncated while the returned `Pbo` (or any [`Bytes`]
    /// read from it) is alive.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if is_ebo(path) {
            return Err(Error::Encrypted);
        }
        Self::from_bytes(map_file(path)?)
    }

    /// Parses a PBO held in memory.
    pub fn from_bytes(data: impl Into<Bytes>) -> Result<Self> {
        let data = data.into();
        let header = parse_header(&data)?;
        let mut offset = header.len;
        let mut entries = header.entries;
        for entry in &mut entries {
            entry.data_offset = offset;
            offset += u64::from(entry.data_size);
        }
        let file_len = data.len() as u64;
        if offset > file_len {
            return Err(Error::Malformed(format!(
                "entry data ends at byte {offset}, past the end of the {file_len}-byte file"
            )));
        }
        let trailer = &data[offset as usize..];
        let stored_hash = (trailer.len() >= TRAILER_LEN && trailer[0] == 0)
            .then(|| trailer[1..TRAILER_LEN].try_into().expect("20-byte slice"));
        Ok(Self {
            data,
            properties: header.properties,
            entries,
            header_len: header.len,
            data_end: offset,
            stored_hash,
            index: OnceLock::new(),
        })
    }

    /// The header properties (`prefix`, `product`, `version`, ...) in file order.
    pub fn properties(&self) -> &Properties {
        &self.properties
    }

    /// The `prefix` property, normalised. `None` when the PBO declares no prefix.
    pub fn prefix(&self) -> Option<VfsPath> {
        self.properties.prefix()
    }

    /// All entries in header order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Looks up an entry by path, ignoring case and separator style. If several entries share
    /// a path, the first one wins.
    pub fn entry(&self, path: &str) -> Option<&Entry> {
        let index = self.index.get_or_init(|| {
            let mut index = HashMap::with_capacity(self.entries.len());
            for (i, entry) in self.entries.iter().enumerate() {
                index.entry(VfsPath::new(&entry.name)).or_insert(i);
            }
            index
        });
        index
            .get(VfsPath::new(path).as_str())
            .map(|&i| &self.entries[i])
    }

    /// The bytes of `entry` exactly as stored in the archive (still packed if compressed).
    pub fn raw(&self, entry: &Entry) -> Bytes {
        let start = entry.data_offset as usize;
        self.data.slice(start..start + entry.data_size as usize)
    }

    /// The unpacked content of `entry`.
    pub fn read_entry(&self, entry: &Entry) -> Result<Bytes> {
        match entry.method {
            PackingMethod::Stored => Ok(self.raw(entry)),
            PackingMethod::Encrypted => Err(Error::Encrypted),
            method => Err(Error::Unsupported {
                name: entry.name.clone(),
                method,
            }),
        }
    }

    /// The unpacked content of the entry at `path` (case-insensitive).
    pub fn read(&self, path: &str) -> Result<Bytes> {
        let entry = self
            .entry(path)
            .ok_or_else(|| Error::NotFound(path.to_owned()))?;
        self.read_entry(entry)
    }

    /// Length in bytes of the header (properties and entry records).
    pub fn header_len(&self) -> u64 {
        self.header_len
    }

    /// The SHA-1 digest stored in the trailer, if the archive has one.
    pub fn stored_hash(&self) -> Option<[u8; 20]> {
        self.stored_hash
    }

    /// Computes the SHA-1 digest of the header and data, the bytes that the trailer covers.
    /// Reads the whole archive.
    pub fn compute_hash(&self) -> [u8; 20] {
        Sha1::digest(&self.data[..self.data_end as usize]).into()
    }

    /// Checks the trailer digest against the archive content. Reads the whole archive.
    pub fn verify(&self) -> Result<()> {
        let stored = self.stored_hash.ok_or(Error::MissingHash)?;
        let computed = self.compute_hash();
        if stored == computed {
            Ok(())
        } else {
            Err(Error::HashMismatch {
                stored: hex(&stored),
                computed: hex(&computed),
            })
        }
    }
}

/// Reads only the properties of the PBO or EBO at `path`, without mapping the file.
///
/// Works on encrypted EBOs, whose properties are stored in clear text.
pub fn read_properties(path: impl AsRef<Path>) -> Result<Properties> {
    let mut reader = BufReader::with_capacity(4096, File::open(path)?);
    let mut offset = 0u64;
    let mut read_cstr = |reader: &mut BufReader<File>| -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        reader.read_until(0, &mut buf)?;
        offset += buf.len() as u64;
        if buf.pop() != Some(0) {
            return Err(Error::Truncated { offset });
        }
        Ok(buf)
    };
    let name = read_cstr(&mut reader)?;
    let mut record = [0u8; RECORD_LEN];
    std::io::Read::read_exact(&mut reader, &mut record)?;
    let mut properties = Properties::default();
    if !name.is_empty()
        || u32::from_le_bytes(record[..4].try_into().expect("4 bytes")) != METHOD_PROPERTIES
    {
        return Ok(properties);
    }
    loop {
        let key = read_cstr(&mut reader)?;
        if key.is_empty() {
            return Ok(properties);
        }
        let value = read_cstr(&mut reader)?;
        properties.push(decode(&key), decode(&value));
    }
}

fn is_ebo(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ebo"))
}

#[allow(unsafe_code)]
fn map_file(path: &Path) -> Result<Bytes> {
    let file = File::open(path)?;
    if file.metadata()?.len() == 0 {
        return Err(Error::Truncated { offset: 0 });
    }
    // SAFETY: the map is read-only and game archives are not modified while the engine runs.
    // `Pbo::open` documents that the file must not change while mapped; that is the only
    // way the mapped bytes could change underneath a `&[u8]`.
    let map = unsafe { memmap2::Mmap::map(&file)? };
    Ok(Bytes::from_owner(map))
}

struct Header {
    properties: Properties,
    entries: Vec<Entry>,
    len: u64,
}

fn parse_header(data: &[u8]) -> Result<Header> {
    let mut cursor = Cursor { data, pos: 0 };
    let mut properties = Properties::default();
    let mut entries = Vec::new();
    let mut first = true;
    loop {
        let name = cursor.cstr()?;
        let method = cursor.u32()?;
        let original_size = cursor.u32()?;
        let reserved = cursor.u32()?;
        let timestamp = cursor.u32()?;
        let data_size = cursor.u32()?;
        if name.is_empty() {
            if first && method == METHOD_PROPERTIES {
                first = false;
                loop {
                    let key = cursor.cstr()?;
                    if key.is_empty() {
                        break;
                    }
                    let value = cursor.cstr()?;
                    properties.push(decode(key), decode(value));
                }
                continue;
            }
            break;
        }
        first = false;
        entries.push(Entry {
            name: decode(name),
            method: PackingMethod::from_raw(method),
            original_size,
            reserved,
            timestamp,
            data_size,
            data_offset: 0,
        });
    }
    Ok(Header {
        properties,
        entries,
        len: cursor.pos as u64,
    })
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn cstr(&mut self) -> Result<&'a [u8]> {
        let rest = &self.data[self.pos..];
        let len = rest.iter().position(|&b| b == 0).ok_or(Error::Truncated {
            offset: self.data.len() as u64,
        })?;
        self.pos += len + 1;
        Ok(&rest[..len])
    }

    fn u32(&mut self) -> Result<u32> {
        let bytes = self
            .data
            .get(self.pos..self.pos + 4)
            .ok_or(Error::Truncated {
                offset: self.pos as u64,
            })?;
        self.pos += 4;
        Ok(u32::from_le_bytes(bytes.try_into().expect("4 bytes")))
    }
}

/// Decodes header text: UTF-8 when valid, otherwise each byte as one Latin-1 character.
fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
