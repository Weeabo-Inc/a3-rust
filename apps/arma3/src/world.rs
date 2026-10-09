//! Loading a World's terrain from the game data: `CfgWorlds >> name`, its WRP and its layer
//! materials.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use a3_gamedata::{GameData, LoadOptions};
use a3_landscape::{TerrainLayers, WorldConfig};
use a3_landscape_render::landscape::TerrainShading;
use a3_landscape_render::{FileReader, Landscape};
use a3_wrp::Terrain;
use anyhow::Context as _;
use glam::DVec3;

use crate::models::{ObjectOptions, WorldObjects};

/// A terrain ready for the renderer.
pub struct LoadedWorld {
    /// The CfgWorlds class name.
    pub name: String,
    pub landscape: Landscape,
    /// VFS access for streaming satellite tiles.
    pub reader: FileReader,
    /// `centerPosition` in world space (x east, z north; y is 0).
    pub centre: DVec3,
    /// The placed objects, when they should be drawn.
    pub objects: Option<WorldObjects>,
    /// The roads, when the World has a readable roads shapefile.
    pub roads: Option<crate::roads::LoadedRoads>,
    /// Date, sun, lighting tables, weather and fog of the World.
    pub environment: a3_environment::WorldEnvironment,
    /// The cloud noise texture (`SimulWeather >> noiseTexture`), decoded.
    pub sky_noise: Option<a3_render::TextureData>,
}

/// Mount the game at `game_dir`, find `CfgWorlds >> world` and load its terrain, and its
/// placed objects when `objects` is given.
pub fn load(
    game_dir: &Path,
    world: &str,
    objects: Option<ObjectOptions>,
) -> anyhow::Result<LoadedWorld> {
    let start = Instant::now();
    let data = GameData::load(&LoadOptions::new(game_dir))
        .with_context(|| format!("cannot load the game in {}", game_dir.display()))?;
    let mounted = start.elapsed();
    let config = WorldConfig::load(&data.config, world)
        .with_context(|| format!("no usable world `{world}` in CfgWorlds"))?;
    let wrp = config.wrp.as_str();
    let bytes = data
        .vfs
        .open(wrp)
        .with_context(|| format!("cannot open {wrp}"))?;
    let terrain = Terrain::parse(&bytes).with_context(|| format!("cannot parse {wrp}"))?;
    drop(bytes);
    let objects =
        objects.map(|options| WorldObjects::from_terrain(&terrain, data.vfs.clone(), options));
    let parsed = start.elapsed();
    let layers = TerrainLayers::load(&data.vfs, &terrain);
    if !layers.errors.is_empty() {
        log::warn!(
            "{} layer materials failed to load, first: {}",
            layers.errors.len(),
            layers.errors[0]
        );
    }
    let roads = match crate::roads::load(&data.vfs, &config, &terrain) {
        Ok(roads) => Some(roads),
        Err(e) => {
            log::warn!("no roads: {e:#}");
            None
        }
    };
    let vfs = data.vfs.clone();
    let mut landscape = Landscape::from_terrain(&terrain, &layers, |p| vfs.open(p.as_str()).ok());
    let class = data.config.root().get("CfgWorlds").get(&config.class);
    let defaults = TerrainShading::default();
    let positive = |value: f32, default: f32| if value > 0.0 { value } else { default };
    landscape.shading = TerrainShading {
        full_detail_dist: positive(config.full_detail_dist, defaults.full_detail_dist),
        no_detail_dist: positive(config.no_detail_dist, defaults.no_detail_dist),
        max_darken: positive(
            class.get("terrainBlendMaxDarkenCoef").number(),
            defaults.max_darken,
        ),
        max_brighten: positive(
            class.get("terrainBlendMaxBrightenCoef").number(),
            defaults.max_brighten,
        ),
    };
    let centre = DVec3::new(
        f64::from(config.center_position.x),
        0.0,
        f64::from(config.center_position.y),
    );
    log::info!(
        "world {}: {wrp}, {} m, {}x{} heights; game data {:.2?}, terrain {:.2?}, layers and landscape {:.2?}",
        config.class,
        terrain.world_size(),
        terrain.heightmap.width(),
        terrain.heightmap.height(),
        mounted,
        parsed - mounted,
        start.elapsed() - parsed
    );
    let environment = a3_environment::WorldEnvironment::from_config(&class);
    let noise_path = class.get("SimulWeather").get("noiseTexture");
    let sky_noise = if noise_path.is_text() {
        decode_paa_rgba8(&data.vfs, &noise_path.text())
    } else {
        None
    };
    let reader: FileReader = Arc::new(move |p| vfs.open(p.as_str()).ok().map(|b| b.to_vec()));
    Ok(LoadedWorld {
        name: config.class,
        landscape,
        reader,
        centre,
        objects,
        roads,
        environment,
        sky_noise,
    })
}

/// Decodes a PAA with all its mipmaps to RGBA8.
fn decode_paa_rgba8(vfs: &a3_vfs::Vfs, path: &str) -> Option<a3_render::TextureData> {
    let bytes = vfs.open(path).ok()?;
    let texture = a3_paa::Texture::read(&bytes)
        .map_err(|e| log::warn!("cannot read {path}: {e}"))
        .ok()?;
    let mips = texture
        .mips
        .iter()
        .map(|m| a3_paa::decode_rgba8(texture.format, m))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(a3_render::TextureData {
        format: a3_render::TextureFormat::Rgba8,
        width: u32::from(texture.width()),
        height: u32::from(texture.height()),
        mips,
    })
}

/// A camera placement given on the command line: `east,north,altitude,heading,pitch` with
/// the altitude in metres above the terrain and the angles in degrees (heading clockwise from
/// north, pitch positive up). Trailing fields may be left out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraSpec {
    pub east: f64,
    pub north: f64,
    pub altitude: f64,
    pub heading: f32,
    pub pitch: f32,
}

impl std::str::FromStr for CameraSpec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let values = s
            .split(',')
            .map(|v| v.trim().parse::<f64>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("expected numbers east,north[,altitude,heading,pitch]: {e}"))?;
        if !(2..=5).contains(&values.len()) {
            return Err("expected 2 to 5 values: east,north[,altitude,heading,pitch]".into());
        }
        let at = |i: usize, default: f64| values.get(i).copied().unwrap_or(default);
        Ok(CameraSpec {
            east: values[0],
            north: values[1],
            altitude: at(2, 300.0),
            heading: at(3, 0.0) as f32,
            pitch: at(4, -15.0) as f32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_spec_parses_full_and_short_forms() {
        let spec: CameraSpec = "3600, 13000, 150, 45, -20".parse().unwrap();
        assert_eq!(
            spec,
            CameraSpec {
                east: 3600.0,
                north: 13000.0,
                altitude: 150.0,
                heading: 45.0,
                pitch: -20.0
            }
        );
        let short: CameraSpec = "100,200".parse().unwrap();
        assert_eq!(
            (short.altitude, short.heading, short.pitch),
            (300.0, 0.0, -15.0)
        );
        assert!("1".parse::<CameraSpec>().is_err());
        assert!("a,b".parse::<CameraSpec>().is_err());
    }
}
