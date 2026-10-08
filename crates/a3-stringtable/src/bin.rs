//! Binarized `stringtable.bin`:
//!
//! ```text
//! "BLMX"
//! u32 language_count, asciiz languages[]
//! u32 offset_count,   i32 offsets[]        file offset of each language's column
//! u32 key_count,      asciiz keys[]
//! per language, at its offset: u32 count (== key_count), asciiz texts[]
//! ```

use crate::{Entry, Error, Result, Stringtable};

pub const SIGNATURE: &[u8] = b"BLMX";

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn u32(&mut self) -> Result<u32> {
        let bytes = self
            .data
            .get(self.pos..self.pos + 4)
            .ok_or(Error::Truncated {
                offset: self.data.len(),
            })?;
        self.pos += 4;
        Ok(u32::from_le_bytes(bytes.try_into().expect("4 bytes")))
    }

    /// A count of items each at least `min_size` bytes long.
    fn count(&mut self, min_size: usize) -> Result<usize> {
        let at = self.pos;
        let n = self.u32()? as usize;
        if n.saturating_mul(min_size) > self.data.len().saturating_sub(self.pos) {
            return Err(Error::Malformed(format!(
                "count {n} at byte {at} exceeds the file size"
            )));
        }
        Ok(n)
    }

    fn cstr(&mut self) -> Result<String> {
        let rest = &self.data[self.pos..];
        let len = rest.iter().position(|&b| b == 0).ok_or(Error::Truncated {
            offset: self.data.len(),
        })?;
        let text = String::from_utf8_lossy(&rest[..len]).into_owned();
        self.pos += len + 1;
        Ok(text)
    }

    fn strings(&mut self) -> Result<Vec<String>> {
        let n = self.count(1)?;
        (0..n).map(|_| self.cstr()).collect()
    }
}

pub fn read(data: &[u8]) -> Result<Stringtable> {
    let mut c = Cursor { data, pos: 4 };
    let languages = c.strings()?;
    let offset_count = c.count(4)?;
    let offsets = (0..offset_count)
        .map(|_| c.u32())
        .collect::<Result<Vec<_>>>()?;
    if offsets.len() != languages.len() {
        return Err(Error::Malformed(format!(
            "{} languages but {} column offsets",
            languages.len(),
            offsets.len()
        )));
    }
    let keys = c.strings()?;
    let mut columns = Vec::with_capacity(languages.len());
    for (language, &offset) in languages.iter().zip(&offsets) {
        let offset = offset as usize;
        if offset > data.len() {
            return Err(Error::Malformed(format!(
                "column {language:?} at byte {offset} is past the end"
            )));
        }
        let column = Cursor { data, pos: offset }.strings()?;
        if column.len() != keys.len() {
            return Err(Error::Malformed(format!(
                "column {language:?} has {} texts for {} keys",
                column.len(),
                keys.len()
            )));
        }
        columns.push(column.into_iter());
    }
    let entries = keys
        .into_iter()
        .map(|key| Entry {
            key,
            translations: languages
                .iter()
                .zip(columns.iter_mut())
                .map(|(l, column)| (l.clone(), column.next().expect("length checked")))
                .collect(),
        })
        .collect();
    Ok(Stringtable { entries })
}

pub fn write(table: &Stringtable) -> Vec<u8> {
    let languages = table.languages();
    let mut out = SIGNATURE.to_vec();
    let put_u32 = |out: &mut Vec<u8>, v: usize| {
        out.extend_from_slice(&u32::try_from(v).expect("fits u32").to_le_bytes());
    };
    let put_str = |out: &mut Vec<u8>, s: &str| {
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    };
    put_u32(&mut out, languages.len());
    for language in &languages {
        put_str(&mut out, language);
    }
    put_u32(&mut out, languages.len());
    let offsets_at = out.len();
    out.resize(out.len() + 4 * languages.len(), 0);
    put_u32(&mut out, table.entries.len());
    for entry in &table.entries {
        put_str(&mut out, &entry.key);
    }
    for (i, language) in languages.iter().enumerate() {
        let offset = out.len();
        out[offsets_at + 4 * i..offsets_at + 4 * i + 4]
            .copy_from_slice(&u32::try_from(offset).expect("fits u32").to_le_bytes());
        put_u32(&mut out, table.entries.len());
        for entry in &table.entries {
            put_str(&mut out, entry.resolve(language).unwrap_or(""));
        }
    }
    out
}
