//! Decodes compressed data found in the user's Arma 3 install. Skipped when `A3_ROOT` is unset.
//!
//! Shipped PBOs contain no `Cprs` entries, so PAA mipmaps are the LZSS and LZO samples. ODOL and
//! WRP blocks are left to the readers of those formats, which know where the arrays are.

mod common;

use a3_compress::lzss::{self, ChecksumKind};
use a3_compress::{Codec, lzo, read_compressed_block};
use common::{
    Mip, first_mip, first_mip_header, game_pbos, pbo_entries, read_entry, read_entry_prefix,
};

/// How many LZO mipmaps to decode; the install has tens of thousands.
const LZO_SAMPLE: usize = 300;

/// Calls `f` with the first mipmap of every PAA whose header passes `want(tag, width)`.
fn for_each_paa_mip(want: impl Fn(u16, u16) -> bool, mut f: impl FnMut(&str, &Mip<'_>) -> bool) {
    let Some(pbos) = game_pbos() else {
        return;
    };
    for pbo in pbos {
        let Ok(entries) = pbo_entries(&pbo) else {
            eprintln!("cannot walk {}", pbo.display());
            continue;
        };
        for e in entries.iter().filter(|e| e.has_extension("paa")) {
            let name = format!("{}:{}", pbo.display(), e.name);
            let head = read_entry_prefix(&pbo, e, 4096);
            let Some((tag, width, ..)) = first_mip_header(&head) else {
                panic!("{name}: PAA header longer than 4 KiB");
            };
            if !want(tag, width) {
                continue;
            }
            let paa = read_entry(&pbo, e);
            let mip = first_mip(&paa).unwrap_or_else(|| panic!("{name}: truncated mipmap"));
            if !f(&name, &mip) {
                return;
            }
        }
    }
}

#[test]
fn non_dxt_paa_mipmaps_are_lzss_with_signed_checksum() {
    let mut decoded = 0;
    for_each_paa_mip(
        |tag, _| matches!(tag, 0x4444 | 0x1555 | 0x8080),
        |name, mip| {
            let expected = usize::from(mip.width) * usize::from(mip.height) * 2;

            let (out, consumed) = lzss::decompress(mip.data, expected, ChecksumKind::Signed)
                .unwrap_or_else(|e| panic!("{name}: {e}"));

            assert_eq!(consumed, mip.data.len(), "{name}");
            let repacked = lzss::compress(&out, ChecksumKind::Signed);
            let (again, _) = lzss::decompress(&repacked, expected, ChecksumKind::Signed).unwrap();
            assert_eq!(again, out, "{name}: re-encode round trip");
            decoded += 1;
            true
        },
    );
    eprintln!("decoded {decoded} LZSS mipmaps");
}

#[test]
fn dxt_paa_mipmaps_flagged_in_width_are_lzo() {
    let mut decoded = 0;
    for_each_paa_mip(
        |tag, width| matches!(tag, 0xFF01 | 0xFF05) && width & 0x8000 != 0,
        |name, mip| {
            let block_bytes = if mip.tag == 0xFF01 { 8 } else { 16 };
            let width = usize::from(mip.width & 0x7FFF);
            let height = usize::from(mip.height);
            let expected = width.div_ceil(4) * height.div_ceil(4) * block_bytes;

            let (out, consumed) =
                lzo::decompress(mip.data, expected).unwrap_or_else(|e| panic!("{name}: {e}"));

            assert_eq!(out.len(), expected, "{name}");
            assert_eq!(consumed, mip.data.len(), "{name}");
            // The stream API stops at the same place.
            let mut r = mip.data;
            let streamed = read_compressed_block(&mut r, expected, Codec::Lzo).unwrap();
            assert_eq!(streamed, out, "{name}");
            assert!(r.is_empty(), "{name}");
            decoded += 1;
            decoded < LZO_SAMPLE
        },
    );
    eprintln!("decoded {decoded} LZO mipmaps");
}
