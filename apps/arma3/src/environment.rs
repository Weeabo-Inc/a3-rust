//! The World's environment (date, time, weather, fog) driving the renderer: lighting, fog,
//! exposure and the sky feature.

use a3_environment::{DateTime, EnvironmentState, Fog, WorldEnvironment};
use a3_render::sky::{SkyFeature, SkyHandle};
use a3_render::{Gpu, Renderer, TextureData};
use glam::{Vec2, Vec3};

/// Rec. 601 luma, as the engine uses for light colours.
fn luma(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.299, 0.587, 0.114))
}

/// Environment overrides from the command line.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EnvironmentSpec {
    /// `--date year-month-day`.
    pub date: Option<(i32, u32, u32)>,
    /// `--time hh:mm`, in hours.
    pub time: Option<f64>,
    /// `--overcast 0..1`.
    pub overcast: Option<f32>,
    /// `--fog value[,decay[,base]]`.
    pub fog: Option<FogSpec>,
    /// `--fog-distance metres`: linear fog end.
    pub fog_distance: Option<f32>,
}

/// `--fog value[,decay[,base]]`, as `setFog` takes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogSpec {
    pub value: f32,
    pub decay: Option<f32>,
    pub base: Option<f32>,
}

impl std::str::FromStr for FogSpec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let v: Vec<f32> = s
            .split(',')
            .map(|p| p.trim().parse::<f32>())
            .collect::<Result<_, _>>()
            .map_err(|e| format!("expected value[,decay[,base]]: {e}"))?;
        if v.is_empty() || v.len() > 3 {
            return Err("expected value[,decay[,base]]".into());
        }
        Ok(FogSpec {
            value: v[0],
            decay: v.get(1).copied(),
            base: v.get(2).copied(),
        })
    }
}

/// Parses `--date` as `year-month-day`.
pub fn parse_date(s: &str) -> Result<(i32, u32, u32), String> {
    let p: Vec<&str> = s.split('-').collect();
    let [y, m, d] = p[..] else {
        return Err("expected year-month-day".into());
    };
    let parse = |x: &str| x.trim().parse::<i64>().map_err(|e| e.to_string());
    let (y, m, d) = (parse(y)?, parse(m)?, parse(d)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return Err("month 1-12, day 1-31".into());
    }
    Ok((y as i32, m as u32, d as u32))
}

/// Parses `--time` as `hh:mm` (or decimal hours).
pub fn parse_time(s: &str) -> Result<f64, String> {
    let (h, m) = s.split_once(':').unwrap_or((s, "0"));
    let h: f64 = h.trim().parse().map_err(|e| format!("{e}"))?;
    let m: f64 = m.trim().parse().map_err(|e| format!("{e}"))?;
    if !(0.0..24.0).contains(&h) || !(0.0..60.0).contains(&m) {
        return Err("hours 0-23, minutes 0-59".into());
    }
    Ok(h + m / 60.0)
}

/// The environment of the loaded World and the sky feature it drives.
pub struct SceneEnvironment {
    pub world: WorldEnvironment,
    pub state: EnvironmentState,
    sky: SkyHandle,
    fog_distance: Option<f32>,
    time: f32,
}

impl SceneEnvironment {
    /// Registers the sky feature and applies the command-line overrides to the World's start
    /// state.
    pub fn new(
        gpu: &Gpu,
        renderer: &mut Renderer,
        world: WorldEnvironment,
        noise: Option<&TextureData>,
        spec: &EnvironmentSpec,
    ) -> SceneEnvironment {
        let mut state = world.start;
        let dt = state.date_time;
        let (year, month, day) = spec.date.unwrap_or((dt.year, dt.month, dt.day));
        state.date_time = DateTime::new(year, month, day, spec.time.unwrap_or(dt.hours));
        if let Some(o) = spec.overcast {
            state.set_overcast(o);
        }
        if let Some(f) = spec.fog {
            state.set_fog(Fog {
                value: f.value,
                decay: f.decay.unwrap_or(state.fog.decay),
                base: f.base.unwrap_or(state.fog.base),
            });
        }
        let (feature, sky) = SkyFeature::new(gpu, renderer, noise);
        renderer.add_feature(Box::new(feature));
        log::info!(
            "environment: {:?}, overcast {}, fog {:?}",
            state.date_time,
            state.overcast,
            state.fog
        );
        SceneEnvironment {
            world,
            state,
            sky,
            fog_distance: spec.fog_distance,
            time: 0.0,
        }
    }

