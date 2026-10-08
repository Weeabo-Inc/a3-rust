//! The PAA/PAC container: type tag, TAGGs, palette, mipmap chain.

use a3_compress::lzss::ChecksumKind;

use crate::cursor::Cursor;
use crate::error::malformed;
use crate::{AlphaFlags, Color, Error, PixelFormat, Result, Swizzle};

/// The checksum after LZSS-compressed (non-DXT) mipmaps.
pub(crate) const LZSS_CHECKSUM: ChecksumKind = ChecksumKind::Signed;

/// Set in a DXT mipmap's stored width when its data is LZO1X compressed.
pub(crate) const LZO_FLAG: u16 = 0x8000;

/// Number of `u32` slots in the `OFFS` TAGG.
pub(crate) const OFFSET_SLOTS: usize = 16;

/// How a mipmap's data is stored in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Compression {
    /// Raw.
    #[default]
    None,
    /// LZO1X (DXT formats; flagged by the top bit of the stored width).
    Lzo,
    /// BI LZSS with a signed checksum (non-DXT formats; recognised by a stored size that
    /// differs from the raw size, larger when compression did not pay off).
    Lzss,
}

/// A TAGG this crate does not interpret, kept verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Tagg {
    /// The readable name (`OFFS`, `AVGC`, ...); stored reversed in the file (`GGATSFFO`).
    pub name: [u8; 4],
    /// The TAGG's data.
    pub data: Vec<u8>,
}

/// One level of the mipmap chain, decompressed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Mip {
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// Pixel data in the texture's [`PixelFormat`]: DXT blocks stay block-compressed.
    pub data: Vec<u8>,
    /// How the level is (or will be) stored in the file.
    pub compression: Compression,
}

/// Where one mipmap lies in a PAA file, without its data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MipInfo {
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// How the data is stored.
    pub compression: Compression,
    /// File offset of the mipmap's 7-byte header.
    pub offset: usize,
    /// Size of the stored (possibly compressed) data that follows the header.
    pub stored_len: usize,
}

impl MipInfo {
    /// File offset of the stored data.
    pub fn data_offset(&self) -> usize {
        self.offset + 7
    }
}

/// A texture: pixel format, header TAGGs and the mipmap chain, largest first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Texture {
    /// The pixel format of every mipmap.
    pub format: PixelFormat,
    /// `AVGC`: the average colour, used for distant objects and as a fallback.
    pub average_color: Option<Color>,
    /// `MAXC`: the maximum of each channel; the engine scales the texture by it. Almost always
    /// white.
    pub max_color: Option<Color>,
    /// `FLAG`: how alpha is used.
    pub flags: Option<AlphaFlags>,
    /// `SWIZ`: the channel rearrangement applied before compressing.
    pub swizzle: Option<Swizzle>,
    /// `PROC`: procedural texture text the texture was generated from.
    pub procedural: Option<String>,
    /// TAGGs this crate does not interpret, in file order.
    pub other_taggs: Vec<Tagg>,
    /// Palette (B, G, R triples). Empty in every shipped texture.
    pub palette: Vec<[u8; 3]>,
    /// The mipmap chain, largest first.
    pub mips: Vec<Mip>,
}

/// The parsed header of a PAA file and the location of every mipmap, without decompressing
/// anything. Cheap; for streaming individual mipmaps and for tools.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaaHeader {
    /// Everything except the mipmap data (`mips` is empty).
    pub meta: Texture,
    /// The `OFFS` TAGG: file offsets of the mipmaps as declared (zero slots dropped). Empty when
    /// the TAGG is missing or all zero (some old PAC files).
    pub declared_offsets: Vec<u32>,
    /// Where each mipmap lies.
    pub mips: Vec<MipInfo>,
}

impl PaaHeader {
    /// Parses the header of the PAA file in `data` and walks the mipmap chain.
    pub fn read(data: &[u8]) -> Result<Self> {
        let mut cur = Cursor::new(data);
        let tag = cur.u16("type tag")?;
        let format = PixelFormat::from_tag(tag).ok_or(Error::UnknownFormat(tag))?;
        let mut header = PaaHeader {
            meta: Texture {
                format,
                ..Texture::default()
            },
            ..PaaHeader::default()
        };

        while cur.rest().starts_with(b"GGAT") {
            cur.skip(4, "TAGG signature")?;
            let mut name = cur.array::<4>("TAGG name")?;
            name.reverse();
            let len = cur.u32("TAGG length")? as usize;
            let body = cur.bytes(len, "TAGG data")?;
            header.apply_tagg(name, body)?;
        }

        let colors = cur.u16("palette size")?;
        let palette = cur.bytes(usize::from(colors) * 3, "palette")?;
        header.meta.palette = palette
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect();

        loop {
            let offset = cur.pos();
            if cur.remaining() < 4 {
                // Some files end right after the last mipmap.
                break;
            }
            let stored_width = cur.u16("mipmap width")?;
            let height = cur.u16("mipmap height")?;
            if stored_width == 0 && height == 0 {
                break;
            }
            if stored_width == 1234 && height == 8765 {
                return malformed("PAA", "OFP-era indexed mipmap (1234x8765 marker)");
            }
            let stored_len = cur.u24("mipmap size")? as usize;
            let (width, compression) = if format.is_dxt() {
                if stored_width & LZO_FLAG != 0 {
                    (stored_width & !LZO_FLAG, Compression::Lzo)
                } else {
                    (stored_width, Compression::None)
                }
            } else if stored_len != format.data_len(stored_width, height) {
                (stored_width, Compression::Lzss)
            } else {
                (stored_width, Compression::None)
            };
            if width == 0 || height == 0 {
                return malformed("PAA", format!("mipmap {width}x{height} at byte {offset}"));
            }
            cur.skip(stored_len, "mipmap data")?;
            header.mips.push(MipInfo {
                width,
                height,
                compression,
                offset,
                stored_len,
            });
        }
        if header.mips.is_empty() {
            return malformed("PAA", "no mipmaps");
        }
        Ok(header)
    }

