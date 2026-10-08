//! Background loading of full-resolution satellite tiles.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use a3_core::VfsPath;
use a3_paa::{PaaHeader, PixelFormat};
use a3_render::{TextureData, TextureFormat};

/// Reads a VFS file; shared with the loader threads.
pub type FileReader = Arc<dyn Fn(&VfsPath) -> Option<Vec<u8>> + Send + Sync>;

/// The shape every streamed tile is converted to: one layer of a tile texture array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileFormat {
    /// The array's pixel format: BC1 for DXT1 and BC3 for DXT5 tiles when the GPU samples BC,
    /// otherwise RGBA8 (decoded).
    pub format: TextureFormat,
    /// Edge of mip 0 in pixels.
    pub size: u32,
    /// Mip levels, largest first.
    pub mips: u32,
}

impl TileFormat {
    /// The format for tiles shaped like the PAA in `bytes`. DXT1 and DXT5 tiles stay
    /// block-compressed when the GPU samples BC (`bc_supported`); everything else is decoded
    /// to RGBA8.
    pub fn probe(bytes: &[u8], bc_supported: bool) -> Option<TileFormat> {
        let header = PaaHeader::read(bytes).ok()?;
        let first = header.mips.first()?;
        let size = u32::from(first.width);
        if size == 0 || u32::from(first.height) != size {
            return None;
        }
        let mips = header
            .mips
            .iter()
            .enumerate()
            .take_while(|(i, m)| u32::from(m.width) == size >> i && size >> i >= 4)
            .count() as u32;
        Some(TileFormat {
            format: match header.meta.format {
                PixelFormat::Dxt1 if bc_supported => TextureFormat::Bc1,
                PixelFormat::Dxt5 if bc_supported => TextureFormat::Bc3,
                _ => TextureFormat::Rgba8,
            },
            size,
            mips,
        })
    }

    /// Convert one tile PAA. `None` when it does not match this format.
    pub fn decode(&self, bytes: &[u8]) -> Option<TextureData> {
        let header = PaaHeader::read(bytes).ok()?;
        let native = match self.format {
            TextureFormat::Bc1 => Some(PixelFormat::Dxt1),
            TextureFormat::Bc3 => Some(PixelFormat::Dxt5),
            _ => None,
        };
        if header.mips.len() < self.mips as usize
            || header.mips.first().map(|m| u32::from(m.width)) != Some(self.size)
            || native.is_some_and(|f| f != header.meta.format)
        {
            return None;
        }
        let mut mips = Vec::with_capacity(self.mips as usize);
        for index in 0..self.mips as usize {
            let mip = header.read_mip(bytes, index).ok()?;
            if u32::from(mip.width) != self.size >> index {
                return None;
            }
            if native.is_some() {
                mips.push(mip.data);
            } else {
                let mut rgba = a3_paa::decode_rgba8(header.meta.format, &mip).ok()?;
                if let Some(swizzle) = header.meta.swizzle {
                    swizzle.restore(&mut rgba);
                }
                mips.push(rgba);
            }
        }
        let data = TextureData {
            format: self.format,
            width: self.size,
            height: self.size,
            mips,
        };
        data.validate().ok()?;
        Some(data)
    }
}

/// What to load for one tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileRequest {
    pub tile: u16,
    pub satellite: VfsPath,
    pub mask: Option<VfsPath>,
}

