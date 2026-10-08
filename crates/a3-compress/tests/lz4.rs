use a3_compress::{Error, lz4};

#[test]
fn literal_only_block_decodes() {
    // One sequence: token 0x50 = 5 literals, no match (last sequence).
    let input = [0x50, b'h', b'e', b'l', b'l', b'o'];

    let out = lz4::decompress(&input, 5).unwrap();

    assert_eq!(out, b"hello");
}

#[test]
fn block_with_match_decodes() {
    // "abcd" + match (offset 4, length 4+4=8) + 5 final literals "WXYZ!".
    let input = [
        0x44, b'a', b'b', b'c', b'd', 0x04, 0x00, // 4 literals, match len 4 + 4
        0x50, b'W', b'X', b'Y', b'Z', b'!',
    ];

    let out = lz4::decompress(&input, 17).unwrap();

    assert_eq!(out, b"abcdabcdabcdWXYZ!");
}

#[test]
fn wrong_expected_length_is_an_error() {
    let input = [0x50, b'h', b'e', b'l', b'l', b'o'];

    assert!(lz4::decompress(&input, 4).is_err());
    assert!(matches!(
        lz4::decompress(&input, 6).unwrap_err(),
        Error::OutputUnderrun {
            produced: 5,
            expected: 6
        }
    ));
}

#[test]
fn round_trips_through_lz4_flex_encoder() {
    let data = b"the quick brown fox jumps over the lazy dog ".repeat(50);
    let packed = lz4_flex::block::compress(&data);

    assert_eq!(lz4::decompress(&packed, data.len()).unwrap(), data);
}
