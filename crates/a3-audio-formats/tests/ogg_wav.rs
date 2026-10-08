//! Ogg Vorbis (a generated fixture: 0.25 s of a 440 Hz sine, stereo, 22,050 Hz) and WAV.

use a3_audio_formats::{Format, Sound, decode, probe};

const SINE_OGG: &[u8] = include_bytes!("fixtures/sine440_stereo.ogg");

#[test]
fn probes_an_ogg_vorbis_header() {
    let info = probe(SINE_OGG).unwrap();
    assert_eq!(info.format, Format::OggVorbis);
    assert_eq!((info.sample_rate, info.channels), (22050, 2));
    assert_eq!(info.frames, Some(5513));
}

#[test]
fn decodes_ogg_vorbis_to_a_440_hz_tone() {
    let sound = decode(SINE_OGG).unwrap();
    assert_eq!((sound.sample_rate, sound.channels), (22050, 2));
    assert_eq!(sound.frames(), 5513);

    // Count rising zero crossings of the left channel over the middle 0.2 s: about 88.
    let left: Vec<i16> = sound.samples.iter().step_by(2).copied().collect();
    let middle = &left[551..551 + 4410];
    let rising = middle.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count();
    assert!((86..=90).contains(&rising), "{rising} rising crossings");
    let peak = middle.iter().map(|s| s.unsigned_abs()).max().unwrap();
    assert!(peak > 2000, "audible: peak {peak}");
}

#[test]
fn wav_round_trips_through_to_wav() {
    let sound = Sound {
        sample_rate: 8000,
        channels: 2,
        samples: vec![0, -1, i16::MAX, i16::MIN, 1234, -4321],
    };
    let wav = sound.to_wav();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(decode(&wav).unwrap(), sound);

    let info = probe(&wav).unwrap();
    assert_eq!(info.format, Format::Wav);
    assert_eq!(info.frames, Some(3));
    assert_eq!(info.bits_per_sample, Some(16));
}

#[test]
fn converts_to_f32() {
    let sound = Sound {
        sample_rate: 8000,
        channels: 1,
        samples: vec![0, i16::MIN, 16384],
    };
    assert_eq!(sound.to_f32(), [0.0, -1.0, 0.5]);
    assert_eq!(sound.duration().as_micros(), 375);
}