impl TileFormat {
    /// Convert any square PAA to this format, as a texture array layer of detail textures:
    /// the chain starting at the mip whose edge is `size`, or, for a smaller texture, its
    /// largest mip scaled up (nearest) and re-encoded. `None` when it is unreadable.
    pub fn decode_scaled(&self, bytes: &[u8]) -> Option<TextureData> {
        let header = PaaHeader::read(bytes).ok()?;
        let start = header
            .mips
            .iter()
            .position(|m| u32::from(m.width) == self.size && u32::from(m.height) == self.size);
        if let Some(start) = start.filter(|s| header.mips.len() >= s + self.mips as usize) {
            let mut mips = Vec::with_capacity(self.mips as usize);
            for index in start..start + self.mips as usize {
                let mip = header.read_mip(bytes, index).ok()?;
                mips.push(match self.format {
                    TextureFormat::Bc1 | TextureFormat::Bc3 if self.native(header.meta.format) => {
                        mip.data
                    }
                    _ => rgba(&header, &mip)?,
                });
            }
            let data = TextureData {
                format: if self.native(header.meta.format) {
                    self.format
                } else {
                    TextureFormat::Rgba8
                },
                width: self.size,
                height: self.size,
                mips,
            };
            return self.convert(data);
        }
        // Smaller than the array (or an odd chain): scale the largest mip up.
        let first = header.read_mip(bytes, 0).ok()?;
        let (w, h) = (u32::from(first.width), u32::from(first.height));
        let src = rgba(&header, &first)?;
        let n = self.size;
        let mut scaled = vec![0u8; (n * n * 4) as usize];
        for y in 0..n {
            for x in 0..n {
                let (sx, sy) = (x * w / n, y * h / n);
                let s = ((sy * w + sx) * 4) as usize;
                let d = ((y * n + x) * 4) as usize;
                scaled[d..d + 4].copy_from_slice(&src[s..s + 4]);
            }
        }
        let rgba8 = TextureData {
            format: TextureFormat::Rgba8,
            width: n,
            height: n,
            mips: rgba_mips(n, scaled, self.mips),
        };
        self.convert(rgba8)
    }

    /// A layer filled with one RGBA colour.
    pub fn solid(&self, rgba: [u8; 4]) -> Option<TextureData> {
        let n = self.size;
        let level0 = rgba.repeat((n * n) as usize);
        self.convert(TextureData {
            format: TextureFormat::Rgba8,
            width: n,
            height: n,
            mips: rgba_mips(n, level0, self.mips),
        })
    }

    /// Whether PAA data in `format` uploads as-is to this format.
    fn native(&self, format: PixelFormat) -> bool {
        matches!(
            (self.format, format),
            (TextureFormat::Bc1, PixelFormat::Dxt1) | (TextureFormat::Bc3, PixelFormat::Dxt5)
        )
    }

    /// Bring RGBA8 data into this format (re-encoding to DXT when the array is BC).
    fn convert(&self, data: TextureData) -> Option<TextureData> {
        if data.format == self.format {
            return data.validate().ok().map(|_| data);
        }
        let target = match self.format {
            TextureFormat::Bc1 => PixelFormat::Dxt1,
            TextureFormat::Bc3 => PixelFormat::Dxt5,
            _ => return None,
        };
        let options = a3_paa::EncodeOptions {
            format: target,
            mipmaps: true,
            compress: false,
        };
        let encoded =
            a3_paa::encode_rgba8(self.size as u16, self.size as u16, &data.mips[0], &options)
                .ok()?;
        let out = TextureData {
            format: self.format,
            width: self.size,
            height: self.size,
            mips: encoded
                .mips
                .into_iter()
                .take(self.mips as usize)
                .map(|m| m.data)
                .collect(),
        };
        (out.mips.len() == self.mips as usize && out.validate().is_ok()).then_some(out)
    }
}

fn rgba(header: &PaaHeader, mip: &a3_paa::Mip) -> Option<Vec<u8>> {
    let mut rgba = a3_paa::decode_rgba8(header.meta.format, mip).ok()?;
    if let Some(swizzle) = header.meta.swizzle {
        swizzle.restore(&mut rgba);
    }
    Some(rgba)
}

/// `count` box-filtered mips of a square RGBA8 image.
fn rgba_mips(size: u32, level0: Vec<u8>, count: u32) -> Vec<Vec<u8>> {
    let mut mips = vec![level0];
    let mut n = size;
    while (mips.len() as u32) < count && n > 1 {
        let half = n / 2;
        let prev = mips.last().expect("level 0");
        let mut next = vec![0u8; (half * half * 4) as usize];
        for y in 0..half {
            for x in 0..half {
                for c in 0..4 {
                    let at = |dx: u32, dy: u32| {
                        u32::from(prev[(((2 * y + dy) * n + 2 * x + dx) * 4 + c) as usize])
                    };
                    next[((y * half + x) * 4 + c) as usize] =
                        ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                }
            }
        }
        mips.push(next);
        n = half;
    }
    mips
}

