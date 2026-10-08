//! Reader for `.fxy` bitmap font metadata.
//!
//! A font of one size is a `.fxy` glyph table plus texture pages `<name>-01.paa`,
//! `<name>-02.paa`, ... Each [`Glyph`] gives the page and the pixel rectangle of a character,
//! how far to offset it when drawing and how far to advance the pen. Three layouts ship:
//!
//! - `BIFo` version 0x102: page blocks with page metrics, kerning pairs and glyphs.
//! - `BIFo` version 0x101: fixed 14-byte glyph records with an advance.
//! - Unversioned (no signature): fixed 12-byte glyph records; the advance is the width.
//!
//! See `docs/re/fxy.md`.

use std::fmt;

/// Errors from reading an FXY file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file ends inside a record.
    #[error("FXY is truncated at byte {offset}")]
    Truncated {
        /// Byte offset at which more data was expected.
        offset: usize,
    },
    /// A `BIFo` version newer than 0x102.
    #[error("unsupported FXY version {0:#x}")]
    UnsupportedVersion(u32),
    /// A count that cannot fit in the file.
    #[error("malformed FXY: {0}")]
    Malformed(String),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// The layout of an FXY file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxyVersion {
    /// No signature: 12-byte glyph records.
    Legacy,
    /// `BIFo` 0x101: 14-byte glyph records.
    V101,
    /// `BIFo` 0x102: page blocks with metrics and kerning.
    V102,
}

/// Metrics of one texture page (version 0x102 only; zero otherwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    /// Page number, from 1; the texture is `<font>-NN.paa`.
    pub index: u16,
    /// Line height in pixels _(uncertain meaning)_.
    pub height: i32,
    /// Distance from the top of the line to the baseline in pixels _(uncertain meaning)_.
    pub ascent: i32,
}

/// One character's image on a texture page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    /// Character code (a UTF-16 code unit; every shipped code is in the Basic Multilingual Plane).
    pub code: u16,
    /// Texture page number, from 1.
    pub page: u16,
    /// Left edge on the page, in pixels.
    pub x: u16,
    /// Top edge on the page, in pixels.
    pub y: u16,
    /// Width of the image, in pixels.
    pub width: u16,
    /// Height of the image, in pixels.
    pub height: u16,
    /// Horizontal drawing offset (0 before version 0x102).
    pub offset_x: i32,
    /// Vertical drawing offset (0 before version 0x102).
    pub offset_y: i32,
    /// Pen advance after the character, in pixels.
    pub advance: u32,
}

/// A spacing adjustment between two characters (version 0x102).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KerningPair {
    /// The character on the left.
    pub first: u16,
    /// The character on the right.
    pub second: u16,
    /// Pixels to add to the advance of `first` when followed by `second`.
    pub amount: i32,
}

/// A decoded FXY file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    /// The layout the file used.
    pub version: FxyVersion,
    /// Texture pages in file order (every page any glyph uses).
    pub pages: Vec<Page>,
    /// Glyphs in file order.
    pub glyphs: Vec<Glyph>,
    /// Kerning pairs in file order.
    pub kerning: Vec<KerningPair>,
}

impl Font {
    /// Decodes an FXY file.
    pub fn read(data: &[u8]) -> Result<Self> {
        let mut r = Reader { data, pos: 0 };
        if data.starts_with(b"BIFo") {
            r.pos = 4;
            match r.u32()? {
                0x102 => return read_v102(r),
                0x101 => return read_records(r, FxyVersion::V101),
                v if v > 0x102 => return Err(Error::UnsupportedVersion(v)),
                _ => {}
            }
        }
        r.pos = 0;
        read_records(r, FxyVersion::Legacy)
    }

    /// The glyph of `c`, if the font has one.
    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        let code = u16::try_from(u32::from(c)).ok()?;
        self.glyphs.iter().find(|g| g.code == code)
    }

    /// Kerning between `first` and `second` (0 when the font has none).
    pub fn kerning(&self, first: char, second: char) -> i32 {
        let (Ok(a), Ok(b)) = (
            u16::try_from(u32::from(first)),
            u16::try_from(u32::from(second)),
        ) else {
            return 0;
        };
        self.kerning
            .iter()
            .find(|k| k.first == a && k.second == b)
            .map_or(0, |k| k.amount)
    }
}

