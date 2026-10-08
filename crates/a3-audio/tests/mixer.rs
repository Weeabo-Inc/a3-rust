//! The mixer driven directly: gains, panning, resampling, voice limits, fades, streams.

use a3_audio::{Clip, Command, Mixer, PlayParams, Source, Stream, VoiceId};
use a3_audio_formats::Sound;

const RATE: u32 = 48_000;
const CENTRE: f32 = std::f32::consts::FRAC_1_SQRT_2;

fn constant(level: f32, frames: usize, rate: u32) -> Clip {
    let v = (level * 32768.0) as i16;
    Clip::from_sound(Sound {
        sample_rate: rate,
        channels: 1,
        samples: vec![v; frames],
    })
    .unwrap()
}

/// Renders `frames` frames and returns them as (left, right) pairs.
fn render(mixer: &mut Mixer, frames: usize) -> Vec<(f32, f32)> {
    let mut out = vec![0.0; frames * 2];
    mixer.render(&mut out);
    out.chunks(2).map(|c| (c[0], c[1])).collect()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 2e-3
}

#[test]
fn a_centred_voice_reaches_both_channels_at_equal_power() {
    let mut m = Mixer::new(RATE, 8);
    m.play(
        VoiceId(1),
        Source::Clip(constant(0.5, 10_000, RATE)),
        PlayParams::default(),
    );
    let out = render(&mut m, 1024);
    // The first block ramps up from silence; after it the level is steady.
    assert!(out[0].0.abs() < 0.01);
    let (l, r) = out[600];
    assert!(close(l, 0.5 * CENTRE) && close(r, 0.5 * CENTRE), "{l} {r}");
}

#[test]
fn panning_hard_left_silences_the_right_channel() {
    let mut m = Mixer::new(RATE, 8);
    let params = PlayParams {
        pan: -1.0,
        gain: 0.5,
        ..PlayParams::default()
    };
    m.play(
        VoiceId(1),
        Source::Clip(constant(1.0, 10_000, RATE)),
        params,
    );
    let (l, r) = render(&mut m, 1024)[700];
    assert!(close(l, 0.5) && r.abs() < 1e-6, "{l} {r}");
}

#[test]
fn a_lower_rate_clip_is_resampled_with_linear_interpolation() {
    // A ramp at 24 kHz: sample i is i / 4096. Played at 48 kHz, output frame n reads source
    // position n / 2.
    let samples: Vec<i16> = (0..4096).map(|i| (i * 8) as i16).collect();
    let clip = Clip::from_sound(Sound {
        sample_rate: 24_000,
        channels: 1,
        samples,
    })
    .unwrap();
    let mut m = Mixer::new(RATE, 8);
    let params = PlayParams {
        pan: -1.0,
        ..PlayParams::default()
    };
    m.play(VoiceId(1), Source::Clip(clip), params);
    let out = render(&mut m, 2048);
    let expected = |n: usize| (n as f32 / 2.0) * 8.0 / 32768.0;
    for n in [601, 1001, 1500] {
        assert!(
            close(out[n].0, expected(n)),
            "frame {n}: {} vs {}",
            out[n].0,
            expected(n)
        );
    }
}

#[test]
fn pitch_changes_the_playback_rate_and_one_shots_end() {
    let mut m = Mixer::new(RATE, 8);
    let params = PlayParams {
        pitch: 2.0,
        ..PlayParams::default()
    };
    m.play(VoiceId(1), Source::Clip(constant(0.5, 1000, RATE)), params);
    render(&mut m, 256);
    assert!(m.is_playing(VoiceId(1)));
    render(&mut m, 512); // 768 output frames read 1536 source frames: past the end
    assert!(!m.is_playing(VoiceId(1)));
    assert_eq!(m.stats().voices, 0);
}

#[test]
fn looping_clips_keep_playing() {
    let mut m = Mixer::new(RATE, 8);
    let params = PlayParams {
        looping: true,
        ..PlayParams::default()
    };
    m.play(VoiceId(1), Source::Clip(constant(0.5, 100, RATE)), params);
    let out = render(&mut m, 4096);
    assert!(m.is_playing(VoiceId(1)));
    assert!(close(out[4000].0, 0.5 * CENTRE));
}