/// A loaded tile: its index, satellite texture (`None` when it could not be read) and mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub tile: u16,
    pub satellite: Option<TextureData>,
    pub mask: Option<TextureData>,
}

/// Loader threads fed with tile requests.
pub struct TileLoader {
    requests: Option<Sender<TileRequest>>,
    results: Receiver<Loaded>,
}

impl TileLoader {
    /// Threads converting satellite tiles to `format` and masks to `mask_format` (masks are
    /// skipped without one).
    pub fn spawn(
        reader: FileReader,
        format: TileFormat,
        mask_format: Option<TileFormat>,
        threads: usize,
    ) -> TileLoader {
        let (request_tx, request_rx) = mpsc::channel::<TileRequest>();
        let (result_tx, result_rx) = mpsc::channel();
        let request_rx = Arc::new(Mutex::new(request_rx));
        for n in 0..threads.max(1) {
            let (requests, results, reader) =
                (request_rx.clone(), result_tx.clone(), reader.clone());
            let spawned = std::thread::Builder::new()
                .name(format!("tile loader {n}"))
                .spawn(move || {
                    loop {
                        let next = requests.lock().map(|r| r.recv());
                        let Ok(Ok(request)) = next else { break };
                        // A panic while decoding must not leave the tile pending forever.
                        let load = std::panic::AssertUnwindSafe(|| {
                            let satellite =
                                reader(&request.satellite).and_then(|b| format.decode(&b));
                            let mask = match (&request.mask, mask_format, &satellite) {
                                (Some(path), Some(f), Some(_)) => {
                                    reader(path).and_then(|b| f.decode(&b))
                                }
                                _ => None,
                            };
                            (satellite, mask)
                        });
                        let (satellite, mask) =
                            std::panic::catch_unwind(load).unwrap_or_else(|_| {
                                log::error!("loading tile {} panicked", request.satellite);
                                (None, None)
                            });
                        if satellite.is_none() {
                            // Placeholder tiles (4x4 px) do not fit the array; they keep
                            // their overview colour.
                            log::debug!("satellite tile {} does not stream", request.satellite);
                        }
                        let loaded = Loaded {
                            tile: request.tile,
                            satellite,
                            mask,
                        };
                        if results.send(loaded).is_err() {
                            break;
                        }
                    }
                });
            if let Err(e) = spawned {
                log::error!("cannot start a tile loader thread: {e}");
            }
        }
        TileLoader {
            requests: Some(request_tx),
            results: result_rx,
        }
    }

    pub fn request(&self, request: TileRequest) {
        if let Some(tx) = &self.requests {
            let _ = tx.send(request);
        }
    }

    /// A finished tile, if any.
    pub fn try_recv(&self) -> Option<Loaded> {
        self.results.try_recv().ok()
    }
}