impl fmt::Display for FxyVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Legacy => "legacy (unversioned)",
            Self::V101 => "BIFo 0x101",
            Self::V102 => "BIFo 0x102",
        })
    }
}

/// The texture of page `page` of the font at `fxy_path` (`name.fxy` -> `name-NN.paa`).
pub fn page_texture_path(fxy_path: &str, page: u16) -> String {
    let stem = fxy_path
        .len()
        .checked_sub(4)
        .filter(|&i| fxy_path[i..].eq_ignore_ascii_case(".fxy"))
        .map_or(fxy_path, |i| &fxy_path[..i]);
    format!("{stem}-{page:02}.paa")
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn at_end(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        let out = self
            .data
            .get(self.pos..self.pos + N)
            .ok_or(Error::Truncated {
                offset: self.data.len(),
            })?;
        self.pos += N;
        Ok(out.try_into().expect("N bytes"))
    }

    fn u16(&mut self) -> Result<u16> {
        self.bytes().map(u16::from_le_bytes)
    }

    fn u32(&mut self) -> Result<u32> {
        self.bytes().map(u32::from_le_bytes)
    }

    fn i32(&mut self) -> Result<i32> {
        self.bytes().map(i32::from_le_bytes)
    }

    /// An `i32` count of records of `size` bytes, checked against the rest of the file.
    fn count(&mut self, size: usize) -> Result<usize> {
        let at = self.pos;
        let n = self.i32()?;
        let n = usize::try_from(n)
            .map_err(|_| Error::Malformed(format!("negative count {n} at byte {at}")))?;
        if n.saturating_mul(size) > self.data.len() - self.pos {
            return Err(Error::Malformed(format!(
                "count {n} at byte {at} exceeds the file size"
            )));
        }
        Ok(n)
    }
}

fn add_page(pages: &mut Vec<Page>, index: u16) -> &mut Page {
    if let Some(i) = pages.iter().position(|p| p.index == index) {
        return &mut pages[i];
    }
    pages.push(Page {
        index,
        height: 0,
        ascent: 0,
    });
    pages.last_mut().expect("just pushed")
}

/// Version 0x102: page blocks to the end of the file.
fn read_v102(mut r: Reader) -> Result<Font> {
    let mut font = Font {
        version: FxyVersion::V102,
        pages: Vec::new(),
        glyphs: Vec::new(),
        kerning: Vec::new(),
    };
    while !r.at_end() {
        let index = r.u16()?;
        let height = r.i32()?;
        let ascent = r.i32()?;
        let page = add_page(&mut font.pages, index);
        page.height = height;
        page.ascent = ascent;
        for _ in 0..r.count(8)? {
            font.kerning.push(KerningPair {
                first: r.u16()?,
                second: r.u16()?,
                amount: r.i32()?,
            });
        }
        for _ in 0..r.count(22)? {
            font.glyphs.push(Glyph {
                code: r.u16()?,
                x: r.u16()?,
                y: r.u16()?,
                width: r.u16()?,
                height: r.u16()?,
                offset_x: r.i32()?,
                offset_y: r.i32()?,
                advance: r.u32()?,
                page: index,
            });
        }
    }
    Ok(font)
}

/// Unversioned and 0x101: fixed glyph records to the end of the file.
fn read_records(mut r: Reader, version: FxyVersion) -> Result<Font> {
    let mut font = Font {
        version,
        pages: Vec::new(),
        glyphs: Vec::new(),
        kerning: Vec::new(),
    };
    while !r.at_end() {
        // Codes are stored minus 0x20 (the first printable character).
        let code = r.u16()?.wrapping_add(0x20);
        let page = r.u16()?;
        let (x, y, width, height) = (r.u16()?, r.u16()?, r.u16()?, r.u16()?);
        let advance = match version {
            FxyVersion::Legacy => u32::from(width),
            _ => u32::from(r.u16()?),
        };
        add_page(&mut font.pages, page);
        font.glyphs.push(Glyph {
            code,
            page,
            x,
            y,
            width,
            height,
            offset_x: 0,
            offset_y: 0,
            advance,
        });
    }
    Ok(font)
}
