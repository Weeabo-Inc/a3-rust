//! LZO1X decompression.
//!
//! Used by DXT PAA mipmaps whose width field has the top bit set, and by compressed arrays in
//! ODOL and WRP files. The engine links "LZO Professional"; every LZO1X compressor level emits the
//! same stream grammar, so one decoder handles them all.
//!
//! # Stream format
//!
//! A sequence of instructions, each a byte `t` possibly followed by extra length and distance
//! bytes. How `t < 16` is read depends on what came before (the *state*):
//!
//! | `t`      | meaning                                                                         |
//! |----------|---------------------------------------------------------------------------------|
//! | first byte `> 17` | literal run of `t - 17` bytes                                          |
//! | `0..16`, state 0  | literal run of `t + 3` bytes (`t == 0`: extended length, see below)    |
//! | `0..16`, after a literal run of 4+ | 3-byte match, distance `0x801 + (t >> 2) + (b << 2)`  |
//! | `0..16`, after 1-3 trailing literals | 2-byte match, distance `1 + (t >> 2) + (b << 2)`    |
//! | `16..32` (M4) | length `(t & 7) + 2`, distance `0x4000 + (t & 8) << 11 + (d >> 2)`         |
//! | `32..64` (M3) | length `(t & 31) + 2`, distance `1 + (d >> 2)`                             |
//! | `64..256` (M2) | length `(t >> 5) + 1`, distance `1 + ((t >> 2) & 7) + (b << 3)`           |
//!
//! `b` is the next byte, `d` the next two bytes as little-endian `u16`. A zero length field
//! (`t & 7` for M4, `t & 31` for M3, `t` for a literal run) is extended: each following zero byte
//! adds 255, and the first non-zero byte adds its value plus the field's maximum.
//!
//! After every match, the low two bits of `t` (M1, M2) or of `d` (M3, M4) give 0-3 literals that
//! follow directly; with 0 the state returns to 0. An M4 with distance part zero (`11 00 00`) ends
//! the stream.

use std::io::Read;

use crate::Error;
use crate::input::{Input, MAX_PREALLOC, ReadSource, SliceSource, Source};

/// Decompresses an LZO1X block that produces exactly `expected_len` bytes.
///
/// `input` may extend past the block. Returns the output and the number of input bytes the block
/// occupied, end marker included. An `expected_len` of 0 reads nothing: encoders differ on whether
/// empty input becomes an empty block or a bare end marker, and RV formats never compress empty
/// data.
pub fn decompress(input: &[u8], expected_len: usize) -> Result<(Vec<u8>, usize), Error> {
    Decoder::new(SliceSource::new(input), expected_len).run()
}

/// Like [`decompress`], reading from a stream. Reads exactly the bytes of the block, leaving
/// `reader` at the first byte after it.
pub fn decompress_from<R: Read>(reader: R, expected_len: usize) -> Result<(Vec<u8>, usize), Error> {
    Decoder::new(ReadSource(reader), expected_len).run()
}

/// What preceded the current instruction; decides how `t < 16` is read.
#[derive(Clone, Copy)]
enum State {
    /// A match with no trailing literals (or the start of the stream).
    Match,
    /// A match followed by 1-3 literals.
    ShortLiterals,
    /// A literal run of 4 or more bytes.
    LongLiterals,
}

struct Decoder<S> {
    input: Input<S>,
    out: Vec<u8>,
    expected: usize,
}

impl<S: Source> Decoder<S> {
    fn new(source: S, expected: usize) -> Self {
        Self {
            input: Input::new(source, expected),
            out: Vec::with_capacity(expected.min(MAX_PREALLOC)),
            expected,
        }
    }

    fn byte(&mut self) -> Result<usize, Error> {
        self.input.byte(self.out.len()).map(usize::from)
    }

    /// A zero length field extended by the following bytes; `max` is the field's largest value.
    fn extended_length(&mut self, max: usize) -> Result<usize, Error> {
        let mut len = max;
        loop {
            match self.byte()? {
                0 => len += 255,
                b => return Ok(len + b),
            }
            if len > self.expected {
                return Err(self.overrun());
            }
        }
    }

    fn overrun(&self) -> Error {
        Error::OutputOverrun {
            expected: self.expected,
        }
    }

    fn literals(&mut self, n: usize) -> Result<(), Error> {
        if self.out.len() + n > self.expected {
            return Err(self.overrun());
        }
        for _ in 0..n {
            let b = self.input.byte(self.out.len())?;
            self.out.push(b);
        }
        Ok(())
    }

    fn copy_match(&mut self, distance: usize, len: usize) -> Result<(), Error> {
        let at = self.out.len();
        if distance > at {
            return Err(Error::InvalidDistance { at, distance });
        }
        if at + len > self.expected {
            return Err(self.overrun());
        }
        for i in at..at + len {
            let b = self.out[i - distance];
            self.out.push(b);
        }
        Ok(())
    }

    /// Copies the 0-3 literals that follow a match and returns the new state.
    fn trailing_literals(&mut self, n: usize) -> Result<State, Error> {
        if n == 0 {
            return Ok(State::Match);
        }
        self.literals(n)?;
        Ok(State::ShortLiterals)
    }

    fn run(mut self) -> Result<(Vec<u8>, usize), Error> {
        if self.expected == 0 {
            return Ok((self.out, 0));
        }
        let mut state = State::Match;
        let mut t = self.byte()?;
        if t > 17 {
            let n = t - 17;
            self.literals(n)?;
            state = if n < 4 {
                State::ShortLiterals
            } else {
                State::LongLiterals
            };
            t = self.byte()?;
        }

        loop {
            match (t, state) {
                (0..16, State::Match) => {
                    let n = if t == 0 { self.extended_length(15)? } else { t };
                    self.literals(n + 3)?;
                    state = State::LongLiterals;
                }
                (0..16, State::LongLiterals) => {
                    let distance = 0x801 + (t >> 2) + (self.byte()? << 2);
                    self.copy_match(distance, 3)?;
                    state = self.trailing_literals(t & 3)?;
                }
                (0..16, State::ShortLiterals) => {
                    let distance = 1 + (t >> 2) + (self.byte()? << 2);
                    self.copy_match(distance, 2)?;
                    state = self.trailing_literals(t & 3)?;
                }
                (16..32, _) => {
                    let len = match t & 7 {
                        0 => self.extended_length(7)?,
                        n => n,
                    } + 2;
                    let lo = self.byte()?;
                    let hi = self.byte()?;
                    let distance = ((t & 8) << 11) + (lo >> 2) + (hi << 6);
                    if distance == 0 {
                        break;
                    }
                    self.copy_match(distance + 0x4000, len)?;
                    state = self.trailing_literals(lo & 3)?;
                }
                (32..64, _) => {
                    let len = match t & 31 {
                        0 => self.extended_length(31)?,
                        n => n,
                    } + 2;
                    let lo = self.byte()?;
                    let hi = self.byte()?;
                    self.copy_match(1 + (lo >> 2) + (hi << 6), len)?;
                    state = self.trailing_literals(lo & 3)?;
                }
                _ => {
                    let distance = 1 + ((t >> 2) & 7) + (self.byte()? << 3);
                    self.copy_match(distance, (t >> 5) + 1)?;
                    state = self.trailing_literals(t & 3)?;
                }
            }
            t = self.byte()?;
        }

        if self.out.len() != self.expected {
            return Err(Error::OutputUnderrun {
                produced: self.out.len(),
                expected: self.expected,
            });
        }
        let consumed = self.input.consumed();
        Ok((self.out, consumed))
    }
}
