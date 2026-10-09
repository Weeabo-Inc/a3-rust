//! Little-endian cursor helpers shared by the control payloads and the A2S query answers.
//!
//! Every field on the wire is little-endian and byte aligned, so there is no packing to get wrong;
//! what matters is bounds checking, which is why reads return `Option` and never panic.

/// A forward-only reader over one payload.
#[derive(Debug, Clone)]
pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Bytes left after the cursor.
    pub(crate) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    /// Read `n` raw bytes.
    pub(crate) fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    pub(crate) fn u8(&mut self) -> Option<u8> {
        self.bytes(1).map(|b| b[0])
    }

    pub(crate) fn u32(&mut self) -> Option<u32> {
        self.bytes(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn i32(&mut self) -> Option<i32> {
        self.u32().map(|v| v as i32)
    }

    /// Read a NUL-terminated string of at most `max` bytes.
    ///
    /// The field is fixed width in the message, but the text inside it ends at the first NUL; a
    /// field with no NUL at all is taken whole. Bytes are decoded lossily, as the engine's own
    /// names are not guaranteed to be valid UTF-8.
    pub(crate) fn fixed_string(&mut self, max: usize) -> Option<String> {
        let raw = self.bytes(max)?;
        let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
        Some(String::from_utf8_lossy(&raw[..end]).into_owned())
    }

    /// Read a NUL-terminated string that runs to the end of the payload at most.
    ///
    /// A2S strings are variable length and NUL-terminated, so this consumes up to and including
    /// the terminator; without one the rest of the payload is the string.
    pub(crate) fn string(&mut self) -> Option<String> {
        let rest = self.data.get(self.pos..)?;
        let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
        let text = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.pos += end + usize::from(end < rest.len());
        Some(text)
    }
}

/// Readers the tests use to assert a byte layout field by field.
///
/// The library itself only reads the fields it acts on; these exist so a test can walk a whole
/// answer and show that nothing is left over, which is why they are test-only and stay free of
/// dead-code warnings in the shipped build.
#[cfg(test)]
impl<'a> Cursor<'a> {
    pub(crate) fn u16(&mut self) -> Option<u16> {
        self.bytes(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }

    pub(crate) fn u64(&mut self) -> Option<u64> {
        self.bytes(8)
            .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    pub(crate) fn f32(&mut self) -> Option<f32> {
        self.u32().map(f32::from_bits)
    }
}

/// A growable little-endian writer, used as a consuming builder so that a message reads as one
/// expression: `Writer::new().u32(id).u32(value).finish()`.
#[derive(Debug, Default, Clone)]
pub(crate) struct Writer {
    out: Vec<u8>,
}

impl Writer {
    pub(crate) fn new() -> Self {
        Self { out: Vec::new() }
    }

    pub(crate) fn bytes(mut self, bytes: &[u8]) -> Self {
        self.out.extend_from_slice(bytes);
        self
    }

    pub(crate) fn u8(mut self, value: u8) -> Self {
        self.out.push(value);
        self
    }

    pub(crate) fn u16(mut self, value: u16) -> Self {
        self.out.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub(crate) fn u32(mut self, value: u32) -> Self {
        self.out.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub(crate) fn i32(self, value: i32) -> Self {
        self.u32(value as u32)
    }

    pub(crate) fn u64(mut self, value: u64) -> Self {
        self.out.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub(crate) fn f32(self, value: f32) -> Self {
        self.u32(value.to_bits())
    }

    /// Write a fixed-width, NUL-terminated field of `width` bytes.
    ///
    /// Text longer than `width - 1` bytes is truncated on a char boundary, which is what the
    /// engine's `strncpy`-style copies of these fields amount to.
    pub(crate) fn fixed_string(mut self, text: &str, width: usize) -> Self {
        let mut written = 0;
        for ch in text.chars() {
            let len = ch.len_utf8();
            if written + len > width.saturating_sub(1) {
                break;
            }
            let mut buf = [0u8; 4];
            self.out
                .extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            written += len;
        }
        self.out.resize(self.out.len() + (width - written), 0);
        self
    }

    /// Write a NUL-terminated string of no fixed width.
    pub(crate) fn string(self, text: &str) -> Self {
        self.raw_string(text.as_bytes())
    }

    /// Write raw bytes followed by a NUL, for values that are bytes rather than text.
    pub(crate) fn raw_string(mut self, bytes: &[u8]) -> Self {
        self.out.extend_from_slice(bytes);
        self.out.push(0);
        self
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_fields_and_stops_at_a_nul() {
        let mut cursor = Cursor::new(b"ab\0\0rest");
        assert_eq!(cursor.fixed_string(4).as_deref(), Some("ab"));
        assert_eq!(cursor.bytes(4).map(<[u8]>::to_vec), Some(b"rest".to_vec()));
        assert_eq!(cursor.bytes(1), None);
    }

    #[test]
    fn a_field_without_a_nul_is_taken_whole() {
        let mut cursor = Cursor::new(b"abcd");
        assert_eq!(cursor.fixed_string(4).as_deref(), Some("abcd"));
    }

    #[test]
    fn string_consumes_its_terminator() {
        let mut cursor = Cursor::new(b"hello\0world\0");
        assert_eq!(cursor.string().as_deref(), Some("hello"));
        assert_eq!(cursor.string().as_deref(), Some("world"));
        assert_eq!(cursor.string().as_deref(), Some(""));
    }

    #[test]
    fn fixed_width_writes_are_truncated_and_padded() {
        let bytes = Writer::new().fixed_string("name", 40).finish();
        assert_eq!(bytes.len(), 40);
        assert_eq!(&bytes[..4], b"name");
        assert!(bytes[4..].iter().all(|b| *b == 0));

        assert_eq!(
            Writer::new().fixed_string(&"x".repeat(100), 8).finish(),
            b"xxxxxxx\0"
        );
    }

    #[test]
    fn a_truncated_field_never_splits_a_character() {
        // Four 3-byte characters into a 5-byte field: only one fits with the terminator.
        let bytes = Writer::new().fixed_string("€€€€", 5).finish();
        assert_eq!(bytes.len(), 5);
        assert_eq!(&bytes[..3], "€".as_bytes());
    }
}
