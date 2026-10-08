//! Byte-at-a-time input shared by the decoders.
//!
//! The decoders pull one byte at a time so that, when they read from a stream, they never read
//! past the end of the compressed block: the stream is left positioned at the first byte after
//! it, which is what inline compressed arrays in ODOL and WRP need.

use std::io::{self, Read};

use crate::Error;

/// Upper bound for reserving output space up front, so a corrupt length field cannot trigger a
/// huge allocation before any input is read. Larger outputs still grow as needed.
pub(crate) const MAX_PREALLOC: usize = 16 << 20;

/// A source of compressed bytes.
pub(crate) trait Source {
    /// The next byte, or `None` at the end of the input.
    fn next_byte(&mut self) -> io::Result<Option<u8>>;
}

/// A borrowed slice.
pub(crate) struct SliceSource<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> SliceSource<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
}

impl Source for SliceSource<'_> {
    fn next_byte(&mut self) -> io::Result<Option<u8>> {
        let b = self.bytes.get(self.pos).copied();
        if b.is_some() {
            self.pos += 1;
        }
        Ok(b)
    }
}

/// Any [`Read`], read one byte per call. Wrap unbuffered readers in a `BufReader`.
pub(crate) struct ReadSource<R>(pub(crate) R);

impl<R: Read> Source for ReadSource<R> {
    fn next_byte(&mut self) -> io::Result<Option<u8>> {
        let mut b = [0u8; 1];
        loop {
            match self.0.read(&mut b) {
                Ok(0) => return Ok(None),
                Ok(_) => return Ok(Some(b[0])),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

/// Counts consumed bytes and turns a premature end of input into [`Error::UnexpectedEof`].
pub(crate) struct Input<S> {
    source: S,
    consumed: usize,
    expected: usize,
}

impl<S: Source> Input<S> {
    pub(crate) fn new(source: S, expected: usize) -> Self {
        Self {
            source,
            consumed: 0,
            expected,
        }
    }

    pub(crate) fn consumed(&self) -> usize {
        self.consumed
    }

    /// The next byte. `produced` is the output length so far, used for the error message.
    pub(crate) fn byte(&mut self, produced: usize) -> Result<u8, Error> {
        match self.source.next_byte()? {
            Some(b) => {
                self.consumed += 1;
                Ok(b)
            }
            None => Err(Error::UnexpectedEof {
                consumed: self.consumed,
                produced,
                expected: self.expected,
            }),
        }
    }

    /// Reads a little-endian `u32`.
    pub(crate) fn u32_le(&mut self, produced: usize) -> Result<u32, Error> {
        let mut b = [0u8; 4];
        for x in &mut b {
            *x = self.byte(produced)?;
        }
        Ok(u32::from_le_bytes(b))
    }
}
