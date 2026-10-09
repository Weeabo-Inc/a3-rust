//! A world's config: `CfgWorlds >> <world>`.

use a3_config::{ConfigRef, ConfigTree};
use a3_core::VfsPath;
use glam::{Vec2, Vec3};

use crate::Error;
use crate::cfg::{
    array_n, classes, expr_number, number, number_or, numbers, text, text_or_empty, texts,
};

/// A step level of the map grid (`class Grid >> ZoomN`).
#[derive(Debug, Clone, PartialEq)]
pub struct GridZoom {
    /// Used up to this map zoom.
    pub zoom_max: f32,
    /// Label format, e.g. `"XY"`.
    pub format: String,
    /// Digits of the x label, e.g. `"000"`.
    pub format_x: String,
    /// Digits of the y label.
    pub format_y: String,
    /// Grid spacing along x in metres.
    pub step_x: f32,
    /// Grid spacing along y in metres (negative: labels count down from `offset_y`).
    pub step_y: f32,
}

/// The map grid (grid references) of a world.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapGrid {
    /// World x of grid origin.
    pub offset_x: f32,
    /// World z of grid origin.
    pub offset_y: f32,
    /// Zoom levels, finest first.
    pub zooms: Vec<GridZoom>,
}

/// One surface layer drawn beyond the terrain edge.
#[derive(Debug, Clone, PartialEq)]
pub struct OutsideLayer {
    /// Class name.
    pub class: String,
    /// `_nopx` normal/parallax map.
    pub normal: String,
    /// `_co` detail texture.
    pub texture: String,
}

/// How the area beyond the terrain edge looks (`class OutsideTerrain`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OutsideTerrain {
    /// Satellite texture used outside.
    pub satellite: String,
    /// `enableTerrainSynth`: generate terrain outside the heightmap.
    pub enable_terrain_synth: bool,
    /// Layers used outside.
    pub layers: Vec<OutsideLayer>,
    /// `colorOutside` RGBA.
    pub color: [f32; 4],
}

/// One clutter model (`class clutter >> X`), referenced by surface characters.
#[derive(Debug, Clone, PartialEq)]
pub struct ClutterModel {
    /// Class name, as listed in `CfgSurfaceCharacters >> names`.
    pub class: String,
    /// Model path.
    pub model: String,
    /// `affectedByWind`.
    pub affected_by_wind: f32,
    /// `swLighting`.
    pub sw_lighting: bool,
    /// `scaleMin`.
    pub scale_min: f32,
    /// `scaleMax`.
    pub scale_max: f32,
    /// `relativeColor` RGBA.
    pub relative_color: [f32; 4],
}

/// One environment map (`class EnvMaps >> X`).
#[derive(Debug, Clone, PartialEq)]
pub struct EnvMap {
    /// Texture.
    pub texture: String,
    /// The overcast level it is used from.
    pub overcast: f32,
}

/// Sky and celestial objects.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sky {
    /// `skyObject` model.
    pub sky_object: String,
    /// `horizontObject` model.
    pub horizon_object: String,
    /// `skyTexture`.
    pub sky_texture: String,
    /// `skyTextureR` (reflection).
    pub sky_texture_r: String,
    /// `starsObject`, `sunObject`, `moonObject`, `haloObject`, `rainbowObject`, `pointObject`.
    pub stars_object: String,
    /// Sun model.
    pub sun_object: String,
    /// Moon model.
    pub moon_object: String,
    /// Sun halo model.
    pub halo_object: String,
    /// Rainbow model.
    pub rainbow_object: String,
    /// Point (star) model.
    pub point_object: String,
    /// Cloud models.
    pub clouds: Vec<String>,
    /// Environment maps by overcast.
    pub env_maps: Vec<EnvMap>,
    /// Names of the world's lighting classes (`Lighting`, `DayLighting*`, ...), to look up in
    /// the config.
    pub lighting_classes: Vec<String>,
}

