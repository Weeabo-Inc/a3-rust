//! Rapified config (`\0raP`) reader and writer.
//!
//! Layout (all integers little-endian), as documented in `docs/re/config.md`:
//!
//! ```text
//! header   "\0raP" u32(0) u32(8) u32(enum_table_offset)
//! body     asciiz base, cint entry_count, entry*, u32 end_of_subtree, child bodies...
//! entry    u8 type, then
//!            0 class     asciiz name, u32 body_offset
//!            1 value     u8 subtype, asciiz name, scalar(subtype)
//!            2 array     asciiz name, array
//!            3 external  asciiz name
//!            4 delete    asciiz name
//!            5 append    u32 flags (1 = `+=`), asciiz name, array
//! array    cint count, (u8 subtype, scalar-or-array(subtype))*
//! scalar   0 asciiz string, 1 f32, 2 i32, 3 array (in arrays only), 4 asciiz expression, 6 i64
//! enums    u32 count, (asciiz name, u32 value)*
//! ```
//!
//! `cint` is an unsigned LEB128-style varint (7 bits per byte, low group first).

use crate::{Config, ConfigClass, Entry, EntryKind, EnumEntry, Value};

const MAGIC: &[u8; 4] = b"\0raP";
/// Guards against cyclic class offsets and absurdly deep arrays in corrupt files.
const MAX_DEPTH: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum RapError {
    #[error("not a rapified config (missing \\0raP signature)")]
    BadMagic,
    #[error("unexpected end of data at offset {0:#x}")]
    UnexpectedEof(usize),
    #[error("unknown entry type {kind} at offset {offset:#x}")]
    UnknownEntryType { kind: u8, offset: usize },
    #[error("unknown value subtype {subtype} at offset {offset:#x}")]
    UnknownValueType { subtype: u8, offset: usize },
    #[error("unknown array-append flags {flags:#x} at offset {offset:#x}")]
    UnknownAppendFlags { flags: u32, offset: usize },
    #[error("malformed compressed integer at offset {0:#x}")]
    BadCompressedInt(usize),
    #[error("class body at offset {0:#x} is referenced more than once")]
    SharedClassBody(usize),
    #[error("class nesting deeper than {MAX_DEPTH} levels at offset {0:#x} (cyclic offsets?)")]
    TooDeep(usize),
}

/// Whether `bytes` starts with the rapified-config signature.
pub fn is_rap(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// Parses a rapified config.
pub fn read_rap(bytes: &[u8]) -> Result<Config, RapError> {
    if !is_rap(bytes) {
        return Err(RapError::BadMagic);
    }
    let mut r = Reader {
        bytes,
        pos: 4,
        seen_bodies: std::collections::HashSet::new(),
    };
    let _zero = r.u32()?;
    let _eight = r.u32()?;
    let enum_offset = r.u32()? as usize;
    let root = r.class_body(0)?;

    let mut enums = Vec::new();
    if enum_offset != 0 {
        r.pos = enum_offset;
        let count = r.u32()?;
        for _ in 0..count {
            // Some shipped files (e.g. editor_f's config.bin) declare more enum constants than
            // they contain; the table then simply ends at the end of the file.
            if r.pos == bytes.len() {
                break;
            }
            let name = r.asciiz()?;
            let value = r.u32()? as i32;
            enums.push(EnumEntry { name, value });
        }
    }
    Ok(Config { root, enums })
}

/// Serialises a config to the rapified form, using the same layout as BI's tools.
pub fn write_rap(config: &Config) -> Vec<u8> {
    let mut w = Writer { out: Vec::new() };
    w.out.extend_from_slice(MAGIC);
    w.u32(0);
    w.u32(8);
    let enum_slot = w.placeholder();
    w.class_body(&config.root);
    let enum_offset = w.here();
    w.patch(enum_slot, enum_offset);
    w.u32(config.enums.len() as u32);
    for e in &config.enums {
        w.asciiz(&e.name);
        w.u32(e.value as u32);
    }
    w.out
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    /// Body offsets already read; each body belongs to exactly one class entry.
    seen_bodies: std::collections::HashSet<usize>,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], RapError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.bytes.len())
            .ok_or(RapError::UnexpectedEof(self.pos))?;
        let s = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, RapError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, RapError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f32(&mut self) -> Result<f32, RapError> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn i64(&mut self) -> Result<i64, RapError> {
        let b = self.take(8)?;
        Ok(i64::from_le_bytes(b.try_into().expect("8 bytes")))
    }

    fn cint(&mut self) -> Result<u32, RapError> {
        let start = self.pos;
        let mut value: u32 = 0;
        for shift in (0..35).step_by(7) {
            let b = self.u8()?;
            value |= u32::from(b & 0x7f)
                .checked_shl(shift)
                .ok_or(RapError::BadCompressedInt(start))?;
            if b & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(RapError::BadCompressedInt(start))
    }

    fn asciiz(&mut self) -> Result<String, RapError> {
        let start = self.pos;
        let len = self.bytes[start.min(self.bytes.len())..]
            .iter()
            .position(|&b| b == 0)
            .ok_or(RapError::UnexpectedEof(self.bytes.len()))?;
        let s = decode_string(self.take(len)?);
        self.pos += 1;
        Ok(s)
    }

    fn class_body(&mut self, depth: usize) -> Result<ConfigClass, RapError> {
        if depth > MAX_DEPTH {
            return Err(RapError::TooDeep(self.pos));
        }
        let base = self.asciiz()?;
        let count = self.cint()?;
        let mut entries = Vec::with_capacity(count.min(4096) as usize);
        for _ in 0..count {
            entries.push(self.entry(depth)?);
        }
        Ok(ConfigClass {
            base: (!base.is_empty()).then_some(base),
            entries,
        })
    }

    fn entry(&mut self, depth: usize) -> Result<Entry, RapError> {
        let offset = self.pos;
        let kind = self.u8()?;
        let entry = match kind {
            0 => {
                let name = self.asciiz()?;
                let body = self.u32()? as usize;
                if !self.seen_bodies.insert(body) {
                    return Err(RapError::SharedClassBody(body));
                }
                let resume = self.pos;
                self.pos = body;
                let class = self.class_body(depth + 1)?;
                self.pos = resume;
                Entry::class(name, class)
            }
            1 => {
                let subtype = self.u8()?;
                let name = self.asciiz()?;
                if subtype == 3 {
                    return Err(RapError::UnknownValueType { subtype, offset });
                }
                let value = self.scalar(subtype, depth)?;
                Entry::value(name, value)
            }
            2 => {
                let name = self.asciiz()?;
                Entry::value(name, Value::Array(self.array(depth)?))
            }
            3 => Entry::new(self.asciiz()?, EntryKind::External),
            4 => Entry::new(self.asciiz()?, EntryKind::Delete),
            5 => {
                let flags = self.u32()?;
                if flags != 1 {
                    return Err(RapError::UnknownAppendFlags { flags, offset });
                }
                let name = self.asciiz()?;
                Entry::new(name, EntryKind::ArrayAppend(self.array(depth)?))
            }
            kind => return Err(RapError::UnknownEntryType { kind, offset }),
        };
        Ok(entry)
    }

    fn array(&mut self, depth: usize) -> Result<Vec<Value>, RapError> {
        if depth > MAX_DEPTH {
            return Err(RapError::TooDeep(self.pos));
        }
        let count = self.cint()?;
        let mut items = Vec::with_capacity(count.min(4096) as usize);
        for _ in 0..count {
            let subtype = self.u8()?;
            items.push(self.scalar(subtype, depth)?);
        }
        Ok(items)
    }

    fn scalar(&mut self, subtype: u8, depth: usize) -> Result<Value, RapError> {
        let offset = self.pos;
        Ok(match subtype {
            0 => Value::String(self.asciiz()?),
            1 => Value::Float(self.f32()?),
            2 => Value::Int(self.u32()? as i32),
            3 => Value::Array(self.array(depth + 1)?),
            4 => Value::Expression(self.asciiz()?),
            6 => Value::Int64(self.i64()?),
            subtype => return Err(RapError::UnknownValueType { subtype, offset }),
        })
    }
}

struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn here(&self) -> u32 {
        u32::try_from(self.out.len()).expect("rapified config larger than 4 GiB")
    }

    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    fn placeholder(&mut self) -> usize {
        let at = self.out.len();
        self.u32(0);
        at
    }

    fn patch(&mut self, at: usize, v: u32) {
        self.out[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn cint(&mut self, mut v: u32) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.out.push(b);
                return;
            }
            self.out.push(b | 0x80);
        }
    }

    fn asciiz(&mut self, s: &str) {
        self.out.extend_from_slice(s.as_bytes());
        self.out.push(0);
    }

    fn class_body(&mut self, class: &ConfigClass) {
        self.asciiz(class.base.as_deref().unwrap_or(""));
        self.cint(class.entries.len() as u32);
        let mut children = Vec::new();
        for entry in &class.entries {
            match &entry.kind {
                EntryKind::Class(child) => {
                    self.out.push(0);
                    self.asciiz(&entry.name);
                    children.push((self.placeholder(), child));
                }
                EntryKind::External => {
                    self.out.push(3);
                    self.asciiz(&entry.name);
                }
                EntryKind::Delete => {
                    self.out.push(4);
                    self.asciiz(&entry.name);
                }
                EntryKind::Value(Value::Array(items)) => {
                    self.out.push(2);
                    self.asciiz(&entry.name);
                    self.array(items);
                }
                EntryKind::Value(value) => {
                    self.out.push(1);
                    self.out.push(subtype(value));
                    self.asciiz(&entry.name);
                    self.scalar(value);
                }
                EntryKind::ArrayAppend(items) => {
                    self.out.push(5);
                    self.u32(1);
                    self.asciiz(&entry.name);
                    self.array(items);
                }
            }
        }
        let end_slot = self.placeholder();
        for (slot, child) in children {
            let at = self.here();
            self.patch(slot, at);
            self.class_body(child);
        }
        let end = self.here();
        self.patch(end_slot, end);
    }

    fn array(&mut self, items: &[Value]) {
        self.cint(items.len() as u32);
        for item in items {
            self.out.push(subtype(item));
            self.scalar(item);
        }
    }

    fn scalar(&mut self, value: &Value) {
        match value {
            Value::String(s) | Value::Expression(s) => self.asciiz(s),
            Value::Float(f) => self.u32(f.to_bits()),
            Value::Int(i) => self.u32(*i as u32),
            Value::Int64(i) => self.out.extend_from_slice(&i.to_le_bytes()),
            Value::Array(items) => self.array(items),
        }
    }
}

/// Config strings are UTF-8, but a few shipped files contain Windows-1252 text (e.g. `\x97`
/// for an em dash). Such strings decode as Windows-1252 and are written back as UTF-8.
pub(crate) fn decode_string(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => bytes.iter().map(|&b| windows_1252(b)).collect(),
    }
}

fn windows_1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9f => HIGH[usize::from(b - 0x80)],
        _ => char::from(b),
    }
}

fn subtype(value: &Value) -> u8 {
    match value {
        Value::String(_) => 0,
        Value::Float(_) => 1,
        Value::Int(_) => 2,
        Value::Array(_) => 3,
        Value::Expression(_) => 4,
        Value::Int64(_) => 6,
    }
}
