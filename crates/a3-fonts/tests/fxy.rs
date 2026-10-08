//! FXY files built byte by byte.

use a3_fonts::{Error, Font, FxyVersion, Glyph, page_texture_path};

fn u16s(out: &mut Vec<u8>, values: &[u16]) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn i32s(out: &mut Vec<u8>, values: &[i32]) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// Version 0x102: two page blocks; the first has one kerning pair.
fn bifo_102() -> Vec<u8> {
    let mut out = b"BIFo".to_vec();
    i32s(&mut out, &[0x102]);
    // Page 1: metrics 17/13, one kerning pair (A, V, -2), two glyphs.
    u16s(&mut out, &[1]);
    i32s(&mut out, &[17, 13, 1]);
    u16s(&mut out, &[0x41, 0x56]);
    i32s(&mut out, &[-2, 2]);
    u16s(&mut out, &[0x20, 2, 2, 0, 0]);
    i32s(&mut out, &[-10, -10, 3]);
    u16s(&mut out, &[0x41, 6, 2, 9, 12]);
    i32s(&mut out, &[0, 3, 8]);
    // Page 2: no kerning, one Cyrillic glyph.
    u16s(&mut out, &[2]);
    i32s(&mut out, &[17, 13, 0, 1]);
    u16s(&mut out, &[0x416, 0, 0, 11, 12]);
    i32s(&mut out, &[1, 3, 10]);
    out
}

#[test]
fn reads_pages_kerning_and_glyphs_of_version_0x102() {
    let font = Font::read(&bifo_102()).unwrap();
    assert_eq!(font.version, FxyVersion::V102);
    let pages: Vec<(u16, i32, i32)> = font
        .pages
        .iter()
        .map(|p| (p.index, p.height, p.ascent))
        .collect();
    assert_eq!(pages, [(1, 17, 13), (2, 17, 13)]);
    assert_eq!(font.glyphs.len(), 3);
    assert_eq!(
        font.glyph('A'),
        Some(&Glyph {
            code: 0x41,
            page: 1,
            x: 6,
            y: 2,
            width: 9,
            height: 12,
            offset_x: 0,
            offset_y: 3,
            advance: 8,
        })
    );
    assert_eq!(font.glyph('Ж').map(|g| g.page), Some(2));
    assert_eq!(font.glyph('Z'), None);
    assert_eq!(font.kerning('A', 'V'), -2);
    assert_eq!(font.kerning('V', 'A'), 0);
}

#[test]
fn reads_version_0x101_records_with_an_advance() {
    let mut out = b"BIFo".to_vec();
    i32s(&mut out, &[0x101]);
    // code - 0x20, page, x, y, w, h, advance
    u16s(&mut out, &[0x21, 1, 10, 20, 7, 13, 6]);
    let font = Font::read(&out).unwrap();
    assert_eq!(font.version, FxyVersion::V101);
    let g = font.glyph('A').unwrap();
    assert_eq!(
        (g.page, g.x, g.y, g.width, g.height, g.advance),
        (1, 10, 20, 7, 13, 6)
    );
    assert_eq!((g.offset_x, g.offset_y), (0, 0));
    assert_eq!(font.pages.iter().map(|p| p.index).collect::<Vec<_>>(), [1]);
}

#[test]
fn reads_unversioned_records_whose_advance_is_the_width() {
    let mut out = Vec::new();
    u16s(&mut out, &[0, 1, 0, 0, 1, 1]);
    u16s(&mut out, &[1, 1, 4, 0, 7, 13]);
    let font = Font::read(&out).unwrap();
    assert_eq!(font.version, FxyVersion::Legacy);
    assert_eq!(font.glyph(' ').unwrap().advance, 1);
    let bang = font.glyph('!').unwrap();
    assert_eq!((bang.x, bang.width, bang.advance), (4, 7, 7));
}

#[test]
fn rejects_truncated_and_newer_files() {
    let bytes = bifo_102();
    // Page blocks run to the end of the file, so a file cut after page 1 (78 bytes) is valid.
    assert_eq!(Font::read(&bytes[..78]).unwrap().pages.len(), 1);
    for len in (9..bytes.len()).filter(|&len| len != 78) {
        assert!(Font::read(&bytes[..len]).is_err(), "length {len}");
    }
    let mut newer = bytes.clone();
    newer[4] = 0x03;
    assert!(matches!(
        Font::read(&newer),
        Err(Error::UnsupportedVersion(0x103))
    ));
}

#[test]
fn page_textures_are_numbered_from_one_with_two_digits() {
    assert_eq!(
        page_texture_path(r"a3\uifonts_f\data\fonts\caveat\caveat10.fxy", 3),
        r"a3\uifonts_f\data\fonts\caveat\caveat10-03.paa"
    );
    assert_eq!(page_texture_path(r"fonts\x", 12), r"fonts\x-12.paa");
}
