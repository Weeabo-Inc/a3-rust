//! Keybindings for the client: built-in defaults, overlaid with a `CfgDefaultKeysPresets` preset
//! from the game's merged config and the player's profile.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use a3_config::{ConfigTree, is_rap, parse_text, read_rap};
use a3_input::{ActionMap, Binding, Dik, InputCode, actions, default_preset};
use anyhow::Context as _;

use crate::player;

/// Where the bindings come from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeySources {
    /// Game install to read `CfgDefaultKeysPresets` from.
    pub game_dir: Option<PathBuf>,
    /// Preset class name; the config's `default = 1` preset when `None`.
    pub preset: Option<String>,
    /// `.Arma3Profile` file with the player's `key<Action>[]` overrides.
    pub profile: Option<PathBuf>,
}

impl KeySources {
    /// Whether anything beyond the built-in defaults was requested.
    pub fn is_empty(&self) -> bool {
        self.game_dir.is_none() && self.profile.is_none()
    }
}

/// Loaded bindings and a short description for the overlay.
#[derive(Debug)]
pub struct Keybindings {
    pub map: ActionMap,
    pub description: String,
}

/// Build the action map from `sources` (blocking: loading the game config takes seconds).
pub fn load(sources: &KeySources) -> anyhow::Result<Keybindings> {
    let mut map = actions::default_map();
    let mut parts = Vec::new();
    if let Some(dir) = &sources.game_dir {
        let data = a3_gamedata::GameData::load(&a3_gamedata::LoadOptions::new(dir))
            .with_context(|| format!("loading game data from {}", dir.display()))?;
        let root = data.config.root();
        let preset = match &sources.preset {
            Some(p) => p.clone(),
            None => default_preset(&root).context("config has no default key preset")?,
        };
        let issues = map.load_preset(&root, &preset)?;
        for issue in &issues {
            log::warn!(
                "key preset {preset}, action {}: {}",
                issue.action,
                issue.error
            );
        }
        parts.push(preset);
    }
    if let Some(path) = &sources.profile {
        let profile = read_profile(path)?;
        let issues = map.apply_profile(&profile.root());
        for issue in &issues {
            log::warn!("profile action {}: {}", issue.action, issue.error);
        }
        parts.push("PROFILE".to_owned());
    }
    ensure_client_defaults(&mut map);
    separate_movement_from_turn(&mut map);
    let description = if parts.is_empty() {
        "BUILT-IN".to_owned()
    } else {
        parts.join(" + ")
    };
    Ok(Keybindings { map, description })
}

/// The built-in bindings with the client's own fallback keys already applied: what the client
/// runs on before (or without) the game's key presets.
pub fn client_default_map() -> ActionMap {
    let mut map = actions::default_map();
    ensure_client_defaults(&mut map);
    separate_movement_from_turn(&mut map);
    map
}

/// The client's own policy on top of the game's preset and the player's profile: a key that
/// moves the Man never also turns him, and the mouse never turns him through the action map
/// (the aim reads it directly, so a relative input here would turn him twice per movement).
///
/// The game's own presets bind A and D to `turnLeft`/`turnRight` as well as to `moveLeft`/
/// `moveRight` (issue #228), which both spins and strafes. Turning stays available on whatever
/// the map binds `turnLeft`/`turnRight` to besides movement — the arrow keys in the built-in
/// map. The game itself would leave this to the Action map's own filters; the client has no
/// Action map yet, so it states the rule once here rather than carrying it into the movement.
pub fn separate_movement_from_turn(map: &mut ActionMap) {
    let movement: Vec<InputCode> = [
        actions::MOVE_FORWARD,
        actions::MOVE_BACK,
        actions::MOVE_LEFT,
        actions::MOVE_RIGHT,
    ]
    .iter()
    .flat_map(|action| map.bindings(action).iter().map(|b| b.input))
    .collect();
    for action in [actions::TURN_LEFT, actions::TURN_RIGHT] {
        let mut turn = map.bindings(action).to_vec();
        let before = turn.len();
        turn.retain(|b| !movement.contains(&b.input) && !b.input.is_relative());
        if turn.len() != before {
            map.set_bindings(action, turn);
        }
    }
}

