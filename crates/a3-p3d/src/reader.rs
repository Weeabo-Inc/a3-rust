//! Little-endian cursor over a byte slice with offset-carrying errors.

use glam::{Mat3, Vec3};

use crate::error::{Error, Result};

pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn malformed(&self, message: impl Into<String>) -> Error {
        Error::Malformed {
            offset: self.pos,
            message: message.into(),
        }
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(Error::Truncated {
                offset: self.pos,
                needed: n - self.remaining(),
            });
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub fn i8(&mut self) -> Result<i8> {
        Ok(i8::from_le_bytes(self.array()?))
    }

    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    pub fn vec3(&mut self) -> Result<Vec3> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }

    /// Nine floats, column by column (the engine's `Matrix3` layout, _medium_ confidence).
    pub fn mat3(&mut self) -> Result<Mat3> {
        Ok(Mat3::from_cols(self.vec3()?, self.vec3()?, self.vec3()?))
    }

    /// A count of elements that each take at least `min_size` bytes; rejects counts larger than
    /// what is left, so corrupt input cannot trigger huge allocations.
    pub fn count(&mut self, min_size: usize) -> Result<usize> {
        let start = self.pos;
        let n = self.u32()? as usize;
        if n.saturating_mul(min_size) > self.remaining() {
            return Err(Error::Malformed {
                offset: start,
                message: format!("count {n} exceeds the remaining {} bytes", self.remaining()),
            });
        }
        Ok(n)
    }

    /// A NUL-terminated string. Bytes are taken as Latin-1, so any input decodes.
    pub fn asciiz(&mut self) -> Result<String> {
        let rest = &self.data[self.pos..];
        let Some(len) = rest.iter().position(|&b| b == 0) else {
            return Err(Error::Truncated {
                offset: self.pos,
                needed: 1,
            });
        };
        let s = latin1(&rest[..len]);
        self.pos += len + 1;
        Ok(s)
    }

    /// A fixed-size, NUL-padded string field.
    pub fn fixed_str(&mut self, size: usize) -> Result<String> {
        let raw = self.bytes(size)?;
        let len = raw.iter().position(|&b| b == 0).unwrap_or(size);
        Ok(latin1(&raw[..len]))
    }
}

pub(crate) fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}
