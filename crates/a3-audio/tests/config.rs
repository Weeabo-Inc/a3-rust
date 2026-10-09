//! Sound classes read from a hand-written config.

use a3_audio::Curve;
use a3_audio::config::{CurveRef, SoundBank, db_to_linear};
use a3_config::{ConfigTree, parse_text};

const CONFIG: &str = r#"
class CfgSoundCurves {
    class LinearCurve { points[] = {{0, 1}, {1, 0}}; };
};
class CfgSoundShaders {
    class Factor_SoundShader { samples[] = {}; volume = 0.5; volumeFactor = 2; };
    class Base_SoundShader {
        samples[] = {{"A3\Sounds_F\a", 1}, {"A3\Sounds_F\b", 3}};
        volume = "forest * (windy factor [0.1, 0.5])";
        frequency = 0.9;
        range = 150;
        rangeCurve[] = {{0, 1}, {150, 0.3}};
        limitation = 1;
    };
    class Named_SoundShader: Base_SoundShader {
        rangeCurve = "LinearCurve";
        volume = "db-10";
    };
    class Broken_SoundShader { samples[] = {}; volume = "1 +"; };
};
class CfgSoundSets {
    class Sea_SoundSet {
        soundShaders[] = {"Base_SoundShader", "Named_SoundShader"};
        volumeFactor = 0.18;
        spatial = 0;
        loop = 1;
        soundShadersLimit = 1;
        frequencyRandomizer = 3;
        distanceFilter = "defaultDistanceFreqAttenuationFilter";
        sound3DProcessingType = "default3DProcessingType";
        volumeCurve[] = {{0, 1}, {100, 0}};
        posOffset[] = {0, 1.5, 0};
    };
    class Bare_SoundSet { soundShaders[] = {"Base_SoundShader"}; };
};
class CfgDistanceFilters {
    class defaultDistanceFreqAttenuationFilter {
        type = "lowPassFilter"; minCutoffFrequency = 150; qFactor = 1;
        innerRange = 10; range = 1000; powerFactor = 32;
    };
};
class CfgSound3DProcessors {
    class default3DProcessingType { type = "emitter"; innerRange = 0; range = 15; radius = 3; };
    class Panner { type = "panner"; innerRange = 5; range = 22; rangeCurve = "LinearCurve"; };
};
class CfgSoundGlobals {
    defaultDistanceFilter = "defaultDistanceFreqAttenuationFilter";
    OldConfigurationVolumeFactor = 0.63095737;
};
class CfgSounds {
    class Hint { sound[] = {"\a3\ui_f\data\sound\hint", 0.8912509, 1}; titles[] = {}; };
    class Loud { sound[] = {"\a3\x", "db+10", 1.2, 50}; };
};
class CfgMusic { class Track { sound[] = {"\music\track.ogg", 1, 1}; duration = 120; }; };
class CfgSFX {
    class Birds {
        sounds[] = {"bird1"};
        bird1[] = {"\a3\birds1", 0.5, 1, 300, 0.4, 2, 5, 10};
        empty[] = {"", 0, 0, 0, 0.6, 1, 2, 3};
    };
};
class CfgEnvSounds {
    class Sea { sound[] = {"", 0.00031622776, 1}; volume = "coast"; };
    soundSetEnvironment[] = {"Sea_SoundSet"};
};
"#;

fn bank() -> SoundBank {
    let config = parse_text(CONFIG).unwrap();
    SoundBank::load(&ConfigTree::from_config(&config))
}

#[test]
fn reads_sound_shaders_with_expressions_and_inheritance() {
    let bank = bank();
    let base = bank.shader("base_soundshader").unwrap();
    assert_eq!(base.samples.len(), 2);
    assert_eq!(base.samples[1].path, r"A3\Sounds_F\b");
    assert_eq!(base.samples[1].probability, 3.0);
    assert_eq!(base.volume.variables(), ["forest", "windy"]);
    assert_eq!(base.frequency.as_constant(), Some(0.9));
    assert_eq!(base.range, 150.0);
    assert_eq!(
        base.range_curve,
        Some(CurveRef::Points(vec![(0.0, 1.0), (150.0, 0.3)]))
    );
    assert!(base.limitation);

    let named = bank.shader("Named_SoundShader").unwrap();
    assert_eq!(
        named.range_curve,
        Some(CurveRef::Named("LinearCurve".into()))
    );
    assert_eq!(named.range, 150.0, "inherited");
    assert!((named.volume.as_constant().unwrap() - db_to_linear(-10.0)).abs() < 1e-6);

    // A constant volume includes the shader's volumeFactor.
    assert_eq!(
        bank.shader("Factor_SoundShader")
            .unwrap()
            .volume
            .as_constant(),
        Some(1.0)
    );

    assert_eq!(bank.warnings.len(), 1, "{:?}", bank.warnings);
    assert!(bank.warnings[0].contains("Broken_SoundShader"));
}

