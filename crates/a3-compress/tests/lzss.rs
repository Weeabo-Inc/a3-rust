use a3_compress::Error;
use a3_compress::lzss::{self, ChecksumKind};
use proptest::prelude::*;

#[test]
fn literal_only_stream_decodes_bytes_verbatim() {
    let input = [0xFF, b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H'];

    let (out, consumed) = lzss::decompress(&input, 8, ChecksumKind::None).unwrap();

    assert_eq!(out, b"ABCDEFGH");
    assert_eq!(consumed, 9);
}

#[test]
fn back_reference_counts_distance_from_current_position() {
    // "ABC" then a reference 3 back, length 6 (overlapping): "ABCABCABC".
    // Flags: literal, literal, literal, reference -> 0b0111.
    let input = [0b0000_0111, b'A', b'B', b'C', 0x03, 0x03];

    let (out, consumed) = lzss::decompress(&input, 9, ChecksumKind::None).unwrap();

    assert_eq!(out, b"ABCABCABC");
    assert_eq!(consumed, 6);
}

#[test]
fn twelve_bit_distance_uses_high_nibble_of_second_byte() {
    // 0x101 literal 'x' bytes, then a 3-byte reference 0x101 back.
    let mut input = Vec::new();
    let mut expected = Vec::new();
    for chunk in 0..(0x101usize.div_ceil(8)) {
        let n = (0x101 - chunk * 8).min(8);
        input.push(if n == 8 { 0xFF } else { (1u8 << n) - 1 });
        for i in 0..n {
            let b = (chunk * 8 + i) as u8;
            input.push(b);
            expected.push(b);
        }
        if n < 8 {
            // The reference sits in the same group, right after the last literal.
            input.extend([0x01, 0x10]);
        }
    }
    expected.extend([0, 1, 2]);

    let (out, _) = lzss::decompress(&input, expected.len(), ChecksumKind::None).unwrap();

    assert_eq!(out, expected);
}

#[test]
fn reference_before_start_of_output_reads_spaces() {
    // A reference at output offset 0, distance 2, length 4: two window spaces then a copy of them.
    let input = [0b0000_0000, 0x02, 0x01];

    let (out, _) = lzss::decompress(&input, 4, ChecksumKind::None).unwrap();

    assert_eq!(out, b"    ");
}

#[test]
fn signed_checksum_sums_bytes_as_i8() {
    // 0x80 + 0x01 as i8 = -128 + 1 = -127.
    let input = [0xFF, 0x80, 0x01, 0x81, 0xFF, 0xFF, 0xFF];

    let (out, consumed) = lzss::decompress(&input, 2, ChecksumKind::Signed).unwrap();

    assert_eq!(out, [0x80, 0x01]);
    assert_eq!(consumed, 7);
}

#[test]
fn unsigned_checksum_sums_bytes_as_u8() {
    let input = [0xFF, 0x80, 0x01, 0x81, 0x00, 0x00, 0x00];

    let (out, consumed) = lzss::decompress(&input, 2, ChecksumKind::Unsigned).unwrap();

    assert_eq!(out, [0x80, 0x01]);
    assert_eq!(consumed, 7);
}

#[test]
fn wrong_checksum_is_rejected() {
    let input = [0xFF, 0x80, 0x01, 0x82, 0x00, 0x00, 0x00];

    let err = lzss::decompress(&input, 2, ChecksumKind::Unsigned).unwrap_err();

    assert!(matches!(
        err,
        Error::ChecksumMismatch {
            stored: 0x82,
            computed: 0x81
        }
    ));
}

#[test]
fn trailing_bytes_after_the_block_are_not_consumed() {
    let input = [0xFF, b'h', b'i', 0xD1, 0, 0, 0, 0xAA, 0xBB];

    let (out, consumed) = lzss::decompress(&input, 2, ChecksumKind::Unsigned).unwrap();

    assert_eq!(out, b"hi");
    assert_eq!(consumed, 7);
}

#[test]
fn truncated_input_is_an_error() {
    let input = [0xFF, b'A', b'B'];

    let err = lzss::decompress(&input, 3, ChecksumKind::None).unwrap_err();

    assert!(matches!(
        err,
        Error::UnexpectedEof {
            produced: 2,
            expected: 3,
            ..
        }
    ));
}

#[test]
fn reference_past_expected_length_is_an_error() {
    let input = [0b0000_0001, b'A', 0x01, 0x0F];

    let err = lzss::decompress(&input, 4, ChecksumKind::None).unwrap_err();

    assert!(matches!(err, Error::OutputOverrun { expected: 4 }));
}

#[test]
fn stream_decoder_leaves_reader_after_the_block() {
    let input = [0xFF, b'h', b'i', 0xD1, 0, 0, 0, 0xAA, 0xBB];
    let mut reader = &input[..];

    let (out, consumed) = lzss::decompress_from(&mut reader, 2, ChecksumKind::Unsigned).unwrap();

    assert_eq!(out, b"hi");
    assert_eq!(consumed, 7);
    assert_eq!(reader, [0xAA, 0xBB]);
}

#[test]
fn compressing_repetitive_data_shrinks_it() {
    let data = b"abcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabc".repeat(20);

    let packed = lzss::compress(&data, ChecksumKind::Signed);

    assert!(packed.len() < data.len() / 4, "{} bytes", packed.len());
    let (out, consumed) = lzss::decompress(&packed, data.len(), ChecksumKind::Signed).unwrap();
    assert_eq!(out, data);
    assert_eq!(consumed, packed.len());
}

#[test]
fn compressing_empty_input_gives_just_the_checksum() {
    assert_eq!(lzss::compress(&[], ChecksumKind::Unsigned), [0, 0, 0, 0]);
    assert_eq!(lzss::compress(&[], ChecksumKind::None), [] as [u8; 0]);
}

fn checksum_kind() -> impl Strategy<Value = ChecksumKind> {
    prop_oneof![
        Just(ChecksumKind::Signed),
        Just(ChecksumKind::Unsigned),
        Just(ChecksumKind::None),
    ]
}

/// Data with plenty of repeats, long runs and far matches, plus arbitrary bytes.
fn compressible() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop_oneof![
            prop::collection::vec(any::<u8>(), 0..40),
            (any::<u8>(), 0..300usize).prop_map(|(b, n)| vec![b; n]),
            prop::collection::vec(prop::sample::select(b" abc".to_vec()), 0..200),
        ],
        0..30,
    )
    .prop_map(|parts| parts.concat())
}

proptest! {
    #[test]
    fn compress_then_decompress_round_trips(data in compressible(), kind in checksum_kind()) {
        let packed = lzss::compress(&data, kind);
        let (out, consumed) = lzss::decompress(&packed, data.len(), kind).unwrap();
        prop_assert_eq!(out, data);
        prop_assert_eq!(consumed, packed.len());
    }

    #[test]
    fn decompress_never_panics_on_garbage(
        input in prop::collection::vec(any::<u8>(), 0..512),
        len in 0..2048usize,
        kind in checksum_kind(),
    ) {
        let _ = lzss::decompress(&input, len, kind);
    }
}
