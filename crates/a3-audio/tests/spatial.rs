//! 3D processing: attenuation, panning in RV axes, doppler, filters, and 3D voices in the mixer.

use a3_audio::spatial::{Spatialized, doppler_factor, equal_power_pan};
use a3_audio::{
    AudioEngine, Backend, Clip, Curve, DistanceFilter, Emitter, EngineConfig, Listener, Mixer,
    PlayParams, Source, VoiceId,
};
use a3_audio_formats::Sound;
use glam::{DVec3, Vec3};

fn spatialize(listener: &Listener, emitter: &Emitter) -> Spatialized {
    a3_audio::spatial::spatialize(listener, emitter, 48_000.0)
}

fn falloff(range: f32) -> Curve {
    Curve::new([(0.0, 1.0), (range, 0.0)])
}

fn listener_at(position: DVec3) -> Listener {
    Listener {
        position,
        ..Listener::default()
    }
}

#[test]
fn equal_power_pan_keeps_the_power_constant() {
    for pan in [-1.0, -0.3, 0.0, 0.5, 1.0] {
        let [l, r] = equal_power_pan(pan);
        assert!((l * l + r * r - 1.0).abs() < 1e-5);
    }
    assert!(equal_power_pan(-1.0)[1].abs() < 1e-6);
}

#[test]
fn distance_follows_the_attenuation_curve_in_f64_world_space() {
    // Far from the origin, as on a 30 km terrain: positions stay exact in f64.
    let base = DVec3::new(25_000.0, 50.0, 25_000.0);
    let listener = listener_at(base);
    let emitter = Emitter::at(base + DVec3::new(0.0, 0.0, 25.0), falloff(100.0));
    let s = spatialize(&listener, &emitter);
    assert!((s.distance - 25.0).abs() < 1e-4);
    let total = (s.gains[0].powi(2) + s.gains[1].powi(2)).sqrt();
    assert!((total - 0.75).abs() < 1e-4, "{total}");
    let far = Emitter::at(base + DVec3::new(0.0, 0.0, 150.0), falloff(100.0));
    assert_eq!(spatialize(&listener, &far).gains, [0.0, 0.0]);
}

#[test]
fn east_is_on_the_right_when_facing_north() {
    // RV: X east, Y up, Z north. The default listener faces north.
    let listener = Listener::default();
    let east = spatialize(
        &listener,
        &Emitter::at(DVec3::new(10.0, 0.0, 0.0), falloff(100.0)),
    );
    assert!(
        east.gains[1] > 0.99 * east.gains.iter().sum::<f32>(),
        "{:?}",
        east.gains
    );
    let west = spatialize(
        &listener,
        &Emitter::at(DVec3::new(-10.0, 0.0, 0.0), falloff(100.0)),
    );
    assert!(west.gains[0] > 0.99 * west.gains.iter().sum::<f32>());

    // Facing east, a source to the north is on the left.
    let facing_east = Listener {
        forward: Vec3::X,
        ..Listener::default()
    };
    let north = spatialize(
        &facing_east,
        &Emitter::at(DVec3::new(0.0, 0.0, 10.0), falloff(100.0)),
    );
    assert!(north.gains[0] > north.gains[1]);
}

#[test]
fn sources_behind_are_quieter_and_duller_and_close_sources_spread() {
    let listener = Listener::default();
    let front = spatialize(
        &listener,
        &Emitter::at(DVec3::new(0.0, 0.0, 10.0), falloff(100.0)),
    );
    let back = spatialize(
        &listener,
        &Emitter::at(DVec3::new(0.0, 0.0, -10.0), falloff(100.0)),
    );
    assert!(back.gains[0] < front.gains[0]);
    assert!(front.cutoff_hz.is_none() && back.cutoff_hz.is_some());

    let mut near = Emitter::at(DVec3::new(0.5, 0.0, 0.0), falloff(100.0));
    near.spread_radius = 2.0;
    let s = spatialize(&listener, &near);
    assert!(
        s.gains[0] > 0.4 * s.gains[1],
        "a near source is not hard-panned: {:?}",
        s.gains
    );
}

