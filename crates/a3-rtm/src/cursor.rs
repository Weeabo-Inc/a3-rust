//! Little-endian reads over a byte slice that report the offset of a short read.

use glam::Vec3;

use crate::{Error, Result};

pub struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&end| end <= self.data.len())
            .ok_or(Error::Truncated {
                offset: self.data.len().min(self.pos.saturating_add(n)),
            })?;
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.bytes(N)?.try_into().expect("length checked"))
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u32(&mut self) -> Result<u32> {
        self.array().map(u32::from_le_bytes)
    }

    /// A `u32` element count, rejected when even one byte per element would overrun the file.
    pub fn count(&mut self, min_element_size: usize) -> Result<usize> {
        let offset = self.pos;
        let n = self.u32()? as usize;
        if n.saturating_mul(min_element_size) > self.data.len() - self.pos {
            return Err(Error::Malformed(format!(
                "count {n} at byte {offset} exceeds the file size"
            )));
        }
        Ok(n)
    }

    pub fn i32(&mut self) -> Result<i32> {
        self.array().map(i32::from_le_bytes)
    }

    pub fn f32(&mut self) -> Result<f32> {
        self.array().map(f32::from_le_bytes)
    }

    pub fn vec3(&mut self) -> Result<Vec3> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }

    /// A NUL-terminated string (bytes as Latin-1).
    pub fn cstr(&mut self) -> Result<String> {
        let rest = self.remaining();
        let len = rest.iter().position(|&b| b == 0).ok_or(Error::Truncated {
            offset: self.data.len(),
        })?;
        let text = latin1(&rest[..len]);
        self.pos += len + 1;
        Ok(text)
    }

    /// A fixed-size field holding a NUL-padded string.
    pub fn fixed_str(&mut self, size: usize) -> Result<String> {
        let field = self.bytes(size)?;
        let len = field.iter().position(|&b| b == 0).unwrap_or(size);
        Ok(latin1(&field[..len]))
    }

    /// A string prefixed by its `u32` byte length.
    pub fn sized_str(&mut self) -> Result<String> {
        let len = self.count(1)?;
        Ok(latin1(self.bytes(len)?))
    }
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}
