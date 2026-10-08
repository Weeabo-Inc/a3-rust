//! The environment state, the world's tables, and what they give for one frame.

use a3_config::ConfigRef;
use glam::Vec3;

use crate::cfg::{array_n, number, number_or, text_or_empty};
use crate::weather::{Fog, FogLimits, OvercastSample, OvercastTable};
use crate::{
    DateTime, LightingEntry, LightingTable, Observer, moon_illumination, moon_phase, moon_position,
    sun_position,
};

/// The environment as scripts change it (`setDate`, `skipTime`, `setOvercast`, `setFog`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvironmentState {
    /// Local date and time.
    pub date_time: DateTime,
    /// Cloud cover 0..1.
    pub overcast: f32,
    /// Fog.
    pub fog: Fog,
    /// Rain 0..1 (no effect yet).
    pub rain: f32,
    /// Wave height 0..1; `None` follows the overcast.
    pub waves: Option<f32>,
}

impl EnvironmentState {
    /// `setDate [year, month, day, hour, minute]`.
    pub fn set_date(&mut self, year: i32, month: u32, day: u32, hour: u32, minute: u32) {
        self.date_time =
            DateTime::new(year, month, day, f64::from(hour) + f64::from(minute) / 60.0);
    }

    /// `skipTime hours`.
    pub fn skip_time(&mut self, hours: f64) {
        self.date_time.skip_hours(hours);
    }

    /// `setOvercast value` (applied at once).
    pub fn set_overcast(&mut self, overcast: f32) {
        self.overcast = overcast.clamp(0.0, 1.0);
    }

    /// `setFog [value, decay, base]` (applied at once).
    pub fn set_fog(&mut self, fog: Fog) {
        self.fog = Fog {
            value: fog.value.clamp(0.0, 1.0),
            ..fog
        };
    }
}

/// The environment-related settings of one world (`CfgWorlds >> W`).
#[derive(Debug, Clone, PartialEq)]
pub struct WorldEnvironment {
    /// Where the world is on Earth (for the sun and moon).
    pub observer: Observer,
    /// The state at mission start (`startDate`, `startTime`, `startWeather`, `startFog*`).
    pub start: EnvironmentState,
    /// `Weather >> LightingNew`.
    pub lighting: LightingTable,
    /// `Weather >> Overcast`.
    pub overcast: OvercastTable,
    /// `fogBeta0Min`, `fogBeta0Max`.
    pub fog_limits: FogLimits,
    /// `Lighting >> starEmissivity`.
    pub star_emissivity: f32,
    /// `Lighting >> moonObjectColorFull` (RGB, absolute units).
    pub moon_color_full: Vec3,
    /// `Lighting >> moonHaloObjectColorFull`.
    pub moon_halo_color_full: Vec3,
}

/// Everything the renderer needs about the environment for one frame. Colours are linear
/// RGB in the engine's absolute units (luminance about `2^EV`).
#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentFrame {
    /// Unit vector towards the sun (x east, y up, z north).
    pub sun_direction: Vec3,
    /// Unit vector towards the moon.
    pub moon_direction: Vec3,
    /// Unit vector towards the light that lights the scene: the sun, or the moon when the
    /// table's `sunOrMoon` is below 0.5.
    pub light_direction: Vec3,
    /// `light_direction` for shadows: when its height is below 0.4 the engine sets it to 0.4
    /// and renormalises, so shadows never get longer than about 2.3 times the caster height.
    pub shadow_direction: Vec3,
    /// Sun elevation in degrees.
    pub sun_elevation: f32,
    /// The engine's `moonPhase` (0 new, 1 full).
    pub moon_phase: f32,
    /// Illuminated fraction of the moon's disc.
    pub moon_illumination: f32,
    /// Moon disc colour for its phase.
    pub moon_color: Vec3,
    /// The interpolated lighting entry, its cloud variants already blended in.
    pub light: LightingEntry,
    /// How much the clouds cover the sun (blend factor towards the `...Cloud` colours).
    pub cloud_cover: f32,
    /// Weather at the current overcast (sky textures, cloud layer parameters).
    pub weather: OvercastSample,
    /// Fog extinction per metre at sea level (height fog: times `e^(-decay * height)`).
    pub fog_sea_level: f32,
    /// Fog density decay per metre of height.
    pub fog_decay: f32,
    /// Distance haze extinction per metre (the table's Rayleigh + Mie coefficients, per km
    /// in config).
    pub haze: Vec3,
    /// Star brightness (0 by day).
    pub stars: f32,
}