#[test]
fn the_voice_limit_keeps_the_highest_priorities() {
    let mut m = Mixer::new(RATE, 2);
    for (id, priority, level) in [(1, 0, 0.1), (2, 5, 0.2), (3, 1, 0.4)] {
        let params = PlayParams {
            priority,
            pan: -1.0,
            ..PlayParams::default()
        };
        m.play(
            VoiceId(id),
            Source::Clip(constant(level, 10_000, RATE)),
            params,
        );
    }
    let out = render(&mut m, 1024);
    assert_eq!(m.stats().audible, 2);
    assert_eq!(m.stats().voices, 3, "the third voice plays virtually");
    assert!(close(out[800].0, 0.2 + 0.4), "{}", out[800].0);
}

#[test]
fn equal_priorities_keep_the_loudest_and_silent_voices_take_no_slot() {
    let mut m = Mixer::new(RATE, 1);
    let quiet = PlayParams {
        gain: 0.1,
        pan: -1.0,
        ..PlayParams::default()
    };
    let silent = PlayParams {
        gain: 0.0,
        priority: 100,
        ..PlayParams::default()
    };
    let loud = PlayParams {
        gain: 0.8,
        pan: -1.0,
        ..PlayParams::default()
    };
    m.play(VoiceId(1), Source::Clip(constant(1.0, 10_000, RATE)), quiet);
    m.play(
        VoiceId(2),
        Source::Clip(constant(1.0, 10_000, RATE)),
        silent,
    );
    m.play(VoiceId(3), Source::Clip(constant(1.0, 10_000, RATE)), loud);
    let out = render(&mut m, 1024);
    assert!(close(out[800].0, 0.8), "{}", out[800].0);
}

#[test]
fn fades_ramp_in_and_stopping_removes_the_voice_after_the_fade() {
    let mut m = Mixer::new(RATE, 8);
    let params = PlayParams {
        fade_in: 0.1, // 4800 frames
        pan: -1.0,
        ..PlayParams::default()
    };
    m.play(
        VoiceId(1),
        Source::Clip(constant(1.0, 100_000, RATE)),
        params,
    );
    let out = render(&mut m, 4800);
    assert!(close(out[2400].0, 0.5), "half way: {}", out[2400].0);
    assert!(close(render(&mut m, 512)[300].0, 1.0));

    m.apply(Command::Stop {
        id: VoiceId(1),
        fade: 0.05,
    });
    let out = render(&mut m, 1200);
    assert!(close(out[1199].0, 0.5), "{}", out[1199].0);
    render(&mut m, 1300);
    assert!(!m.is_playing(VoiceId(1)));
}

#[test]
fn master_gain_scales_the_mix() {
    let mut m = Mixer::new(RATE, 8);
    m.apply(Command::SetMasterGain(0.5));
    let params = PlayParams {
        pan: -1.0,
        ..PlayParams::default()
    };
    m.play(
        VoiceId(1),
        Source::Clip(constant(0.8, 10_000, RATE)),
        params,
    );
    assert!(close(render(&mut m, 1024)[700].0, 0.4));
}

#[test]
fn a_stream_plays_the_same_audio_as_the_decoded_clip() {
    const OGG: &[u8] = include_bytes!("../../a3-audio-formats/tests/fixtures/sine440_stereo.ogg");
    let clip = Clip::decode(OGG).unwrap();
    let rate = clip.sample_rate();
    let params = PlayParams {
        pan: -1.0,
        ..PlayParams::default()
    };

    let mut a = Mixer::new(rate, 8);
    a.play(VoiceId(1), Source::Clip(clip), params.clone());
    let from_clip = render(&mut a, 4000);

    let mut b = Mixer::new(rate, 8);
    b.play(
        VoiceId(1),
        Source::Stream(Stream::open(OGG, false).unwrap()),
        params,
    );
    // Give the decoder thread a head start so the stream does not underrun.
    std::thread::sleep(std::time::Duration::from_millis(200));
    let from_stream = render(&mut b, 4000);
    for n in [500, 2000, 3999] {
        assert!(close(from_clip[n].0, from_stream[n].0), "frame {n}");
    }
    render(&mut b, 4000);
    assert!(!b.is_playing(VoiceId(1)), "the stream ends");
}
