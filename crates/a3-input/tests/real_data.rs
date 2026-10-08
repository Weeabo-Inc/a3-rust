//! Keybinding presets of the real game. Needs `A3_ROOT`; skipped otherwise.

use a3_gamedata::{GameData, LoadOptions};
use a3_input::{ActionMap, Binding, Dik, InputCode, Trigger, default_preset, presets};

#[test]
fn every_shipped_preset_loads_and_the_default_has_the_usual_keys() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = GameData::load(&LoadOptions::new(root)).expect("game loads");
    let config = data.config.root();

    let all = presets(&config);
    assert!(all.len() >= 5, "{all:?}");
    for preset in &all {
        let mut map = ActionMap::new();
        let issues = map
            .load_preset(&config, &preset.name)
            .expect("preset loads");
        assert!(issues.is_empty(), "{}: {issues:?}", preset.name);
        assert!(map.iter().count() > 100, "{} has few actions", preset.name);
    }

    let default = default_preset(&config).expect("a default preset");
    assert_eq!(default, "Arma3Apex");
    let mut map = ActionMap::new();
    map.load_preset(&config, &default).unwrap();
    assert!(map.bindings("moveForward").contains(&Binding::key(Dik::W)));
    assert!(
        map.bindings("cameraMoveForward")
            .contains(&Binding::key(Dik::W))
    );
    assert_eq!(map.bindings("ingamePause"), &[Binding::key(Dik::ESCAPE)]);
    assert!(
        map.bindings("defaultAction")
            .contains(&Binding::press(InputCode::MouseButton(0)))
    );
    assert!(
        map.bindings("compassToggle")
            .iter()
            .any(|b| b.trigger == Trigger::DoubleTap)
    );
}