/// Sea and water settings (`class Sea`, `class Underwater` and top-level water entries).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sea {
    /// `Sea >> seaTexture`.
    pub sea_texture: String,
    /// `Sea >> seaMaterial` (e.g. `#water`).
    pub sea_material: String,
    /// `Sea >> shoreMaterial`.
    pub shore_material: String,
    /// `Sea >> WaterMapScale`.
    pub water_map_scale: f32,
    /// `Sea >> WaterGrid`.
    pub water_grid: f32,
    /// `Sea >> MaxTide`.
    pub max_tide: f32,
    /// `Sea >> MaxWave`.
    pub max_wave: f32,
    /// `Underwater >> waterColor` RGB.
    pub water_color: Vec3,
    /// `Underwater >> deepWaterColor` RGB.
    pub deep_water_color: Vec3,
    /// `Underwater >> waterFogDistance`.
    pub water_fog_distance: f32,
    /// `waterTexture`.
    pub water_texture: String,
    /// `seaBedUnderwaterDepth`.
    pub sea_bed_underwater_depth: f32,
    /// `shoreTop`.
    pub shore_top: f32,
    /// `peakWaveTop`.
    pub peak_wave_top: f32,
    /// `peakWaveBottom`.
    pub peak_wave_bottom: f32,
    /// The wave entries of `class Sea`, with the engine's defaults for missing ones.
    pub waves: SeaWaves,
    /// `class WaterExPars`: the sea shader's parameters.
    pub water_ex: WaterExPars,
}

/// The wave entries of `CfgWorlds >> <world> >> Sea` (`docs/re/render-water.md` §1). Missing
/// entries take the engine's defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeaWaves {
    /// `WaterMapScale`.
    pub water_map_scale: f32,
    /// `WaterGrid`: water cell edge in metres.
    pub water_grid: f32,
    /// `MaxTide`.
    pub max_tide: f32,
    /// `MaxWave`.
    pub max_wave: f32,
    /// `SeaWaveXScale`: radial wave frequency, cycles per metre.
    pub x_scale: f32,
    /// `SeaWaveZScale`: angular wave frequency, cycles per water cell of arc.
    pub z_scale: f32,
    /// `SeaWaveHScale`: wave height scale.
    pub h_scale: f32,
    /// `SeaWaveXDuration`: radial wave period in milliseconds.
    pub x_duration_ms: i32,
    /// `SeaWaveZDuration`: angular wave period in milliseconds.
    pub z_duration_ms: i32,
}

impl Default for SeaWaves {
    /// The engine's defaults (loader `0x141648a40`).
    fn default() -> Self {
        SeaWaves {
            water_map_scale: 20.0,
            water_grid: 50.0,
            max_tide: 1.5,
            max_wave: 0.25,
            x_scale: 0.04,
            z_scale: 0.02,
            h_scale: 1.0,
            x_duration_ms: 5000,
            z_duration_ms: 10000,
        }
    }
}

/// `CfgWorlds >> <world> >> WaterExPars` (`docs/re/render-water.md` §1). `None` marks an
/// entry the world does not set; the engine then passes 0 to the shaders.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WaterExPars {
    pub fog_density: Option<f32>,
    pub fog_color: Option<Vec3>,
    pub fog_color_extinction_speed: Option<Vec3>,
    pub light_extinction_speed: Option<Vec3>,
    pub diffuse_light_extinction_speed: Option<Vec3>,
    pub fog_gradient_coefs: Option<Vec3>,
    pub fog_color_light_influence: Option<Vec3>,
    pub ss_reflection_strength: Option<f32>,
    pub ss_reflection_max_jitter: Option<f32>,
    pub ss_reflection_ripple_influence: Option<f32>,
    pub ss_reflection_edge_fading_coef: Option<f32>,
    pub ss_reflection_dist_fading_coef: Option<f32>,
    pub specular_max_intensity: Option<f32>,
    pub specular_power_overcast0: Option<f32>,
    pub specular_power_overcast1: Option<f32>,
    pub specular_normal_modify_coef: Option<f32>,
    pub refraction_min_coef: Option<f32>,
    pub refraction_max_coef: Option<f32>,
    pub refraction_max_dist: Option<f32>,
    pub surface_opacity: Option<f32>,
    pub shadow_intensity: Option<f32>,
    pub foam_around_objects_intensity: Option<f32>,
    pub foam_around_objects_fade_coef: Option<f32>,
    pub foam_color_coef: Option<f32>,
    pub foam_deformation_coef: Option<f32>,
    pub foam_texture_coef: Option<f32>,
    pub foam_time_move_speed: Option<f32>,
    pub foam_time_move_amount: Option<f32>,
}

