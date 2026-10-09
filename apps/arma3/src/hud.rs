//! The in-game HUD over `--play`: the `RscInGameUI` displays of [`a3_ingame_ui`] for the
//! player's loadout and stance, drawn over the frame by the UI render feature.

use std::sync::Arc;

use a3_config::ConfigTree;
use a3_fonts::Font;
use a3_ingame_ui::loadout::spawned_unit_info;
use a3_ingame_ui::{InGameUi, UnitInfo};
use a3_render::{Gpu, Renderer};
use a3_render_ui::UiFeature;
use a3_ui::{FontLoader, Fonts, Screen};
use a3_vfs::Vfs;

use crate::player::{Player, Stance};

/// `$STR_` lookups through the game's stringtables.
pub struct Strings(pub a3_stringtable::Localizer);

impl a3_gamedata::Localizer for Strings {
    fn localize(&self, key: &str) -> Option<String> {
        self.0.get(key).map(str::to_owned)
    }
}

/// Loads the English stringtables of the game data.
pub fn load_strings(vfs: &Vfs) -> Arc<dyn a3_gamedata::Localizer> {
    let (strings, report) = a3_stringtable::Localizer::load_vfs(vfs, a3_stringtable::ENGLISH);
    log::info!(
        "stringtables: {} keys from {} tables ({} failed)",
        strings.len(),
        report.tables.len(),
        report.failed.len()
    );
    Arc::new(Strings(strings))
}

/// FXY fonts read from the game files (`fonts[]` lists paths without the extension).
struct VfsFonts(Vfs);

impl FontLoader for VfsFonts {
    fn load_font(&mut self, path: &str) -> Option<Font> {
        let data = self
            .0
            .open(path)
            .ok()
            .or_else(|| self.0.open(&format!("{path}.fxy")).ok())?;
        Font::read(&data).ok()
    }
}

/// The HUD of the player.
pub struct Hud {
    igui: InGameUi,
    fonts: Fonts,
    feature: UiFeature,
    /// The player's weapons as his class spawns him.
    info: UnitInfo,
    time: f64,
}

impl Hud {
    /// The HUD of a player of `CfgVehicles >> class`, drawn through a UI feature added to
    /// `renderer`.
    pub fn new(
        gpu: &Gpu,
        renderer: &mut Renderer,
        config: Arc<ConfigTree>,
        vfs: Vfs,
        strings: Arc<dyn a3_gamedata::Localizer>,
        class: &str,
    ) -> Hud {
        let feature = UiFeature::new(gpu, renderer);
        feature.lock().set_assets(Box::new(vfs.clone()));
        renderer.add_feature(Box::new(feature.clone()));
        let info = spawned_unit_info(&config, class, &|key| strings.localize(key));
        let igui = InGameUi::new(
            Arc::clone(&config),
            Some(vfs.clone()),
            Some(strings),
            Screen::from_config(1280, 720, &config),
        );
        let fonts = Fonts::from_config(&config, Box::new(VfsFonts(vfs)));
        Hud {
            igui,
            fonts,
            feature,
            info,
            time: 0.0,
        }
    }

    /// Shows the live rounds of the selected weapon: `loaded` in its magazine (`None`: empty)
    /// and `spare` further magazines.
    pub fn set_rounds(&mut self, loaded: Option<u32>, spare: u32) {
        if let Some(weapon) = &mut self.info.weapon {
            match (loaded, &mut weapon.loaded) {
                (Some(ammo), Some(m)) => m.ammo = ammo,
                (Some(ammo), slot @ None) => {
                    *slot = Some(a3_ingame_ui::LoadedMagazine {
                        ammo,
                        ..Default::default()
                    })
                }
                (None, slot) => *slot = None,
            }
            weapon.magazines = spare;
        }
    }

    /// Updates the HUD for `player` on a `width` x `height` output and hands its quads to the
    /// UI feature.
    pub fn frame(&mut self, player: &Player, (width, height): (u32, u32), dt: f64) {
        self.time += dt;
        let screen = Screen {
            width,
            height,
            ..self.igui.ui().metrics.screen
        };
        self.igui.set_screen(screen);
        self.info.stance = match player.stance {
            Stance::Stand => a3_ingame_ui::Stance::Stand,
            Stance::Crouch => a3_ingame_ui::Stance::Crouch,
            Stance::Prone => a3_ingame_ui::Stance::Prone,
        };
        self.igui.update(&self.info, self.time);
        let list = self.igui.draw(&mut self.fonts);
        self.feature.lock().set_draw_list(list);
    }
}
