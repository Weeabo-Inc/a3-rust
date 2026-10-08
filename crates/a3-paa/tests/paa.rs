//! Reading and writing PAA files built byte by byte in the test.

use a3_paa::{Color, PixelFormat, Texture};

/// Appends a TAGG: `GGAT` + reversed four-character name + `u32` length + data.
fn tagg(out: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(b"GGAT");
    out.extend_from_slice(name);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
}

fn mip_header(out: &mut Vec<u8>, width: u16, height: u16, size: u32) {
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes()[..3]);
}

/// A 4x4 DXT1 block: colour0 = red (0xF800), colour1 = blue (0x001F), all texels index 0.
const RED_BLOCK: [u8; 8] = [0x00, 0xF8, 0x1F, 0x00, 0, 0, 0, 0];

/// A two-mip (8x8, 4x4) DXT1 PAA shaped like the shipped `a3\data_f\default.pac`.
fn dxt1_two_mips() -> Vec<u8> {
    let mut f = vec![0x01, 0xFF];
    tagg(&mut f, b"CGVA", &[0x10, 0x20, 0x30, 0xFF]);
    tagg(&mut f, b"CXAM", &[0xFF, 0xFF, 0xFF, 0xFF]);
    let mut offs = [0u8; 64];
    let first = (f.len() + 8 + 4 + 64 + 2) as u32;
    let second = first + 7 + 32;
    offs[..4].copy_from_slice(&first.to_le_bytes());
    offs[4..8].copy_from_slice(&second.to_le_bytes());
    tagg(&mut f, b"SFFO", &offs);
    f.extend_from_slice(&0u16.to_le_bytes()); // palette: no colours
    mip_header(&mut f, 8, 8, 32);
    for _ in 0..4 {
        f.extend_from_slice(&RED_BLOCK);
    }
    mip_header(&mut f, 4, 4, 8);
    f.extend_from_slice(&RED_BLOCK);
    f.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // terminator
    f
}

#[test]
fn reads_an_uncompressed_dxt1_mip_chain_with_its_taggs() {
    let texture = Texture::read(&dxt1_two_mips()).unwrap();

    assert_eq!(texture.format, PixelFormat::Dxt1);
    assert_eq!((texture.width(), texture.height()), (8, 8));
    assert_eq!(texture.mips.len(), 2);
    assert_eq!((texture.mips[1].width, texture.mips[1].height), (4, 4));
    assert_eq!(texture.mips[1].data, RED_BLOCK);
    assert_eq!(texture.mips[0].data.len(), 32);
    // Colours are stored as B, G, R, A bytes.
    assert_eq!(
        texture.average_color,
        Some(Color {
            r: 0x30,
            g: 0x20,
            b: 0x10,
            a: 0xFF
        })
    );
    assert_eq!(texture.max_color, Some(Color::WHITE));
}

#[test]
fn writing_a_read_texture_reproduces_the_file() {
    let file = dxt1_two_mips();
    let texture = Texture::read(&file).unwrap();
    assert_eq!(texture.to_bytes().unwrap(), file);
}

#[test]
fn compressed_mips_round_trip_through_lzo_and_lzss() {
    use a3_paa::{AlphaFlags, Compression, Mip, Swizzle};

    let gradient = |len: usize| (0..len).map(|i| (i / 7) as u8).collect::<Vec<u8>>();
    for (format, compression) in [
        (PixelFormat::Dxt5, Compression::Lzo),
        (PixelFormat::Dxt1, Compression::Lzo),
        (PixelFormat::Argb4444, Compression::Lzss),
        (PixelFormat::Ai88, Compression::Lzss),
        (PixelFormat::Argb8888, Compression::None),
    ] {
        let mips = [(64u16, 32u16), (32, 16), (16, 8)]
            .into_iter()
            .map(|(width, height)| Mip {
                width,
                height,
                data: gradient(format.data_len(width, height)),
                compression,
            })
            .collect();
        let texture = Texture {
            format,
            average_color: Some(Color::rgba(1, 2, 3, 4)),
            max_color: Some(Color::WHITE),
            flags: Some(AlphaFlags(AlphaFlags::INTERPOLATED)),
            swizzle: Some(Swizzle::from_bytes([5, 4, 2, 3])),
            mips,
            ..Texture::default()
        };
        let bytes = texture.to_bytes().unwrap();
        assert_eq!(Texture::read(&bytes).unwrap(), texture, "{format:?}");
    }
}

mod round_trip {
    use a3_paa::{AlphaFlags, Color, Compression, Mip, PixelFormat, Swizzle, Texture};
    use proptest::prelude::*;

    fn format() -> impl Strategy<Value = PixelFormat> {
        proptest::sample::select(PixelFormat::ALL.to_vec())
    }

    prop_compose! {
        fn texture()(
            format in format(),
            width in 1u16..40,
            height in 1u16..40,
            levels in 1usize..4,
            compress in any::<bool>(),
            seed in any::<u64>(),
            avg in proptest::option::of(any::<[u8; 4]>()),
            flags in proptest::option::of(0u32..4),
            swizzle in proptest::option::of(any::<[u8; 4]>()),
        ) -> Texture {
            let compression = match (compress, format.is_dxt()) {
                (false, _) => Compression::None,
                (true, true) => Compression::Lzo,
                (true, false) => Compression::Lzss,
            };
            let mut state = seed;
            let mut next = move || {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                // Low-entropy bytes so compression usually pays off.
                ((state >> 60) as u8) * 17
            };
            let (mut w, mut h) = (width, height);
            let mut mips = Vec::new();
            for _ in 0..levels {
                let data = (0..format.data_len(w, h)).map(|_| next()).collect();
                mips.push(Mip { width: w, height: h, data, compression });
                (w, h) = ((w / 2).max(1), (h / 2).max(1));
            }
            Texture {
                format,
                average_color: avg.map(Color::from_bgra),
                max_color: Some(Color::WHITE),
                flags: flags.map(AlphaFlags),
                swizzle: swizzle.map(Swizzle::from_bytes),
                mips,
                ..Texture::default()
            }
        }
    }

    proptest! {
        #[test]
        fn read_returns_what_was_written(texture in texture()) {
            let bytes = texture.to_bytes().unwrap();
            let mut read = Texture::read(&bytes).unwrap();
            // LZSS output that happens to equal the raw size is stored raw.
            for (r, w) in read.mips.iter_mut().zip(&texture.mips) {
                if r.compression == Compression::None {
                    r.compression = w.compression;
                }
            }
            prop_assert_eq!(read, texture);
        }
    }
}