    /// Sets the sky dome's elevation ramp from the World's `skyTexture` (`CfgWorlds >>
    /// skyTexture`): the ramp the engine's own dome shades the sky with (`PSHorizon`, see
    /// `docs/re/render-atmosphere.md` §4). Without it the sky keeps the lighting table's
    /// gradient alone.
    pub fn set_sky_texture(&mut self, texture: Option<&TextureData>) {
        let Some(ramp) = texture.and_then(a3_render::sky::dome_ramp) else {
            return;
        };
        log::info!(
            "sky dome ramp: horizon ({:.3}, {:.3}, {:.3}), zenith ({:.3}, {:.3}, {:.3})",
            ramp[0].x,
            ramp[0].y,
            ramp[0].z,
            ramp[a3_render::sky::DOME_RAMP_STEPS - 1].x,
            ramp[a3_render::sky::DOME_RAMP_STEPS - 1].y,
            ramp[a3_render::sky::DOME_RAMP_STEPS - 1].z,
        );
        let mut sky = self.sky.lock().unwrap_or_else(|p| p.into_inner());
        sky.dome_ramp = ramp;
    }

    /// Evaluates the environment for a camera at `camera_height` and writes lighting, fog,
    /// exposure limits and sky parameters.
    pub fn apply(&mut self, renderer: &mut Renderer, camera_height: f32, dt: f32) {
        self.time += dt;
        let f = self.world.evaluate(&self.state, camera_height);
        let l = &f.light;
        let s = &mut renderer.settings;
        s.sun_direction = f.light_direction;
        s.sun_color = l.diffuse;
        s.hemisphere = Some(a3_render::HemisphereAmbient {
            sky: l.ambient,
            mid: l.ambient_mid,
            ground: l.ground_reflection,
        });
        // Shaders without hemisphere ambient (the terrain) scale the sky colours by this:
        // `mix(horizon, zenith, 0.5) * ambient * 2` should be the horizon ambient.
        let sky_mid = luma((l.fog_color + l.sky) * 0.5).max(1e-6);
        s.ambient = luma(l.ambient_mid) / (2.0 * sky_mid);
        s.sky_zenith = l.sky;
        s.sky_horizon = l.fog_color;
        s.fog_density = f.fog_sea_level;
        s.fog_decay = f.fog_decay;
        s.haze = f.haze;
        if let Some(end) = self.fog_distance {
            s.fog_start = end * 0.5;
            s.fog_end = end;
        }
        s.procedural_sky = false;
        // The table's apertures say how far exposure may follow the scene: `standardAvgLum` is
        // exposed like `apertureStandard`, and the aperture stays within apertureMin..Max
        // (exposure 1/aperture², docs/re/render-atmosphere.md §3.3). Our eye adaptation
        // tracks the scene's average luminance, so bound that: never darker than the
        // standard luminance scaled by (min / standard)² (night stays night), and allow
        // brighter scenes than the table's maximum, which the engine's own meter handles in
        // ways not traced yet.
        let standard = l.aperture_standard.max(1e-3);
        let lum = l.standard_avg_lum.max(1e-3);
        s.hdr.min_aperture = lum * (l.aperture_min / standard).powi(2);
        s.hdr.max_aperture = lum * (l.aperture_max / standard).max(1.0).powi(2) * 16.0;

        let mut sky = self.sky.lock().unwrap_or_else(|p| p.into_inner());
        sky.zenith = l.sky;
        sky.horizon = l.fog_color;
        sky.around_sun = l.sky_around_sun;
        sky.ground = l.ground_reflection * 0.3;
        sky.sun_direction = f.sun_direction;
        sky.sun_color = f.sun_color_for_sky;
        sky.sun_disc_scale = 40.0;
        // The engine's sun and moon objects are drawn larger than the real 0.27 degrees.
        sky.sun_radius = 0.45f32.to_radians();
        sky.moon_radius = 0.9f32.to_radians();
        sky.moon_direction = f.moon_direction;
        sky.moon_color = self.world.moon_color_full;
        sky.stars = f.stars;
        sky.star_rotation = f.star_rotation;
        // Cloud layer from the overcast level; lit by the sky and, through the clouds, the sun.
        let w = &f.weather.level;
        sky.cloud_cover = (w.size * f32::from(self.state.overcast > 0.05)).clamp(0.0, 1.0);
        sky.cloud_opacity = (0.55 + 0.45 * self.state.overcast).clamp(0.0, 1.0);
        let hue = l.clouds_color / luma(l.clouds_color).max(1e-6);
        sky.cloud_color = hue * (luma(l.ambient_mid) * 1.1 + luma(l.diffuse) * 0.12 * w.through);
        sky.wind = Vec2::new(4.0, 1.5) * (0.5 + w.speed);
        sky.time = self.time;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_date_time_and_fog() {
        assert_eq!(parse_date("2035-6-24"), Ok((2035, 6, 24)));
        assert!(parse_date("24/6/2035").is_err());
        assert_eq!(parse_time("05:30"), Ok(5.5));
        assert_eq!(parse_time("18"), Ok(18.0));
        assert!(parse_time("25:00").is_err());
        let fog: FogSpec = "0.3,0.01,50".parse().unwrap();
        assert_eq!(
            (fog.value, fog.decay, fog.base),
            (0.3, Some(0.01), Some(50.0))
        );
        let fog: FogSpec = "0.5".parse().unwrap();
        assert_eq!(fog.decay, None);
    }
}
