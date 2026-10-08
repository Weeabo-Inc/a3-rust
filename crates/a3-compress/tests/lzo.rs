use a3_compress::{Error, lzo};
use proptest::prelude::*;

/// End-of-stream marker: an M4 match with distance 0x4000 and no extra length.
const EOS: [u8; 3] = [0x11, 0x00, 0x00];

#[test]
fn initial_literal_run_then_end_marker() {
    // A first byte above 17 is a literal run of (byte - 17) bytes.
    let mut input = vec![17 + 5];
    input.extend(b"hello");
    input.extend(EOS);

    let (out, consumed) = lzo::decompress(&input, 5).unwrap();

    assert_eq!(out, b"hello");
    assert_eq!(consumed, 9);
}

#[test]
fn m2_match_copies_from_recent_output() {
    // Literals "abcd", then an M2 match (instruction >= 64): length 3, distance 4.
    // M2: length = (t >> 5) + 1, distance = ((t >> 2) & 7) + (next << 3) + 1.
    let t = (2 << 5) | (3 << 2); // length 3, distance low bits 3 -> distance 4
    let mut input = vec![17 + 4];
    input.extend(b"abcd");
    input.extend([t, 0x00]);
    input.extend(EOS);

    let (out, consumed) = lzo::decompress(&input, 7).unwrap();

    assert_eq!(out, b"abcdabc");
    assert_eq!(consumed, input.len());
}

#[test]
fn match_state_bits_append_trailing_literals() {
    // The low two bits of an M2 instruction copy 1..=3 literals after the match.
    let t = (2 << 5) | (3 << 2) | 2; // length 3, distance 4, then 2 literals
    let mut input = vec![17 + 4];
    input.extend(b"abcd");
    input.extend([t, 0x00, b'X', b'Y']);
    input.extend(EOS);

    let (out, _) = lzo::decompress(&input, 9).unwrap();

    assert_eq!(out, b"abcdabcXY");
}

#[test]
fn trailing_bytes_after_end_marker_are_not_consumed() {
    let mut input = vec![17 + 2, b'o', b'k'];
    input.extend(EOS);
    input.extend([0xDE, 0xAD]);

    let (out, consumed) = lzo::decompress(&input, 2).unwrap();

    assert_eq!(out, b"ok");
    assert_eq!(consumed, 6);
}

#[test]
fn stream_decoder_leaves_reader_after_the_block() {
    let mut input = vec![17 + 2, b'o', b'k'];
    input.extend(EOS);
    input.extend([0xDE, 0xAD]);
    let mut reader = &input[..];

    let (out, consumed) = lzo::decompress_from(&mut reader, 2).unwrap();

    assert_eq!(out, b"ok");
    assert_eq!(consumed, 6);
    assert_eq!(reader, [0xDE, 0xAD]);
}

#[test]
fn end_marker_before_expected_length_is_an_error() {
    let mut input = vec![17 + 2, b'o', b'k'];
    input.extend(EOS);

    let err = lzo::decompress(&input, 3).unwrap_err();

    assert!(matches!(
        err,
        Error::OutputUnderrun {
            produced: 2,
            expected: 3
        }
    ));
}

#[test]
fn more_output_than_expected_is_an_error() {
    let mut input = vec![17 + 5];
    input.extend(b"hello");
    input.extend(EOS);

    let err = lzo::decompress(&input, 4).unwrap_err();

    assert!(matches!(err, Error::OutputOverrun { expected: 4 }));
}

#[test]
fn distance_before_start_of_output_is_an_error() {
    // M2 match reaching 8 bytes back after only 2 bytes of output.
    let t = (2 << 5) | (7 << 2);
    let mut input = vec![17 + 2, b'a', b'b', t, 0x00];
    input.extend(EOS);

    let err = lzo::decompress(&input, 5).unwrap_err();

    assert!(matches!(err, Error::InvalidDistance { .. }));
}

#[test]
fn truncated_input_is_an_error() {
    let input = [17 + 5, b'h', b'e'];

    let err = lzo::decompress(&input, 5).unwrap_err();

    assert!(matches!(err, Error::UnexpectedEof { .. }));
}

/// Data with plenty of repeats, long runs and far matches, plus arbitrary bytes.
fn compressible() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop_oneof![
            prop::collection::vec(any::<u8>(), 0..300),
            (any::<u8>(), 0..3000usize).prop_map(|(b, n)| vec![b; n]),
            prop::collection::vec(prop::sample::select(b"abcd".to_vec()), 0..500),
        ],
        0..40,
    )
    .prop_map(|parts| parts.concat())
}

proptest! {
    /// `lzokay-native` (an independent LZO1X encoder) is the oracle.
    #[test]
    fn decodes_independent_encoder_output(data in compressible(), trailer in prop::collection::vec(any::<u8>(), 0..8)) {
        let mut packed = lzokay_native::compress(&data).unwrap();
        let block_len = packed.len();
        packed.extend(&trailer);

        let (out, consumed) = lzo::decompress(&packed, data.len()).unwrap();

        prop_assert_eq!(out, data);
        prop_assert_eq!(consumed, block_len);
    }

    #[test]
    fn decompress_never_panics_on_garbage(
        input in prop::collection::vec(any::<u8>(), 0..512),
        len in 0..4096usize,
    ) {
        let _ = lzo::decompress(&input, len);
    }
}
