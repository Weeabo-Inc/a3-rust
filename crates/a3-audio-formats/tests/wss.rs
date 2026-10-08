//! WSS files built byte by byte.

use a3_audio_formats::{Error, Format, Sound, WssCompression, decode, delta8_step, probe};

fn wss(compression: u32, channels: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
    let block_align = channels * bits / 8;
    let mut out = b"WSS0".to_vec();
    out.extend_from_slice(&compression.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM format tag
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(data);
    out
}

#[test]
fn decodes_16_bit_pcm() {
    let data: Vec<u8> = [0i16, 1000, -1000, i16::MAX]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    let sound = decode(&wss(0, 2, 44100, 16, &data)).unwrap();
    assert_eq!(
        sound,
        Sound {
            sample_rate: 44100,
            channels: 2,
            samples: vec![0, 1000, -1000, i16::MAX],
        }
    );
    assert_eq!(sound.frames(), 2);
}

#[test]
fn decodes_8_bit_unsigned_pcm() {
    let sound = decode(&wss(0, 1, 8000, 8, &[128, 255, 0])).unwrap();
    assert_eq!(sound.samples, [0, 127 << 8, -128 << 8]);
}

#[test]
fn decodes_24_bit_pcm_to_its_top_16_bits() {
    let sound = decode(&wss(
        0,
        1,
        192000,
        24,
        &[0x56, 0x34, 0x12, 0x00, 0x00, 0x80],
    ))
    .unwrap();
    assert_eq!(sound.samples, [0x1234, i16::MIN]);
}

#[test]
fn decodes_logarithmic_deltas_per_channel() {
    // Codes are signed bytes; each adds sign(c) * round(1.0853122^|c|) to its channel.
    // 127 -> 32768, 64 -> 189, 1 -> 1, 0 -> 0.
    let codes = [64u8, 1, 64, 0, (-64i8) as u8, 0, (-127i8) as u8, 127];
    let sound = decode(&wss(8, 2, 22050, 16, &codes)).unwrap();
    assert_eq!(sound.channels, 2);
    assert_eq!(sound.samples, [189, 1, 378, 1, 189, 1, -32579, 32767]);
}

#[test]
fn delta8_table_matches_known_entries() {
    let table: Vec<i32> = [1u8, 5, 92, 122, 126, 127, 0x80, 0xff, 0x81]
        .map(delta8_step)
        .to_vec();
    assert_eq!(table, [1, 2, 1867, 21761, 30192, 32768, 0, -1, -32768]);
}

#[test]
fn delta_sums_run_past_the_sample_range_and_only_the_output_saturates() {
    let sound = decode(&wss(8, 1, 22050, 16, &[127, 127, 0x81, 0x81, 0x81])).unwrap();
    // Running sum: 32768, 65536, 32768, 0, -32768.
    assert_eq!(sound.samples, [32767, 32767, 32767, 0, -32768]);
}

#[test]
fn decodes_4_bit_deltas_high_nibble_first() {
    // Table: 0..=15 -> -8192 -4096 -2048 -1024 -512 -256 -64 0 64 256 512 1024 2048 4096 8192 0.
    let mono = decode(&wss(4, 1, 22050, 16, &[0x8e, 0x07])).unwrap();
    assert_eq!(mono.samples, [64, 8256, 64, 64]);

    // Stereo: high nibble left, low nibble right.
    let stereo = decode(&wss(4, 2, 22050, 16, &[0x8e, 0x0f])).unwrap();
    assert_eq!(stereo.samples, [64, 8192, -8128, 8192]);
    assert_eq!(
        probe(&wss(4, 2, 22050, 16, &[0; 3])).unwrap().frames,
        Some(3)
    );
}

#[test]
fn probes_the_header_without_decoding() {
    let info = probe(&wss(8, 1, 22050, 16, &[1, 2, 3, 4, 5])).unwrap();
    assert_eq!(info.format, Format::Wss(WssCompression::Delta8));
    assert_eq!((info.sample_rate, info.channels), (22050, 1));
    assert_eq!(info.frames, Some(5));
}

#[test]
fn rejects_unknown_compression() {
    let err = decode(&wss(2, 1, 22050, 16, &[0; 8])).unwrap_err();
    assert!(matches!(err, Error::Unsupported(_)), "{err:?}");
}

#[test]
fn rejects_a_truncated_header() {
    let file = wss(0, 1, 22050, 16, &[]);
    for len in 4..file.len() {
        assert!(decode(&file[..len]).is_err(), "length {len}");
    }
}

#[test]
fn rejects_unknown_signatures() {
    let err = decode(b"RIFX\0\0\0\0").unwrap_err();
    assert!(matches!(err, Error::UnknownFormat), "{err:?}");
}
