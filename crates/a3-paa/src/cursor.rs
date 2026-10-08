//! A bounds-checked little-endian reader over a byte slice.

use crate::{Error, Result};

pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub(crate) fn rest(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    pub(crate) fn bytes(&mut self, len: usize, what: &'static str) -> Result<&'a [u8]> {
        if self.remaining() < len {
            return Err(Error::Truncated {
                offset: self.pos,
                what,
            });
        }
        let out = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(out)
    }

    pub(crate) fn skip(&mut self, len: usize, what: &'static str) -> Result<()> {
        self.bytes(len, what).map(|_| ())
    }

    pub(crate) fn array<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N]> {
        Ok(self.bytes(N, what)?.try_into().expect("length checked"))
    }

    pub(crate) fn u8(&mut self, what: &'static str) -> Result<u8> {
        Ok(self.array::<1>(what)?[0])
    }

    pub(crate) fn u16(&mut self, what: &'static str) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array(what)?))
    }

    pub(crate) fn u24(&mut self, what: &'static str) -> Result<u32> {
        let [a, b, c] = self.array(what)?;
        Ok(u32::from_le_bytes([a, b, c, 0]))
    }

    pub(crate) fn u32(&mut self, what: &'static str) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array(what)?))
    }

    pub(crate) fn f32(&mut self, what: &'static str) -> Result<f32> {
        Ok(f32::from_le_bytes(self.array(what)?))
    }

    /// A NUL-terminated string (bytes kept as Latin-1-ish; invalid UTF-8 replaced).
    pub(crate) fn cstr(&mut self, what: &'static str) -> Result<String> {
        let rest = self.rest();
        let Some(len) = rest.iter().position(|&b| b == 0) else {
            return Err(Error::Truncated {
                offset: self.data.len(),
                what,
            });
        };
        let text = String::from_utf8_lossy(&rest[..len]).into_owned();
        self.pos += len + 1;
        Ok(text)
    }
}