#[test]
fn reads_sound_sets_and_leaves_unset_fields_empty() {
    let bank = bank();
    let sea = bank.set("SEA_SOUNDSET").unwrap();
    assert_eq!(sea.shaders, ["Base_SoundShader", "Named_SoundShader"]);
    assert_eq!(sea.volume_factor, 0.18);
    assert!(!sea.spatial);
    assert!(sea.looping);
    assert_eq!(sea.shaders_limit, 1);
    assert_eq!(sea.frequency_randomizer, 3.0);
    assert_eq!(
        sea.distance_filter.as_deref(),
        Some("defaultDistanceFreqAttenuationFilter")
    );
    assert_eq!(sea.position_offset, Some([0.0, 1.5, 0.0]));
    assert_eq!(
        sea.volume_curve,
        Some(CurveRef::Points(vec![(0.0, 1.0), (100.0, 0.0)]))
    );

    // Inline curves keep their points but resolve renormalised to 0..1, as in the engine.
    assert_eq!(
        bank.resolve_curve(sea.volume_curve.as_ref().unwrap()),
        Some(Curve::new([(0.0, 1.0), (1.0, 0.0)]))
    );

    // Engine defaults and clamps.
    let bare = bank.set("Bare_SoundSet").unwrap();
    assert_eq!(bare.volume_factor, 1.0);
    assert_eq!(
        (bare.spatial, bare.doppler, bare.looping),
        (true, true, false)
    );
    assert_eq!(bare.volume_curve, None);
    assert_eq!(
        (bare.volume_randomizer, bare.frequency_randomizer_min),
        (1.0, 0.0)
    );
    assert_eq!(
        (bare.occlusion_factor, bare.obstruction_factor),
        (0.96, 0.7)
    );
    assert_eq!((bare.spatiality_range, bare.delay), (0.5, None));
    assert_eq!(bare.volume.as_constant(), Some(1.0));
}

#[test]
fn reads_curves_filters_processors_and_globals() {
    let bank = bank();
    assert_eq!(bank.curve("linearcurve").unwrap().eval(0.25), 0.75);
    let filter = bank
        .distance_filter("defaultDistanceFreqAttenuationFilter")
        .unwrap();
    assert_eq!(
        (
            filter.min_cutoff_hz,
            filter.inner_range,
            filter.range,
            filter.power
        ),
        (150.0, 10.0, 1000.0, 32.0)
    );
    let emitter = bank.processor("default3DProcessingType").unwrap();
    assert_eq!(
        (emitter.kind.as_str(), emitter.radius),
        ("emitter", Some(3.0))
    );
    let panner = bank.processor("panner").unwrap();
    assert_eq!(
        bank.resolve_curve(panner.range_curve.as_ref().unwrap()),
        Some(Curve::new([(0.0, 1.0), (1.0, 0.0)]))
    );
    assert!((bank.globals.old_configuration_volume_factor - 0.630_957_4).abs() < 1e-6);
}

#[test]
fn reads_old_style_sounds_music_sfx_and_env_sounds() {
    let bank = bank();
    let hint = bank.sound("hint").unwrap();
    assert_eq!(hint.path, r"\a3\ui_f\data\sound\hint");
    assert!((hint.volume - 0.891_250_9).abs() < 1e-6);
    let loud = bank.sound("Loud").unwrap();
    assert!((loud.volume - db_to_linear(10.0)).abs() < 1e-5);
    assert_eq!((loud.pitch, loud.distance), (1.2, Some(50.0)));
    assert_eq!(bank.music("track").unwrap().path, r"\music\track.ogg");

    let birds = bank.sfx("Birds").unwrap();
    assert_eq!(birds.sounds.len(), 1);
    assert_eq!(birds.sounds[0].probability, 0.4);
    assert_eq!(birds.sounds[0].delay, [2.0, 5.0, 10.0]);
    assert_eq!(birds.empty.as_ref().unwrap().probability, 0.6);

    let sea = bank.env_sound("sea").unwrap();
    assert_eq!(sea.volume_expr.as_ref().unwrap().variables(), ["coast"]);
    assert_eq!(bank.environment_sets, ["Sea_SoundSet"]);
}