/// One ambient life species (`AmbientA3 >> RadiusX >> Species >> Y`).
#[derive(Debug, Clone, PartialEq)]
pub struct AmbientSpecies {
    /// The CfgVehicles class spawned.
    pub class: String,
    /// `maxCircleCount` expression.
    pub max_circle_count: String,
    /// `maxWorldCount`.
    pub max_world_count: f32,
    /// `cost`.
    pub cost: f32,
}

/// One ambient life spawn ring (`AmbientA3 >> RadiusX`).
#[derive(Debug, Clone, PartialEq)]
pub struct AmbientRadius {
    /// Class name.
    pub class: String,
    /// `areaSpawnRadius`.
    pub area_spawn_radius: f32,
    /// `areaMaxRadius`.
    pub area_max_radius: f32,
    /// `spawnCircleRadius`.
    pub spawn_circle_radius: f32,
    /// `spawnInterval` seconds.
    pub spawn_interval: f32,
    /// Species spawned.
    pub species: Vec<AmbientSpecies>,
}

/// A named location (`class Names >> X`).
#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    /// Class name.
    pub class: String,
    /// Display name (often a `$STR_` key).
    pub name: String,
    /// Type, e.g. `NameVillage`, `NameCity`, `Hill`.
    pub kind: String,
    /// World `(x, z)`.
    pub position: Vec2,
    /// `radiusA`.
    pub radius_a: f32,
    /// `radiusB`.
    pub radius_b: f32,
    /// `angle` in degrees.
    pub angle: f32,
}

/// The settings of one world (`CfgWorlds >> <class>`), inheritance resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldConfig {
    /// The class name, e.g. `Altis`.
    pub class: String,
    /// The terrain file (`worldName`).
    pub wrp: VfsPath,
    /// `description` (often a `$STR_` key).
    pub description: String,
    /// `author`.
    pub author: String,
    /// `mapSize` in metres.
    pub map_size: f32,
    /// `centerPosition` (x, z, y-ish third value as written).
    pub center_position: Vec3,
    /// `longitude`.
    pub longitude: f32,
    /// `latitude`.
    pub latitude: f32,
    /// `elevationOffset`.
    pub elevation_offset: f32,
    /// `startTime` text, e.g. `"12:00"`.
    pub start_time: String,
    /// `startDate` text, e.g. `"24/6/2035"`.
    pub start_date: String,
    /// `soundMapSizeCoef`: sound map cells per land cell along each axis.
    pub sound_map_size_coef: u32,
    /// `newRoadsShape`: the roads shapefile, if any.
    pub roads_shape: Option<VfsPath>,
    /// `pictureMap`.
    pub picture_map: String,
    /// `class Grid`.
    pub grid: MapGrid,
    /// `class OutsideTerrain`.
    pub outside_terrain: OutsideTerrain,
    /// `outsideHeight`.
    pub outside_height: f32,
    /// `minHeight`.
    pub min_height: f32,
    /// `satelliteNormalBlendStart`.
    pub satellite_normal_blend_start: f32,
    /// `satelliteNormalBlendEnd`.
    pub satellite_normal_blend_end: f32,
    /// `midDetailTexture`.
    pub mid_detail_texture: String,
    /// `clutterGrid`: clutter spacing in metres.
    pub clutter_grid: f32,
    /// `clutterDist`.
    pub clutter_dist: f32,
    /// `noDetailDist`.
    pub no_detail_dist: f32,
    /// `fullDetailDist`.
    pub full_detail_dist: f32,
    /// `class clutter`.
    pub clutter: Vec<ClutterModel>,
    /// Sky objects, textures and lighting class names.
    pub sky: Sky,
    /// Sea and water.
    pub sea: Sea,
    /// `class AmbientA3`.
    pub ambient: Vec<AmbientRadius>,
    /// `class Names`.
    pub locations: Vec<Location>,
}

