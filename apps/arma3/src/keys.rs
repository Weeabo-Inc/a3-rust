//! Keybindings for the client: built-in defaults, overlaid with a `CfgDefaultKeysPresets` preset
//! from the game's merged config and the player's profile.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use a3_config::{ConfigTree, is_rap, parse_text, read_rap};
use a3_input::{ActionMap, actions, default_preset};
use anyhow::Context as _;

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
    let description = if parts.is_empty() {
        "BUILT-IN".to_owned()
    } else {
        parts.join(" + ")
    };
    Ok(Keybindings { map, description })
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
}
