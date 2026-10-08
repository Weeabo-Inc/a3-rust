//! Playing config sound sets: sample choice, randomizers, shader limits, distance gains.

use a3_audio::config::SoundBank;
use a3_audio::player::{
    AUDIBLE_THRESHOLD, Rng, SoundSetPlayer, audible_shaders, choose_sample, shader_distance_gain,
    start_levels,
};
use a3_audio::{AudioEngine, Backend, Curve, EngineConfig, SoundLoader};
use a3_audio_formats::Sound;
use a3_config::{ConfigTree, parse_text};
use a3_pbo::{Pbo, PboWriter};
use a3_vfs::Vfs;

fn bank(text: &str) -> SoundBank {
    SoundBank::load(&ConfigTree::from_config(&parse_text(text).unwrap()))
}

#[test]
fn samples_follow_their_probabilities_and_do_not_repeat() {
    let mut rng = Rng::new(7);
    let mut counts = [0; 3];
    for _ in 0..10_000 {
        counts[choose_sample(&[1.0, 3.0, 0.0], None, &mut rng).unwrap()] += 1;
    }
    assert_eq!(counts[2], 0);
    let share = counts[1] as f32 / 10_000.0;
    assert!((share - 0.75).abs() < 0.03, "{counts:?}");
    // A repeat of the previous pick moves to the next sample.
    for _ in 0..100 {
        assert_eq!(choose_sample(&[0.0, 1.0], Some(1), &mut rng), Some(0));
    }
    assert_eq!(choose_sample(&[], None, &mut rng), None);
}

#[test]
fn the_rng_is_the_engine_lcg() {
    let mut rng = Rng::new(1);
    // state = (1 * 1103515245 + 12345) & 0x7fffffff = 1103527590
    assert!((rng.next_f32() - 1_103_527_590.0 * 4.656_613e-10).abs() < 1e-6);
    let xs: Vec<f32> = (0..1000).map(|_| rng.next_f32()).collect();
    assert!(xs.iter().all(|x| (0.0..1.0).contains(x)));
}

#[test]
fn randomizers_vary_volume_in_db_and_pitch_in_semitones() {
    let b = bank(
        r#"class CfgSoundSets {
            class Plain { soundShaders[] = {}; volumeFactor = 0.5; frequencyFactor = 1.5; };
            class Random { soundShaders[] = {}; volumeRandomizer = 1.995262; frequencyRandomizer = 12; frequencyRandomizerMin = 12; };
        };"#,
    );
    let mut rng = Rng::new(3);
    assert_eq!(start_levels(b.set("Plain").unwrap(), &mut rng), (0.5, 1.5));
    for _ in 0..100 {
        let (volume, pitch) = start_levels(b.set("Random").unwrap(), &mut rng);
        // Up to +-6 dB, and exactly an octave up or down.
        assert!((0.5..=2.0).contains(&volume), "{volume}");
        assert!(
            (pitch - 2.0).abs() < 1e-4 || (pitch - 0.5).abs() < 1e-4,
            "{pitch}"
        );
    }
}

#[test]
fn shader_distance_gain_uses_the_range_curve_and_cuts_beyond_range() {
    let curve = Curve::new([(0.0, 1.0), (1.0, 0.0)]);
    assert_eq!(shader_distance_gain(100.0, Some(&curve), 25.0), 0.75);
    assert_eq!(shader_distance_gain(100.0, None, 99.0), 1.0);
    assert_eq!(shader_distance_gain(100.0, None, 101.0), 0.0);
}

#[test]
fn the_shader_limit_keeps_the_loudest_limited_shaders() {
    let volumes = [0.5, 0.9, 0.7, 0.001, 0.2];
    let limited = [true, true, true, true, false];
    assert_eq!(
        audible_shaders(&volumes, &limited, 2),
        [false, true, true, false, true]
    );
    assert_eq!(
        audible_shaders(&volumes, &limited, 0),
        [true, true, true, false, true]
    );
    assert!((AUDIBLE_THRESHOLD - 10f32.powf(-50.0 / 20.0)).abs() < 1e-7, "-50 dB");
}

#[test]
fn a_looping_set_starts_one_voice_per_playable_shader() {
    let b = bank(
        r#"
        class CfgSoundShaders {
            class A { samples[] = {{"a3\snd\a", 1}}; volume = "forest"; };
            class B { samples[] = {{"a3\snd\missing", 1}}; volume = 1; };
        };
        class CfgSoundSets {
            class Env_SoundSet { soundShaders[] = {"A", "B", "Nope"}; loop = 1; spatial = 0; volumeFactor = 0.5; volume = "1 - rain"; };
        };
        "#,
    );
    let wav = Sound {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![1000; 4800],
    }
    .to_wav();
    let vfs = Vfs::new();
    let pbo = PboWriter::new()
        .property("prefix", r"a3\snd")
        .file("a.wav", wav)
        .to_bytes();
    vfs.mount_pbo(Pbo::from_bytes(pbo).unwrap(), None);
    let loader = SoundLoader::new(vfs);
    let engine = AudioEngine::start(EngineConfig {
        backend: Backend::Null,
        ..EngineConfig::default()
    })
    .unwrap();

    let player =
        SoundSetPlayer::start_loop(&b, &loader, &engine, "env_soundset", &mut Rng::new(1)).unwrap();
    assert_eq!(player.name(), "Env_SoundSet");
    assert_eq!(player.playing_shaders(), 1);
    let vars = |v: &str| match v {
        "forest" => Some(0.25),
        "rain" => Some(0.5),
        _ => None,
    };
    // Shader volume x set volume expression: A 0.25 * 0.5, B 1 * 0.5.
    assert_eq!(player.shader_volumes(&vars, None), [0.125, 0.5]);
    // Times volumeFactor 0.5 and the gain 2.
    assert_eq!(player.update(&engine, &vars, 2.0), 0.5);

    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(engine.stats().voices, 1);
    player.stop(&engine, 0.0);
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(engine.stats().voices, 0);
    assert!(SoundSetPlayer::start_loop(&b, &loader, &engine, "nope", &mut Rng::new(1)).is_none());
}