/// The `CfgWorlds` classes that are worlds (have a `worldName`), in config order.
pub fn world_classes(config: &ConfigTree) -> Vec<String> {
    classes(&config.root().get("CfgWorlds"))
        .iter()
        .filter(|c| text(c, "worldName").is_some_and(|w| !w.is_empty()))
        .map(|c| c.name().to_owned())
        .collect()
}

impl WorldConfig {
    /// Reads `CfgWorlds >> class`.
    pub fn load(config: &ConfigTree, class: &str) -> Result<Self, Error> {
        let c = config.root().get("CfgWorlds").get(class);
        if !c.is_class() {
            return Err(Error::NoWorld(class.to_owned()));
        }
        Ok(Self::from_config(&c))
    }

    /// Reads a world class.
    pub fn from_config(c: &ConfigRef<'_>) -> Self {
        let roads = text_or_empty(c, "newRoadsShape");
        Self {
            class: c.name().to_owned(),
            wrp: VfsPath::new(&text_or_empty(c, "worldName")),
            description: text_or_empty(c, "description"),
            author: text_or_empty(c, "author"),
            map_size: number_or(c, "mapSize", 0.0),
            center_position: Vec3::from_array(array_n::<3>(c, "centerPosition", 0.0)),
            longitude: number_or(c, "longitude", 0.0),
            latitude: number_or(c, "latitude", 0.0),
            elevation_offset: number_or(c, "elevationOffset", 0.0),
            start_time: text_or_empty(c, "startTime"),
            start_date: text_or_empty(c, "startDate"),
            sound_map_size_coef: number_or(c, "soundMapSizeCoef", 1.0).max(1.0) as u32,
            roads_shape: (!roads.is_empty()).then(|| VfsPath::new(&roads)),
            picture_map: text_or_empty(c, "pictureMap"),
            grid: read_grid(&c.get("Grid")),
            outside_terrain: read_outside(&c.get("OutsideTerrain")),
            outside_height: number_or(c, "outsideHeight", 0.0),
            min_height: number_or(c, "minHeight", 0.0),
            satellite_normal_blend_start: number_or(c, "satelliteNormalBlendStart", 0.0),
            satellite_normal_blend_end: number_or(c, "satelliteNormalBlendEnd", 0.0),
            mid_detail_texture: text_or_empty(c, "midDetailTexture"),
            clutter_grid: number_or(c, "clutterGrid", 0.0),
            clutter_dist: number_or(c, "clutterDist", 0.0),
            no_detail_dist: number_or(c, "noDetailDist", 0.0),
            full_detail_dist: number_or(c, "fullDetailDist", 0.0),
            clutter: read_clutter(c),
            sky: read_sky(c),
            sea: read_sea(c),
            ambient: read_ambient(&c.get("AmbientA3")),
            locations: classes(&c.get("Names")).iter().map(read_location).collect(),
        }
    }

    /// The clutter model named `class` (case-insensitive).
    pub fn clutter_model(&self, class: &str) -> Option<&ClutterModel> {
        self.clutter
            .iter()
            .find(|m| m.class.eq_ignore_ascii_case(class))
    }
}

fn read_grid(g: &ConfigRef<'_>) -> MapGrid {
    MapGrid {
        offset_x: number_or(g, "offsetX", 0.0),
        offset_y: number_or(g, "offsetY", 0.0),
        zooms: classes(g)
            .iter()
            .map(|z| GridZoom {
                zoom_max: number_or(z, "zoomMax", 0.0),
                format: text_or_empty(z, "format"),
                format_x: text_or_empty(z, "formatX"),
                format_y: text_or_empty(z, "formatY"),
                step_x: number_or(z, "stepX", 0.0),
                step_y: number_or(z, "stepY", 0.0),
            })
            .collect(),
    }
}

fn read_outside(o: &ConfigRef<'_>) -> OutsideTerrain {
    OutsideTerrain {
        satellite: text_or_empty(o, "satellite"),
        enable_terrain_synth: number_or(o, "enableTerrainSynth", 0.0) != 0.0,
        layers: classes(&o.get("Layers"))
            .iter()
            .map(|l| OutsideLayer {
                class: l.name().to_owned(),
                normal: text_or_empty(l, "nopx"),
                texture: text_or_empty(l, "texture"),
            })
            .collect(),
        color: array_n::<4>(o, "colorOutside", 0.0),
    }
}