    fn apply_tagg(&mut self, name: [u8; 4], body: &[u8]) -> Result<()> {
        let four = |what: &str| -> Result<[u8; 4]> {
            body.get(..4)
                .map(|b| b.try_into().expect("four bytes"))
                .ok_or_else(|| Error::Malformed {
                    format: "PAA",
                    reason: format!("{what} TAGG has {} bytes", body.len()),
                })
        };
        match &name {
            b"AVGC" => self.meta.average_color = Some(Color::from_bgra(four("AVGC")?)),
            b"MAXC" => self.meta.max_color = Some(Color::from_bgra(four("MAXC")?)),
            b"FLAG" => self.meta.flags = Some(AlphaFlags(u32::from_le_bytes(four("FLAG")?))),
            b"SWIZ" => self.meta.swizzle = Some(Swizzle::from_bytes(four("SWIZ")?)),
            b"PROC" => {
                let text = body.split(|&b| b == 0).next().unwrap_or_default();
                self.meta.procedural = Some(String::from_utf8_lossy(text).into_owned());
            }
            b"OFFS" => {
                self.declared_offsets = body
                    .chunks_exact(4)
                    .map(|c| u32::from_le_bytes(c.try_into().expect("four bytes")))
                    .filter(|&o| o != 0)
                    .collect();
            }
            _ => self.meta.other_taggs.push(Tagg {
                name,
                data: body.to_vec(),
            }),
        }
        Ok(())
    }

    /// Reads and decompresses mipmap `index` from the same `data` the header was read from.
    pub fn read_mip(&self, data: &[u8], index: usize) -> Result<Mip> {
        let info = self.mips.get(index).ok_or_else(|| Error::Malformed {
            format: "PAA",
            reason: format!("no mipmap {index} (texture has {})", self.mips.len()),
        })?;
        let stored = data
            .get(info.data_offset()..info.data_offset() + info.stored_len)
            .ok_or(Error::Truncated {
                offset: data.len(),
                what: "mipmap data",
            })?;
        let raw_len = self.meta.format.data_len(info.width, info.height);
        let decompress_error = |source| Error::Decompress {
            index,
            width: info.width,
            height: info.height,
            source,
        };
        let data = match info.compression {
            Compression::None => {
                if stored.len() != raw_len {
                    return malformed(
                        "PAA",
                        format!(
                            "mipmap {index} ({}x{}) stores {} bytes, expected {raw_len}",
                            info.width,
                            info.height,
                            stored.len()
                        ),
                    );
                }
                stored.to_vec()
            }
            Compression::Lzo => {
                a3_compress::lzo::decompress(stored, raw_len)
                    .map_err(decompress_error)?
                    .0
            }
            Compression::Lzss => {
                a3_compress::lzss::decompress(stored, raw_len, LZSS_CHECKSUM)
                    .map_err(decompress_error)?
                    .0
            }
        };
        Ok(Mip {
            width: info.width,
            height: info.height,
            data,
            compression: info.compression,
        })
    }
}

impl Texture {
    /// Parses a PAA/PAC file and decompresses every mipmap.
    pub fn read(data: &[u8]) -> Result<Self> {
        let header = PaaHeader::read(data)?;
        let mips = (0..header.mips.len())
            .map(|i| header.read_mip(data, i))
            .collect::<Result<Vec<_>>>()?;
        Ok(Texture {
            mips,
            ..header.meta
        })
    }

    /// Width of the largest mipmap (0 without mipmaps).
    pub fn width(&self) -> u16 {
        self.mips.first().map_or(0, |m| m.width)
    }

    /// Height of the largest mipmap (0 without mipmaps).
    pub fn height(&self) -> u16 {
        self.mips.first().map_or(0, |m| m.height)
    }
}
