use std::io::Read;

use a3_compress::lzss::{self, ChecksumKind};
use a3_compress::{
    COMPRESSED_ARRAY_THRESHOLD, Codec, read_compressed_array, read_compressed_block,
};

fn sample(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 7 * 31) as u8).collect()
}

/// Compressed block followed by a marker the caller reads next.
fn stream(block: &[u8]) -> Vec<u8> {
    let mut s = block.to_vec();
    s.extend(b"NEXT");
    s
}

fn rest(mut r: impl Read) -> Vec<u8> {
    let mut v = Vec::new();
    r.read_to_end(&mut v).unwrap();
    v
}

#[test]
fn lzss_block_is_read_inline() {
    let data = sample(3000);
    let bytes = stream(&lzss::compress(&data, ChecksumKind::Signed));
    let mut r = &bytes[..];

    let out = read_compressed_block(&mut r, data.len(), Codec::Lzss(ChecksumKind::Signed)).unwrap();

    assert_eq!(out, data);
    assert_eq!(rest(r), b"NEXT");
}

#[test]
fn lzo_block_is_read_inline() {
    let data = sample(3000);
    let bytes = stream(&lzokay_native::compress(&data).unwrap());
    let mut r = &bytes[..];

    let out = read_compressed_block(&mut r, data.len(), Codec::Lzo).unwrap();

    assert_eq!(out, data);
    assert_eq!(rest(r), b"NEXT");
}

#[test]
fn lz4_block_reads_its_stored_compressed_length() {
    let data = sample(3000);
    let packed = lz4_flex::block::compress(&data);
    let bytes = stream(&packed);
    let mut r = &bytes[..];

    let out = read_compressed_block(
        &mut r,
        data.len(),
        Codec::Lz4 {
            compressed_len: packed.len(),
        },
    )
    .unwrap();

    assert_eq!(out, data);
    assert_eq!(rest(r), b"NEXT");
}

#[test]
fn array_below_threshold_is_stored_raw() {
    let data = sample(COMPRESSED_ARRAY_THRESHOLD - 1);
    let bytes = stream(&data);
    let mut r = &bytes[..];

    let out = read_compressed_array(&mut r, data.len(), Codec::Lzo).unwrap();

    assert_eq!(out, data);
    assert_eq!(rest(r), b"NEXT");
}

#[test]
fn array_at_threshold_is_compressed() {
    let data = sample(COMPRESSED_ARRAY_THRESHOLD);
    let bytes = stream(&lzss::compress(&data, ChecksumKind::Signed));
    let mut r = &bytes[..];

    let out = read_compressed_array(&mut r, data.len(), Codec::Lzss(ChecksumKind::Signed)).unwrap();

    assert_eq!(out, data);
    assert_eq!(rest(r), b"NEXT");
}
