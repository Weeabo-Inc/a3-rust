//! Reading and writing texHeaders.bin built byte by byte in the test.

use a3_paa::{Color, PixelFormat, TexHeaders, TextureType};

/// One entry shaped like the first one of the shipped `a3\characters_f\texheaders.bin`:
/// a 2048x2048 DXT1 `_co` with two (of its ten) mipmaps listed.
fn entry(path: &str) -> Vec<u8> {
    let mut e = Vec::new();
    e.extend_from_slice(&1u32.to_le_bytes()); // palette count
    e.extend_from_slice(&0u32.to_le_bytes()); // palette pointer
    for v in [0.25f32, 0.5, 0.75, 1.0] {
        e.extend_from_slice(&v.to_le_bytes());
    }
    e.extend_from_slice(&[0, 0, 0, 0]); // average BGRA
    e.extend_from_slice(&[0xFF; 4]); // max BGRA
    e.extend_from_slice(&0u32.to_le_bytes()); // clamp flags
    e.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // transparent colour
    e.extend_from_slice(&[1, 0, 0, 0]); // MAXC present, opaque
    e.extend_from_slice(&2u32.to_le_bytes()); // mip count
    e.extend_from_slice(&6u32.to_le_bytes()); // DXT1
    e.extend_from_slice(&[1, 1]); // little endian, .paa
    e.extend_from_slice(path.as_bytes());
    e.push(0);
    e.extend_from_slice(&0u32.to_le_bytes()); // texture type: diffuse
    e.extend_from_slice(&2u32.to_le_bytes()); // mip count again
    for (size, offset) in [(2048u16, 0x70u32), (1024, 0x18_4AC3)] {
        e.extend_from_slice(&size.to_le_bytes());
        e.extend_from_slice(&size.to_le_bytes());
        e.extend_from_slice(&[0, 0, 6, 3]);
        e.extend_from_slice(&offset.to_le_bytes());
    }
    e.extend_from_slice(&0x0020_210Eu32.to_le_bytes()); // file size
    e
}

fn file(entries: &[Vec<u8>]) -> Vec<u8> {
    let mut f = b"0DHT".to_vec();
    f.extend_from_slice(&1u32.to_le_bytes());
    f.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in entries {
        f.extend_from_slice(e);
    }
    f
}

#[test]
fn reads_every_field_of_an_entry() {
    let data = file(&[
        entry(r"common\data\diver_suit_nato_co.paa"),
        entry(r"data\other_ca.paa"),
    ]);
    let headers = TexHeaders::read(&data).unwrap();

    assert_eq!(headers.version, 1);
    assert_eq!(headers.textures.len(), 2);
    let t = &headers.textures[0];
    assert_eq!(t.path, r"common\data\diver_suit_nato_co.paa");
    assert_eq!(t.format, Some(PixelFormat::Dxt1));
    assert_eq!(t.average, [0.25, 0.5, 0.75, 1.0]);
    assert_eq!(t.max_color, Color::WHITE);
    assert!(t.has_max_color && !t.is_alpha && !t.is_transparent && t.is_paa);
    assert_eq!(t.texture_type(), Some(TextureType::Diffuse));
    assert_eq!(t.mips.len(), 2);
    assert_eq!((t.mips[1].width, t.mips[1].height), (1024, 1024));
    assert_eq!(t.mips[1].offset, 0x18_4AC3);
    assert_eq!(t.file_size, 0x0020_210E);

    let found = headers.find("COMMON/data/Diver_Suit_NATO_co.paa").unwrap();
    assert_eq!(found.path, t.path);
}

#[test]
fn writes_back_identical_bytes() {
    let data = file(&[entry(r"a_co.paa"), entry(r"b\c_nohq.paa")]);
    assert_eq!(TexHeaders::read(&data).unwrap().to_bytes(), data);
}

#[test]
fn rejects_a_truncated_file() {
    let mut data = file(&[entry(r"a_co.paa")]);
    data.truncate(data.len() - 3);
    assert!(TexHeaders::read(&data).is_err());
    assert!(TexHeaders::read(b"0DHX\x01\0\0\0\0\0\0\0").is_err());
}