impl Drop for TileLoader {
    fn drop(&mut self) {
        // Closing the request channel ends the threads after their current tile.
        self.requests.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_paa::{EncodeOptions, Texture, encode_rgba8};

    fn paa(format: PixelFormat, px: u16) -> Vec<u8> {
        let pixels: Vec<u8> = (0..u32::from(px) * u32::from(px))
            .flat_map(|i| [(i % 256) as u8, 128, 0, 255])
            .collect();
        encode_rgba8(px, px, &pixels, &EncodeOptions::new(format))
            .unwrap()
            .to_bytes()
            .unwrap()
    }

    #[test]
    fn dxt1_tiles_stay_block_compressed_when_the_gpu_can_sample_bc() {
        let bytes = paa(PixelFormat::Dxt1, 32);
        let format = TileFormat::probe(&bytes, true).unwrap();
        assert_eq!(
            format,
            TileFormat {
                format: TextureFormat::Bc1,
                size: 32,
                mips: 4
            },
            "32, 16, 8, 4"
        );
        let data = format.decode(&bytes).unwrap();
        assert_eq!(data.format, TextureFormat::Bc1);
        assert_eq!(data.mips.len(), 4);
        assert_eq!(data.mips[0].len(), 8 * 8 * 8);
    }

    #[test]
    fn tiles_are_decoded_to_rgba8_without_bc_support() {
        let bytes = paa(PixelFormat::Dxt1, 16);
        let format = TileFormat::probe(&bytes, false).unwrap();
        assert_eq!(format.format, TextureFormat::Rgba8);
        let data = format.decode(&bytes).unwrap();
        assert_eq!(data.format, TextureFormat::Rgba8);
        assert_eq!(data.mips[0].len(), 16 * 16 * 4);
    }

    #[test]
    fn dxt5_masks_stay_bc3() {
        let bytes = paa(PixelFormat::Dxt5, 16);
        let format = TileFormat::probe(&bytes, true).unwrap();
        assert_eq!(format.format, TextureFormat::Bc3);
        let data = format.decode(&bytes).unwrap();
        assert_eq!(data.mips[0].len(), 4 * 4 * 16);
    }

    #[test]
    fn detail_textures_start_at_the_array_size() {
        let format = TileFormat {
            format: TextureFormat::Bc1,
            size: 16,
            mips: 3,
        };
        // 32 px source: mips 16, 8, 4 are taken as they are.
        let bytes = paa(PixelFormat::Dxt1, 32);
        let data = format.decode_scaled(&bytes).unwrap();
        assert_eq!(
            (data.format, data.width, data.mips.len()),
            (TextureFormat::Bc1, 16, 3)
        );
        let source = Texture::read(&bytes).unwrap();
        assert_eq!(data.mips[0], source.mips[1].data);
    }

    #[test]
    fn smaller_detail_textures_are_scaled_up_and_reencoded() {
        let format = TileFormat {
            format: TextureFormat::Bc3,
            size: 16,
            mips: 3,
        };
        let data = format.decode_scaled(&paa(PixelFormat::Dxt5, 8)).unwrap();
        assert_eq!(
            (data.format, data.width, data.mips.len()),
            (TextureFormat::Bc3, 16, 3)
        );
        assert_eq!(data.validate(), Ok(()));
        let rgba = TileFormat {
            format: TextureFormat::Rgba8,
            ..format
        };
        let data = rgba.decode_scaled(&paa(PixelFormat::Dxt1, 8)).unwrap();
        assert_eq!(
            (data.format, data.mips[0].len()),
            (TextureFormat::Rgba8, 16 * 16 * 4)
        );
    }

    #[test]
    fn tiles_of_another_shape_are_rejected() {
        let format = TileFormat::probe(&paa(PixelFormat::Dxt1, 32), true).unwrap();
        assert_eq!(format.decode(&paa(PixelFormat::Dxt1, 16)), None);
        assert_eq!(format.decode(&paa(PixelFormat::Dxt5, 32)), None);
        assert_eq!(format.decode(b"not a paa"), None);
    }

    #[test]
    fn the_loader_returns_requested_tiles() {
        let bytes = paa(PixelFormat::Dxt1, 16);
        let format = TileFormat::probe(&bytes, true).unwrap();
        let mask = paa(PixelFormat::Dxt5, 16);
        let mask_format = TileFormat::probe(&mask, true).unwrap();
        let reader: FileReader = Arc::new(move |p: &VfsPath| match p.as_str() {
            "a.paa" => Some(bytes.clone()),
            "m.paa" => Some(mask.clone()),
            _ => None,
        });
        let loader = TileLoader::spawn(reader, format, Some(mask_format), 2);
        let request = |tile, satellite: &str, mask: Option<&str>| TileRequest {
            tile,
            satellite: VfsPath::new(satellite),
            mask: mask.map(VfsPath::new),
        };
        loader.request(request(3, "a.paa", Some("m.paa")));
        loader.request(request(4, "missing.paa", Some("m.paa")));
        loader.request(request(5, "a.paa", None));
        let mut got = Vec::new();
        let start = std::time::Instant::now();
        while got.len() < 3 && start.elapsed().as_secs() < 10 {
            match loader.try_recv() {
                Some(l) => got.push((l.tile, l.satellite.is_some(), l.mask.is_some())),
                None => std::thread::yield_now(),
            }
        }
        got.sort();
        assert_eq!(
            got,
            vec![(3, true, true), (4, false, false), (5, true, false)]
        );
    }
}