impl WorldEnvironment {
    /// Reads the environment settings of a world class (`CfgWorlds >> W`, inheritance
    /// resolved by the config tree).
    pub fn from_config(world: &ConfigRef<'_>) -> Self {
        let weather = world.get("Weather");
        let lighting = world.get("Lighting");
        let date_time = DateTime::from_config(
            &text_or_empty(world, "startDate"),
            &text_or_empty(world, "startTime"),
        )
        .unwrap_or(DateTime::new(2035, 6, 24, 12.0));
        WorldEnvironment {
            observer: Observer::from_world_config(
                number_or(world, "latitude", -40.0),
                number_or(world, "longitude", 15.0),
            ),
            start: EnvironmentState {
                date_time,
                overcast: number_or(world, "startWeather", 0.3),
                fog: Fog {
                    value: number_or(world, "startFog", 0.0),
                    decay: number_or(world, "startFogDecay", 0.014),
                    base: number_or(world, "startFogBase", 0.0),
                },
                rain: 0.0,
                waves: None,
            },
            lighting: LightingTable::from_config(&weather.get("LightingNew")),
            overcast: OvercastTable::from_config(&weather.get("Overcast")),
            fog_limits: FogLimits {
                beta0_min: number_or(world, "fogBeta0Min", 0.0),
                beta0_max: number_or(world, "fogBeta0Max", 0.05),
            },
            star_emissivity: number(&lighting, "starEmissivity").unwrap_or(25.0),
            moon_color_full: Vec3::from_slice(&array_n::<3>(
                &lighting,
                "moonObjectColorFull",
                400.0,
            )),
            moon_halo_color_full: Vec3::from_slice(&array_n::<3>(
                &lighting,
                "moonHaloObjectColorFull",
                400.0,
            )),
        }
    }

    /// The frame values for `state`, seen from `camera_height` metres above sea level.
    pub fn evaluate(&self, state: &EnvironmentState, camera_height: f32) -> EnvironmentFrame {
        let sun = sun_position(&self.observer, &state.date_time);
        let moon = moon_position(&self.observer, &state.date_time);
        let (sun_dir, moon_dir) = (sun.direction(), moon.direction());
        let weather = self.overcast.sample(state.overcast);
        let sin_sun = sun_dir.y;
        let height_key = if camera_height < 0.0 { -0.1 } else { 0.0 };
        let raw = self
            .lighting
            .sample(height_key, weather.level.lighting_overcast, sin_sun);
        let cloud_cover = (1.0 - weather.level.through).clamp(0.0, 1.0);
        let light = blend_clouds(&raw, cloud_cover);

        let light_direction = if light.sun_or_moon >= 0.5 {
            sun_dir
        } else {
            moon_dir
        };
        let mut shadow_direction = light_direction;
        if shadow_direction.y < 0.4 {
            shadow_direction.y = 0.4;
            shadow_direction = shadow_direction.normalize();
        }
        let phase = moon_phase(sun_dir, moon_dir);
        let illumination = moon_illumination(phase);
        let fog = state.fog;
        let fog_sea_level = self.fog_limits.beta0(fog.value) * (fog.decay * fog.base).exp();
        let sun_elevation = sun.elevation as f32;
        // Stars fade in between -4 and -12 degrees of sun elevation, behind clouds less so.
        let night = ((-sun_elevation - 4.0) / 8.0).clamp(0.0, 1.0);
        let stars = self.star_emissivity * night * (1.0 - weather.level.alpha).clamp(0.0, 1.0);
        EnvironmentFrame {
            sun_direction: sun_dir,
            moon_direction: moon_dir,
            light_direction,
            shadow_direction,
            sun_elevation,
            moon_phase: phase,
            moon_illumination: illumination,
            moon_color: self.moon_color_full * illumination,
            haze: (light.rayleigh + light.mie) / 1000.0,
            light,
            cloud_cover,
            weather,
            fog_sea_level,
            fog_decay: fog.decay,
            stars,
        }
    }
}

/// Blends each `X` / `XCloud` pair by cloud cover and keeps the result in the plain fields.
fn blend_clouds(e: &LightingEntry, cloud: f32) -> LightingEntry {
    let mix = |a: Vec3, b: Vec3| a + (b - a) * cloud;
    LightingEntry {
        diffuse: mix(e.diffuse, e.diffuse_cloud),
        ambient: mix(e.ambient, e.ambient_cloud),
        ambient_mid: mix(e.ambient_mid, e.ambient_mid_cloud),
        ground_reflection: mix(e.ground_reflection, e.ground_reflection_cloud),
        bidirect: mix(e.bidirect, e.bidirect_cloud),
        ..*e
    }
}
