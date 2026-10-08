//! Weather by overcast (`CfgWorlds >> W >> Weather >> Overcast`) and fog.

use a3_config::ConfigRef;

use crate::cfg::{classes, number_or, text_or_empty};

/// One `Weather >> Overcast >> WeatherN` class.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OvercastLevel {
    /// `overcast`: the key.
    pub overcast: f32,
    /// `sky`: sky gradient texture.
    pub sky: String,
    /// `horizon`: horizon band texture.
    pub horizon: String,
    /// `skyR`: sky (and cloud) texture used for reflections.
    pub sky_reflection: String,
    /// `alpha`: cloud layer opacity.
    pub alpha: f32,
    /// `size`: cloud size.
    pub size: f32,
    /// `height`: cloud height.
    pub height: f32,
    /// `bright`: cloud brightness.
    pub bright: f32,
    /// `speed`: cloud speed.
    pub speed: f32,
    /// `through`: how much sunlight gets through the clouds (1 clear, 0 overcast).
    pub through: f32,
    /// `diffuse`: sun light multiplier.
    pub diffuse: f32,
    /// `cloudDiffuse`.
    pub cloud_diffuse: f32,
    /// `waves`: sea state.
    pub waves: f32,
    /// `lightingOvercast`: the overcast used to look up the lighting table.
    pub lighting_overcast: f32,
}

impl OvercastLevel {
    fn from_config(c: &ConfigRef<'_>) -> Self {
        OvercastLevel {
            overcast: number_or(c, "overcast", 0.0),
            sky: text_or_empty(c, "sky"),
            horizon: text_or_empty(c, "horizon"),
            sky_reflection: text_or_empty(c, "skyR"),
            alpha: number_or(c, "alpha", 0.0),
            size: number_or(c, "size", 0.0),
            height: number_or(c, "height", 1.0),
            bright: number_or(c, "bright", 1.0),
            speed: number_or(c, "speed", 0.0),
            through: number_or(c, "through", 1.0),
            diffuse: number_or(c, "diffuse", 1.0),
            cloud_diffuse: number_or(c, "cloudDiffuse", 1.0),
            waves: number_or(c, "waves", 0.0),
            lighting_overcast: number_or(c, "lightingOvercast", 0.0),
        }
    }
}

/// The weather at one overcast: numbers interpolated between the two neighbouring levels,
/// textures of both with the blend factor.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OvercastSample {
    /// Interpolated numbers (texture names of the lower level).
    pub level: OvercastLevel,
    /// The upper level's `sky_reflection` texture to blend towards.
    pub next_sky_reflection: String,
    /// Blend factor towards `next_sky_reflection`.
    pub blend: f32,
}

/// The `Weather >> Overcast` levels, sorted by overcast.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OvercastTable {
    /// The levels, ascending overcast.
    pub levels: Vec<OvercastLevel>,
}

impl OvercastTable {
    /// Reads `Weather >> Overcast`.
    pub fn from_config(c: &ConfigRef<'_>) -> Self {
        let mut levels: Vec<OvercastLevel> =
            classes(c).iter().map(OvercastLevel::from_config).collect();
        levels.sort_by(|a, b| a.overcast.total_cmp(&b.overcast));
        // Several classes may share an overcast (Altis' Weather1 and Weather7); keep the first.
        levels.dedup_by(|b, a| (a.overcast - b.overcast).abs() <= 1e-4);
        OvercastTable { levels }
    }

    /// The weather at `overcast` (clamped to the table).
    pub fn sample(&self, overcast: f32) -> OvercastSample {
        if self.levels.is_empty() {
            return OvercastSample::default();
        }
        let above = self
            .levels
            .iter()
            .position(|l| overcast < l.overcast)
            .unwrap_or(self.levels.len());
        let (a, b) = (
            &self.levels[above.saturating_sub(1)],
            &self.levels[above.min(self.levels.len() - 1)],
        );
        let span = b.overcast - a.overcast;
        let t = if span > 0.0 {
            ((overcast - a.overcast) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let f = |x: f32, y: f32| x + (y - x) * t;
        OvercastSample {
            level: OvercastLevel {
                overcast,
                alpha: f(a.alpha, b.alpha),
                size: f(a.size, b.size),
                height: f(a.height, b.height),
                bright: f(a.bright, b.bright),
                speed: f(a.speed, b.speed),
                through: f(a.through, b.through),
                diffuse: f(a.diffuse, b.diffuse),
                cloud_diffuse: f(a.cloud_diffuse, b.cloud_diffuse),
                waves: f(a.waves, b.waves),
                lighting_overcast: f(a.lighting_overcast, b.lighting_overcast),
                ..a.clone()
            },
            next_sky_reflection: b.sky_reflection.clone(),
            blend: t,
        }
    }
}

/// Fog as `setFog [value, decay, base]` sets it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fog {
    /// Fog amount 0..1.
    pub value: f32,
    /// Density decay per metre of height above `base`.
    pub decay: f32,
    /// Height (metres above sea level) of the densest fog.
    pub base: f32,
}

impl Default for Fog {
    fn default() -> Self {
        Fog {
            value: 0.0,
            decay: 0.014,
            base: 0.0,
        }
    }
}

/// The world's fog limits (`fogBeta0Min`, `fogBeta0Max`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogLimits {
    /// Extinction per metre at fog 0.
    pub beta0_min: f32,
    /// Extinction per metre at fog 1.
    pub beta0_max: f32,
}

impl Default for FogLimits {
    fn default() -> Self {
        FogLimits {
            beta0_min: 0.0,
            beta0_max: 0.05,
        }
    }
}

impl FogLimits {
    /// Extinction per metre at the fog base for a fog `value` (the engine's curve:
    /// `min + (max - min) * (e^(4v) - 1) / (e^4 - 1)`).
    pub fn beta0(&self, value: f32) -> f32 {
        let v = value.clamp(0.0, 1.0);
        let curve = (((4.0 * v).exp() - 1.0) / (4f32.exp() - 1.0)).clamp(0.0, 1.0);
        self.beta0_min + (self.beta0_max - self.beta0_min) * curve
    }

    /// Extinction per metre at `height` above sea level:
    /// `beta0 * e^(-decay * (height - base))`.
    pub fn extinction(&self, fog: &Fog, height: f32) -> f32 {
        self.beta0(fog.value) * (-fog.decay * (height - fog.base)).exp()
    }
}
