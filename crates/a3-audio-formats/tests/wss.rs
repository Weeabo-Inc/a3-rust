//! WSS files built byte by byte.

use a3_audio_formats::{Error, Format, Sound, WssCompression, decode, probe};

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
    let sound = decode(&wss(0, 1, 192000, 24, &[0x56, 0x34, 0x12, 0x00, 0x00, 0x80])).unwrap();
    assert_eq!(sound.samples, [0x1234, i16::MIN]);
}

#[test]
fn decodes_logarithmic_deltas_per_channel() {
    // Codes are signed bytes; each adds sign(c) * round(32767^(|c|/127)) to its channel.
    // 127 -> 32767, 64 -> 189, 1 -> 1, 0 -> 0.
    let codes = [64u8, 1, 64, 0, (-64i8) as u8, 0, (-127i8) as u8, 127];
    let sound = decode(&wss(8, 2, 22050, 16, &codes)).unwrap();
    assert_eq!(sound.channels, 2);
    assert_eq!(sound.samples, [189, 1, 378, 1, 189, 1, -32578, 32767]);
}

#[test]
fn delta_decoding_saturates_at_the_sample_range() {
    let sound = decode(&wss(8, 1, 22050, 16, &[127, 127, 0x81, 0x81, 0x81])).unwrap();
    assert_eq!(sound.samples, [32767, 32767, 0, -32767, -32768]);
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
    let err = decode(&wss(4, 1, 22050, 16, &[0; 8])).unwrap_err();
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