/// Keys the client supplies for actions this build needs but the shipped presets leave empty:
/// `Arma3Apex`/`Arma3` bind `personView` (Numpad Enter) and `turbo` (LShift) but ship
/// `crouch`, `prone` and `stand` as empty arrays, and their walk modifier is a Ctrl+W combo.
/// Filling only empty actions keeps the game's own keys whenever it has any.
pub fn ensure_client_defaults(map: &mut ActionMap) {
    let fallbacks = [
        (player::PERSON_VIEW, Dik::NUMPADENTER),
        (player::TURBO, Dik::LSHIFT),
        (player::WALK, Dik::LCONTROL),
        (player::CROUCH, Dik::C),
        (player::PRONE, Dik::Z),
        (player::STAND, Dik::X),
        (player::NEXT_WEAPON, Dik::F),
        (actions::RELOAD_MAGAZINE, Dik::R),
    ];
    for (action, key) in fallbacks {
        if map.bindings(action).is_empty() {
            map.bind(action, Binding::key(key));
        }
    }
}

/// Load on a background thread; the receiver yields once.
pub fn load_in_background(sources: KeySources) -> Receiver<anyhow::Result<Keybindings>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("keybindings".to_owned())
        .spawn(move || {
            let _ = tx.send(load(&sources));
        })
        .expect("spawning the keybinding loader");
    rx
}

/// Parse a profile file: config text, or rapified config.
pub fn read_profile(path: &Path) -> anyhow::Result<ConfigTree> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading profile {}", path.display()))?;
    let config = if is_rap(&bytes) {
        read_rap(&bytes)?
    } else {
        parse_text(&String::from_utf8_lossy(&bytes))
            .with_context(|| format!("parsing profile {}", path.display()))?
    };
    Ok(ConfigTree::from_config(&config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_input::{Binding, Dik};

    #[test]
    fn profile_alone_overlays_the_built_in_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Test.Arma3Profile");
        std::fs::write(&path, "version=1;\nkeyCameraMoveUp[]={18};\n").unwrap();
        let loaded = load(&KeySources {
            profile: Some(path),
            ..KeySources::default()
        })
        .unwrap();
        assert_eq!(loaded.description, "PROFILE");
        assert!(
            loaded
                .map
                .bindings(actions::CAMERA_MOVE_UP)
                .contains(&Binding::key(Dik::E))
        );
        assert!(
            !loaded
                .map
                .bindings(actions::CAMERA_MOVE_UP)
                .contains(&Binding::key(Dik::Q))
        );
    }

    #[test]
    fn nothing_requested_means_built_in() {
        let loaded = load(&KeySources::default()).unwrap();
        assert_eq!(loaded.description, "BUILT-IN");
        assert!(KeySources::default().is_empty());
    }

    #[test]
    fn a_key_that_moves_the_man_does_not_also_turn_him() {
        // The shipped presets turn with A and D as well as strafing with them (issue #228).
        let mut map = actions::default_map();
        map.set_bindings(
            actions::TURN_LEFT,
            vec![Binding::key(Dik::A), Binding::key(Dik::LEFT)],
        );
        map.set_bindings(
            actions::TURN_RIGHT,
            vec![Binding::key(Dik::D), Binding::key(Dik::RIGHT)],
        );
        separate_movement_from_turn(&mut map);
        let turn = |action| {
            map.bindings(action)
                .iter()
                .map(|b| b.input)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            turn(actions::TURN_LEFT),
            vec![InputCode::Key(Dik::LEFT)],
            "the arrow turns, the strafe key does not"
        );
        assert_eq!(turn(actions::TURN_RIGHT), vec![InputCode::Key(Dik::RIGHT)]);
        assert_eq!(
            map.bindings(actions::MOVE_LEFT).len(),
            1,
            "and A still strafes"
        );
    }

    #[test]
    fn the_mouse_aims_without_also_turning_through_the_action_map() {
        // `--play` reads the mouse for the aim itself; a relative input on a turn action would
        // turn the Man a second time for the same movement.
        let map = client_default_map();
        for action in [actions::TURN_LEFT, actions::TURN_RIGHT] {
            assert!(
                map.bindings(action).iter().all(|b| !b.input.is_relative()),
                "{action} still takes the mouse"
            );
        }
    }

    #[test]
    fn a_profile_cannot_make_a_key_that_moves_the_man_turn_him() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Test.Arma3Profile");
        std::fs::write(
            &path,
            "version=1;\nkeyTurnLeft[]={30};\nkeyTurnRight[]={32};\n",
        )
        .unwrap();
        let loaded = load(&KeySources {
            profile: Some(path),
            ..KeySources::default()
        })
        .unwrap();
        assert!(
            loaded
                .map
                .bindings(actions::TURN_LEFT)
                .iter()
                .all(|b| b.input != InputCode::Key(Dik::A)),
            "A strafes, so it does not turn"
        );
        assert!(
            loaded
                .map
                .bindings(actions::MOVE_LEFT)
                .iter()
                .any(|b| b.input == InputCode::Key(Dik::A)),
            "and the strafe is untouched"
        );
    }
}
