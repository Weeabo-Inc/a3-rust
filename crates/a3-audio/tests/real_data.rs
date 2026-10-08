//! Reads every sound class of the real game's merged config. Skipped when `A3_ROOT` is unset.

use std::path::Path;

use a3_audio::config::SoundBank;
use a3_gamedata::{GameData, LoadOptions};

#[test]
fn every_sound_class_of_the_game_loads() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = GameData::load(&LoadOptions::new(Path::new(&root))).unwrap();
    let bank = SoundBank::load(&data.config);
    let [
        shaders,
        sets,
        curves,
        filters,
        processors,
        sounds,
        music,
        sfx,
    ] = bank.counts();
    eprintln!(
        "{shaders} shaders, {sets} sets, {curves} curves, {filters} distance filters, \
         {processors} 3D processors, {sounds} CfgSounds, {music} CfgMusic, {sfx} CfgSFX; \
         {} environment sets; {} warnings",
        bank.environment_sets.len(),
        bank.warnings.len()
    );
    for w in bank.warnings.iter().take(20) {
        eprintln!("WARN {w}");
    }
    assert!(shaders > 3000 && sets > 2500, "expected the full game");
    assert!(bank.warnings.is_empty(), "every expression parses");

    // Every reference resolves.
    let mut missing = Vec::new();
    for set in bank.sets() {
        for shader in &set.shaders {
            if bank.shader(shader).is_none() {
                missing.push(format!("{}: shader {shader}", set.name));
            }
        }
        for name in [&set.distance_filter, &set.processing_type]
            .into_iter()
            .flatten()
        {
            if bank.distance_filter(name).is_none() && bank.processor(name).is_none() {
                missing.push(format!("{}: {name}", set.name));
            }
        }
        if let Some(curve) = &set.volume_curve
            && bank.resolve_curve(curve).is_none()
        {
            missing.push(format!("{}: volume curve {curve:?}", set.name));
        }
    }
    for shader in bank.shaders() {
        if let Some(curve) = &shader.range_curve
            && bank.resolve_curve(curve).is_none()
        {
            missing.push(format!("{}: range curve {curve:?}", shader.name));
        }
    }
    eprintln!("{} unresolved references", missing.len());
    for m in missing.iter().take(20) {
        eprintln!("MISSING {m}");
    }

    // Variables used by the expressions.
    let mut vars = std::collections::BTreeMap::new();
    for shader in bank.shaders() {
        for v in shader
            .volume
            .variables()
            .iter()
            .chain(shader.frequency.variables())
        {
            *vars.entry(v.clone()).or_insert(0) += 1;
        }
    }
    eprintln!("expression variables: {vars:?}");

    let sea = bank.set("Sea_SoundSet").unwrap();
    assert_eq!(sea.shaders, ["Sea_SoundShader"]);
    assert!(bank.environment_sets.iter().any(|s| s == "Coast_SoundSet"));
    assert!(bank.sound("refuel").is_some());
    assert!(bank.music("LeadTrack01_F").is_some());
}