fn read_clutter(c: &ConfigRef<'_>) -> Vec<ClutterModel> {
    let defaults = c.get("DefaultClutter");
    let num = |m: &ConfigRef<'_>, name: &str, default: f32| {
        number(m, name)
            .or_else(|| number(&defaults, name))
            .unwrap_or(default)
    };
    classes(&c.get("clutter"))
        .iter()
        .map(|m| {
            let color = if m.get("relativeColor").is_array() {
                array_n::<4>(m, "relativeColor", 1.0)
            } else {
                array_n::<4>(&defaults, "relativeColor", 1.0)
            };
            ClutterModel {
                class: m.name().to_owned(),
                model: text_or_empty(m, "model"),
                affected_by_wind: num(m, "affectedByWind", 0.0),
                sw_lighting: num(m, "swLighting", 0.0) != 0.0,
                scale_min: num(m, "scaleMin", 1.0),
                scale_max: num(m, "scaleMax", 1.0),
                relative_color: color,
            }
        })
        .collect()
}

fn read_sky(c: &ConfigRef<'_>) -> Sky {
    Sky {
        sky_object: text_or_empty(c, "skyObject"),
        horizon_object: text_or_empty(c, "horizontObject"),
        sky_texture: text_or_empty(c, "skyTexture"),
        sky_texture_r: text_or_empty(c, "skyTextureR"),
        stars_object: text_or_empty(c, "starsObject"),
        sun_object: text_or_empty(c, "sunObject"),
        moon_object: text_or_empty(c, "moonObject"),
        halo_object: text_or_empty(c, "haloObject"),
        rainbow_object: text_or_empty(c, "rainbowObject"),
        point_object: text_or_empty(c, "pointObject"),
        clouds: texts(c, "clouds"),
        env_maps: classes(&c.get("EnvMaps"))
            .iter()
            .map(|e| EnvMap {
                texture: text_or_empty(e, "texture"),
                overcast: number_or(e, "overcast", 0.0),
            })
            .collect(),
        lighting_classes: classes(c)
            .iter()
            .map(|k| k.name().to_owned())
            .filter(|n| n.to_ascii_lowercase().contains("lighting"))
            .collect(),
    }
}

fn read_sea(c: &ConfigRef<'_>) -> Sea {
    let sea = c.get("Sea");
    let under = c.get("Underwater");
    let rgb = |k: &ConfigRef<'_>, name: &str| Vec3::from_array(array_n::<3>(k, name, 0.0));
    Sea {
        sea_texture: text_or_empty(&sea, "seaTexture"),
        sea_material: text_or_empty(&sea, "seaMaterial"),
        shore_material: text_or_empty(&sea, "shoreMaterial"),
        water_map_scale: number_or(&sea, "WaterMapScale", 0.0),
        water_grid: number_or(&sea, "WaterGrid", 0.0),
        max_tide: number_or(&sea, "MaxTide", 0.0),
        max_wave: number_or(&sea, "MaxWave", 0.0),
        water_color: rgb(&under, "waterColor"),
        deep_water_color: rgb(&under, "deepWaterColor"),
        water_fog_distance: number_or(&under, "waterFogDistance", 0.0),
        water_texture: text_or_empty(c, "waterTexture"),
        sea_bed_underwater_depth: number_or(c, "seaBedUnderwaterDepth", 0.0),
        shore_top: number_or(c, "shoreTop", 0.0),
        peak_wave_top: number_or(c, "peakWaveTop", 0.0),
        peak_wave_bottom: number_or(c, "peakWaveBottom", 0.0),
        waves: read_sea_waves(&sea),
        water_ex: read_water_ex(&c.get("WaterExPars")),
    }
}

