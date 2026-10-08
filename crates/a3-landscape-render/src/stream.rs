//! Background loading of full-resolution satellite tiles.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use a3_core::VfsPath;
use a3_paa::{PaaHeader, PixelFormat};
use a3_render::{TextureData, TextureFormat};

/// Reads a VFS file; shared with the loader threads.
pub type FileReader = Arc<dyn Fn(&VfsPath) -> Option<Vec<u8>> + Send + Sync>;

/// The shape every streamed tile is converted to: one layer of the tile texture array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileFormat {
    /// Keep DXT1 blocks (BC1) instead of decoding to RGBA8.
    pub bc1: bool,
    /// Edge of mip 0 in pixels.
    pub size: u32,
    /// Mip levels, largest first.
    pub mips: u32,
}

impl TileFormat {
    /// The format for tiles shaped like the PAA in `bytes`. DXT1 tiles stay BC1 when the GPU
    /// samples BC (`bc_supported`); everything else is decoded to RGBA8.
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
            bc1: bc_supported && header.meta.format == PixelFormat::Dxt1,
            size,
            mips,
        })
    }

    /// Convert one tile PAA. `None` when it does not match this format.
    pub fn decode(&self, bytes: &[u8]) -> Option<TextureData> {
        let header = PaaHeader::read(bytes).ok()?;
        if header.mips.len() < self.mips as usize
            || header.mips.first().map(|m| u32::from(m.width)) != Some(self.size)
            || (self.bc1 && header.meta.format != PixelFormat::Dxt1)
        {
            return None;
        }
        let mut mips = Vec::with_capacity(self.mips as usize);
        for index in 0..self.mips as usize {
            let mip = header.read_mip(bytes, index).ok()?;
            if u32::from(mip.width) != self.size >> index {
                return None;
            }
            if self.bc1 {
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
            format: if self.bc1 {
                TextureFormat::Bc1
            } else {
                TextureFormat::Rgba8
            },
            width: self.size,
            height: self.size,
            mips,
        };
        data.validate().ok()?;
        Some(data)
    }
}

/// A loaded tile: its index and texture, `None` when it could not be read.
pub type Loaded = (u16, Option<TextureData>);

/// Loader threads fed with tile requests.
pub struct TileLoader {
    requests: Option<Sender<(u16, VfsPath)>>,
    results: Receiver<Loaded>,
}

impl TileLoader {
    pub fn spawn(reader: FileReader, format: TileFormat, threads: usize) -> TileLoader {
        let (request_tx, request_rx) = mpsc::channel::<(u16, VfsPath)>();
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
                        let Ok(Ok((tile, path))) = next else { break };
                        let data = reader(&path).and_then(|b| format.decode(&b));
                        if data.is_none() {
                            // Placeholder tiles (4x4 px) do not fit the array; they keep
                            // their overview colour.
                            log::debug!("satellite tile {path} does not stream");
                        }
                        if results.send((tile, data)).is_err() {
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

    pub fn request(&self, tile: u16, path: VfsPath) {
        if let Some(tx) = &self.requests {
            let _ = tx.send((tile, path));
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
    use a3_paa::{EncodeOptions, encode_rgba8};

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
                bc1: true,
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
        assert!(!format.bc1);
        let data = format.decode(&bytes).unwrap();
        assert_eq!(data.format, TextureFormat::Rgba8);
        assert_eq!(data.mips[0].len(), 16 * 16 * 4);
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
        let reader: FileReader =
            Arc::new(move |p: &VfsPath| (p.as_str() == "a.paa").then(|| bytes.clone()));
        let loader = TileLoader::spawn(reader, format, 2);
        loader.request(3, VfsPath::new("a.paa"));
        loader.request(4, VfsPath::new("missing.paa"));
        let mut got = Vec::new();
        let start = std::time::Instant::now();
        while got.len() < 2 && start.elapsed().as_secs() < 10 {
            match loader.try_recv() {
                Some((tile, data)) => got.push((tile, data.is_some())),
                None => std::thread::yield_now(),
            }
        }
        got.sort();
        assert_eq!(got, vec![(3, true), (4, false)]);
    }
}
