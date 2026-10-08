//! Little-endian reading and writing primitives.

use std::borrow::Cow;

use glam::Vec3;

use crate::{Error, Result};

/// Arrays at least this long are stored compressed; shorter ones are stored raw.
pub(crate) const COMPRESS_THRESHOLD: usize = 1024;

/// The codec of compressed arrays, chosen by the file version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Codec {
    /// Versions before 23.
    Lzss,
    /// Version 23 and later.
    Lzo,
}

pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub(crate) fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(Error::Truncated {
                offset: self.pos,
                what,
            });
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    pub(crate) fn array<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N]> {
        let bytes = self.take(N, what)?;
        let mut out = [0; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    pub(crate) fn u8(&mut self, what: &'static str) -> Result<u8> {
        Ok(self.array::<1>(what)?[0])
    }

    pub(crate) fn u16(&mut self, what: &'static str) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array(what)?))
    }

    pub(crate) fn u32(&mut self, what: &'static str) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array(what)?))
    }

    pub(crate) fn f32(&mut self, what: &'static str) -> Result<f32> {
        Ok(f32::from_le_bytes(self.array(what)?))
    }

    pub(crate) fn vec3(&mut self, what: &'static str) -> Result<Vec3> {
        Ok(Vec3::new(self.f32(what)?, self.f32(what)?, self.f32(what)?))
    }

    pub(crate) fn f32s<const N: usize>(&mut self, what: &'static str) -> Result<[f32; N]> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = self.f32(what)?;
        }
        Ok(out)
    }

    /// A count field, checked against the bytes left so a corrupt count cannot trigger a huge
    /// allocation: every counted item takes at least `min_item_size` bytes.
    pub(crate) fn count(&mut self, min_item_size: usize, what: &'static str) -> Result<usize> {
        let offset = self.pos;
        let n = self.u32(what)? as usize;
        if n.saturating_mul(min_item_size.max(1)) > self.remaining() {
            return Err(Error::Invalid {
                offset,
                what,
                detail: format!("count {n} exceeds the remaining data"),
            });
        }
        Ok(n)
    }

    /// A NUL-terminated string. Bytes outside ASCII are mapped as Latin-1.
    pub(crate) fn asciiz(&mut self, what: &'static str) -> Result<String> {
        let rest = &self.data[self.pos..];
        let Some(len) = rest.iter().position(|&b| b == 0) else {
            return Err(Error::Truncated {
                offset: self.pos,
                what,
            });
        };
        let bytes = &rest[..len];
        self.pos += len + 1;
        Ok(match std::str::from_utf8(bytes) {
            Ok(s) if s.is_ascii() => s.to_owned(),
            _ => bytes.iter().map(|&b| char::from(b)).collect(),
        })
    }

    /// A "compressed array" of exactly `len` bytes: stored raw below
    /// [`COMPRESS_THRESHOLD`] bytes, compressed with `codec` otherwise.
    pub(crate) fn compressed(
        &mut self,
        len: usize,
        codec: Codec,
        what: &'static str,
    ) -> Result<Cow<'a, [u8]>> {
        if len < COMPRESS_THRESHOLD {
            return Ok(Cow::Borrowed(self.take(len, what)?));
        }
        let offset = self.pos;
        let input = &self.data[self.pos..];
        let (out, consumed) = crate::compress::decompress(codec, input, len).map_err(|detail| {
            Error::Decompress {
                offset,
                what,
                detail,
            }
        })?;
        self.pos += consumed;
        Ok(Cow::Owned(out))
    }
}

/// A growing little-endian output buffer.
#[derive(Default)]
pub(crate) struct Writer {
    pub(crate) buf: Vec<u8>,
}

impl Writer {
    pub(crate) fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    pub(crate) fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub(crate) fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    pub(crate) fn f32(&mut self, v: f32) {
        self.bytes(&v.to_le_bytes());
    }

    pub(crate) fn vec3(&mut self, v: Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }

    pub(crate) fn asciiz(&mut self, s: &str) -> Result<()> {
        if s.contains('\0') {
            return Err(Error::Write(format!("string {s:?} contains NUL")));
        }
        for c in s.chars() {
            let b = u8::try_from(u32::from(c))
                .map_err(|_| Error::Write(format!("string {s:?} is not Latin-1")))?;
            self.buf.push(b);
        }
        self.buf.push(0);
        Ok(())
    }

    /// Writes a "compressed array". Only arrays below [`COMPRESS_THRESHOLD`] bytes, which are
    /// stored raw, are supported: the writer exists to build small test terrains.
    pub(crate) fn compressed(&mut self, data: &[u8], what: &str) -> Result<()> {
        if data.len() >= COMPRESS_THRESHOLD {
            return Err(Error::Write(format!(
                "{what} is {} bytes; arrays of {COMPRESS_THRESHOLD} bytes or more must be \
                 compressed, which the writer does not support",
                data.len()
            )));
        }
        self.bytes(data);
        Ok(())
    }
}