fn read_sea_waves(sea: &ConfigRef<'_>) -> SeaWaves {
    let d = SeaWaves::default();
    // Several entries are expressions such as "2.0/50", which the engine evaluates.
    let n = |name: &str, default: f32| expr_number(sea, name).unwrap_or(default);
    SeaWaves {
        water_map_scale: n("WaterMapScale", d.water_map_scale),
        water_grid: n("WaterGrid", d.water_grid),
        max_tide: n("MaxTide", d.max_tide),
        max_wave: n("MaxWave", d.max_wave),
        x_scale: n("SeaWaveXScale", d.x_scale),
        z_scale: n("SeaWaveZScale", d.z_scale),
        h_scale: n("SeaWaveHScale", d.h_scale),
        x_duration_ms: n("SeaWaveXDuration", d.x_duration_ms as f32) as i32,
        z_duration_ms: n("SeaWaveZDuration", d.z_duration_ms as f32) as i32,
    }
}

fn read_water_ex(w: &ConfigRef<'_>) -> WaterExPars {
    let n = |name: &str| expr_number(w, name);
    let v = |name: &str| {
        let values = numbers(w, name);
        (values.len() >= 3).then(|| Vec3::new(values[0], values[1], values[2]))
    };
    WaterExPars {
        fog_density: n("fogDensity"),
        fog_color: v("fogColor"),
        fog_color_extinction_speed: v("fogColorExtinctionSpeed"),
        light_extinction_speed: v("ligtExtinctionSpeed"),
        diffuse_light_extinction_speed: v("diffuseLigtExtinctionSpeed"),
        fog_gradient_coefs: v("fogGradientCoefs"),
        fog_color_light_influence: v("fogColorLightInfluence"),
        ss_reflection_strength: n("ssReflectionStrength"),
        ss_reflection_max_jitter: n("ssReflectionMaxJitter"),
        ss_reflection_ripple_influence: n("ssReflectionRippleInfluence"),
        ss_reflection_edge_fading_coef: n("ssReflectionEdgeFadingCoef"),
        ss_reflection_dist_fading_coef: n("ssReflectionDistFadingCoef"),
        specular_max_intensity: n("specularMaxIntensity"),
        specular_power_overcast0: n("specularPowerOvercast0"),
        specular_power_overcast1: n("specularPowerOvercast1"),
        specular_normal_modify_coef: n("specularNormalModifyCoef"),
        refraction_min_coef: n("refractionMinCoef"),
        refraction_max_coef: n("refractionMaxCoef"),
        refraction_max_dist: n("refractionMaxDist"),
        surface_opacity: n("surfaceOpacity"),
        shadow_intensity: n("shadowIntensity"),
        foam_around_objects_intensity: n("foamAroundObjectsIntensity"),
        foam_around_objects_fade_coef: n("foamAroundObjectsFadeCoef"),
        foam_color_coef: n("foamColorCoef"),
        foam_deformation_coef: n("foamDeformationCoef"),
        foam_texture_coef: n("foamTextureCoef"),
        foam_time_move_speed: n("foamTimeMoveSpeed"),
        foam_time_move_amount: n("foamTimeMoveAmount"),
    }
}

fn read_ambient(a: &ConfigRef<'_>) -> Vec<AmbientRadius> {
    classes(a)
        .iter()
        .map(|r| AmbientRadius {
            class: r.name().to_owned(),
            area_spawn_radius: number_or(r, "areaSpawnRadius", 0.0),
            area_max_radius: number_or(r, "areaMaxRadius", 0.0),
            spawn_circle_radius: number_or(r, "spawnCircleRadius", 0.0),
            spawn_interval: number_or(r, "spawnInterval", 0.0),
            species: classes(&r.get("Species"))
                .iter()
                .map(|s| AmbientSpecies {
                    class: s.name().to_owned(),
                    max_circle_count: text_or_empty(s, "maxCircleCount"),
                    max_world_count: number_or(s, "maxWorldCount", 0.0),
                    cost: number_or(s, "cost", 0.0),
                })
                .collect(),
        })
        .collect()
}

fn read_location(l: &ConfigRef<'_>) -> Location {
    let p = numbers(l, "position");
    Location {
        class: l.name().to_owned(),
        name: text_or_empty(l, "name"),
        kind: text_or_empty(l, "type"),
        position: Vec2::new(
            p.first().copied().unwrap_or(0.0),
            p.get(1).copied().unwrap_or(0.0),
        ),
        radius_a: number_or(l, "radiusA", 0.0),
        radius_b: number_or(l, "radiusB", 0.0),
        angle: number_or(l, "angle", 0.0),
    }
}