#[test]
fn doppler_raises_the_pitch_of_an_approaching_source() {
    // Source 100 m north, flying south toward the listener at 34.3 m/s (a tenth of c).
    let to_source = Vec3::Z;
    let approaching = doppler_factor(Vec3::ZERO, Vec3::new(0.0, 0.0, -34.3), to_source);
    assert!((approaching - 1.0 / 0.9).abs() < 1e-3, "{approaching}");
    let receding = doppler_factor(Vec3::ZERO, Vec3::new(0.0, 0.0, 34.3), to_source);
    assert!((receding - 1.0 / 1.1).abs() < 1e-3);
    let listener_moving = doppler_factor(Vec3::new(0.0, 0.0, 34.3), Vec3::ZERO, to_source);
    assert!((listener_moving - 1.1).abs() < 1e-3);

    let mut emitter = Emitter::at(DVec3::new(0.0, 0.0, 100.0), falloff(1000.0));
    emitter.velocity = Vec3::new(0.0, 0.0, -34.3);
    assert!(spatialize(&Listener::default(), &emitter).pitch > 1.1);
    emitter.doppler = 0.0;
    assert_eq!(spatialize(&Listener::default(), &emitter).pitch, 1.0);
}

#[test]
fn distance_filter_and_occlusion_close_the_low_pass() {
    let filter = DistanceFilter {
        min_cutoff_hz: 150.0,
        q: 1.0,
        inner_range: 10.0,
        range: 1000.0,
        power: 32.0,
    };
    // The engine's formula: open (the sample rate) inside innerRange, the minimum beyond range,
    // fs + (min - fs) * ((d - inner) / (range - inner))^(1 / power) between.
    let fs = 48_000.0;
    assert_eq!(filter.cutoff_at(5.0, fs), fs);
    assert_eq!(filter.cutoff_at(1001.0, fs), 150.0);
    let t = (90.0f32 / 990.0).powf(1.0 / 32.0);
    assert!((filter.cutoff_at(100.0, fs) - (fs + (150.0 - fs) * t)).abs() < 1.0);
    assert!(filter.cutoff_at(100.0, fs) < filter.cutoff_at(20.0, fs));

    let mut emitter = Emitter::at(DVec3::new(0.0, 0.0, 5.0), falloff(100.0));
    assert_eq!(spatialize(&Listener::default(), &emitter).cutoff_hz, None);
    emitter.occlusion = 1.0;
    let s = spatialize(&Listener::default(), &emitter);
    assert!(s.cutoff_hz.unwrap() < 1000.0);
    assert!(s.gains[0] < 0.5 * std::f32::consts::FRAC_1_SQRT_2);
}

fn tone(rate: u32, frames: usize) -> Clip {
    Clip::from_sound(Sound {
        sample_rate: rate,
        channels: 1,
        samples: vec![16_384; frames],
    })
    .unwrap()
}

#[test]
fn a_3d_voice_follows_the_listener() {
    let mut m = Mixer::new(48_000, 8);
    let params = PlayParams {
        emitter: Some(Emitter::at(DVec3::new(10.0, 0.0, 0.0), falloff(20.0))),
        looping: true,
        ..PlayParams::default()
    };
    m.play(VoiceId(1), Source::Clip(tone(48_000, 1000)), params);
    let mut out = vec![0.0; 2048];
    m.render(&mut out);
    let (l, r) = (out[1800], out[1801]);
    assert!(
        r > 0.2 && l.abs() < 0.01,
        "east of a north-facing listener: {l} {r}"
    );

    // Walk the listener far away: the voice fades to silence and stops taking a slot.
    m.apply(a3_audio::Command::SetListener(listener_at(DVec3::new(
        0.0, 0.0, -100.0,
    ))));
    m.render(&mut out);
    m.render(&mut out);
    assert!(out.iter().all(|s| s.abs() < 1e-6));
    assert_eq!(m.stats().audible, 0);
    assert!(m.is_playing(VoiceId(1)));
}

#[test]
fn the_null_backend_plays_voices_in_real_time() {
    let engine = AudioEngine::start(EngineConfig {
        backend: Backend::Null,
        ..EngineConfig::default()
    })
    .unwrap();
    assert_eq!(engine.sample_rate(), 48_000);
    engine.play(Source::Clip(tone(48_000, 2400)), PlayParams::default()); // 50 ms
    let start = std::time::Instant::now();
    let mut seen = false;
    while start.elapsed() < std::time::Duration::from_secs(5) {
        let stats = engine.stats();
        seen |= stats.voices == 1;
        if seen && stats.voices == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let stats = engine.stats();
    assert!(seen, "the voice started");
    assert_eq!(stats.voices, 0, "and finished");
    assert!(stats.frames >= 2400);
}
