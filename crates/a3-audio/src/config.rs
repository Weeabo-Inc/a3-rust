//! The game's sound definitions from config: sound shaders, sound sets, curves, distance
//! filters, 3D processors, and the older CfgSounds / CfgMusic / CfgSFX entries.
//!
//! [`SoundBank::load`] reads them from a merged [`ConfigTree`]; names are looked up ignoring
//! case. The semantics of each field are in `docs/re/audio.md`.

use std::collections::HashMap;

use a3_config::{ConfigRef, ConfigTree, Value};

use crate::Curve;
use crate::expr::Expr;

/// A sound file of a shader, with its relative probability.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// VFS path as written (often without extension).
    pub path: String,
    /// Relative weight in the random choice.
    pub probability: f32,
}

/// A distance curve given by name (`CfgSoundCurves`) or inline as points.
#[derive(Debug, Clone, PartialEq)]
pub enum CurveRef {
    /// A `CfgSoundCurves` class; its x axis is 0..1 of the range.
    Named(String),
    /// Inline `(x, y)` points.
    Points(Vec<(f32, f32)>),
}

/// `CfgSoundShaders` class.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundShader {
    /// Class name.
    pub name: String,
    /// Sound files to choose from.
    pub samples: Vec<Sample>,
    /// Linear volume: a constant (already multiplied by the shader's `volumeFactor`) or an
    /// expression (the engine does not apply `volumeFactor` to expressions).
    pub volume: Expr,
    /// Pitch factor expression (default 1).
    pub frequency: Expr,
    /// Audible range in metres (0 when not given).
    pub range: f32,
    /// Gain over distance; inline points are in metres.
    pub range_curve: Option<CurveRef>,
    /// `limitation = 1`: counts against the set's `soundShadersLimit`.
    pub limitation: bool,
}

/// `CfgSoundSets` class.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundSet {
    /// Class name.
    pub name: String,
    /// Shader class names, played together.
    pub shaders: Vec<String>,
    /// Set-level volume (constant or expression, default 1), multiplied into every shader.
    pub volume: Expr,
    /// Linear gain, 0..=3.1623 (+10 dB), default 1.
    pub volume_factor: f32,
    /// Pitch factor, 0.5..=2, default 1.
    pub frequency_factor: f32,
    /// Gain over distance (`volumeCurve`, x normalised to 0..1 of the largest shader range);
    /// `None` means `CfgSoundGlobals.defaultVolumeCurve`.
    pub volume_curve: Option<CurveRef>,
    /// 3D (default) or 2D.
    pub spatial: bool,
    /// Doppler (default on).
    pub doppler: bool,
    /// Loop the samples (cleared by a `delay`).
    pub looping: bool,
    /// At most this many shaders with `limitation = 1` play at once (0 = no limit).
    pub shaders_limit: u32,
    /// Largest random pitch offset in semitones, 0..=12.
    pub frequency_randomizer: f32,
    /// Smallest random pitch offset in semitones, 0..=12.
    pub frequency_randomizer_min: f32,
    /// Largest random volume ratio, 1..=1.995 (+6 dB); the offset is applied in dB.
    pub volume_randomizer: f32,
    /// Smallest random volume ratio, 1..=1.995.
    pub volume_randomizer_min: f32,
    /// `CfgDistanceFilters` class name; `None` means `CfgSoundGlobals.defaultDistanceFilter`.
    pub distance_filter: Option<String>,
    /// Occlusion factor, 0..=1, default 0.96 (consumer not traced).
    pub occlusion_factor: f32,
    /// Obstruction factor, 0..=1, default 0.7 (consumer not traced).
    pub obstruction_factor: f32,
    /// `CfgSound3DProcessors` class name; `None` means the global default.
    pub processing_type: Option<String>,
    /// Radius around the listener within which the sound surrounds it, metres (default 0.5).
    pub spatiality_range: f32,
    /// Angle of that inner region, radians (default 0.7854).
    pub spatiality_range_angle: f32,
    /// Offset of the source from its object, in metres (model space).
    pub position_offset: Option<[f32; 3]>,
    /// Delay before playing in seconds (`None` when absent or below 0.01).
    pub delay: Option<f32>,
    /// Random addition to the delay, in seconds (0..=delay).
    pub delay_randomizer: f32,
}

/// `CfgDistanceFilters` class.
#[derive(Debug, Clone, PartialEq)]
pub struct DistanceFilterDef {
    /// Class name.
    pub name: String,
    /// `type`, `lowPassFilter` for every shipped filter.
    pub kind: String,
    /// Cutoff at full range, in Hz.
    pub min_cutoff_hz: f32,
    /// Filter resonance.
    pub q: f32,
    /// Distance up to which the filter is open.
    pub inner_range: f32,
    /// Distance at which the cutoff reaches the minimum.
    pub range: f32,
    /// Shape of the fall-off.
    pub power: f32,
}

/// `CfgSound3DProcessors` class.
#[derive(Debug, Clone, PartialEq)]
pub struct Processor3d {
    /// Class name.
    pub name: String,
    /// `emitter` or `panner`.
    pub kind: String,
    /// Inner range in metres.
    pub inner_range: f32,
    /// Range in metres.
    pub range: f32,
    /// Emitter radius in metres (`emitter` type).
    pub radius: Option<f32>,
    /// Panner range curve (`panner` type).
    pub range_curve: Option<CurveRef>,
}

/// `CfgSoundGlobals`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SoundGlobals {
    /// Distance filter of sets without one.
    pub default_distance_filter: Option<String>,
    /// Volume curve of sets without one.
    pub default_volume_curve: Option<String>,
    /// Volume curve of weapon sets without one.
    pub default_weapon_volume_curve: Option<String>,
    /// 3D processor of sets without one.
    pub default_processing_type: Option<String>,
    /// Gain of old-style (`sound[]`) definitions.
    pub old_configuration_volume_factor: f32,
}

/// An old-style sound: `sound[] = {path, volume, pitch, distance}` (CfgSounds, CfgMusic,
/// CfgSFX, CfgEnvSounds).
#[derive(Debug, Clone, PartialEq)]
pub struct LegacySound {
    /// Class or entry name.
    pub name: String,
    /// VFS path (empty for "no sound").
    pub path: String,
    /// Linear volume (`db-10` style strings are converted).
    pub volume: f32,
    /// Pitch factor.
    pub pitch: f32,
    /// Audible distance in metres, if given.
    pub distance: Option<f32>,
    /// Volume expression (CfgEnvSounds `volume`), if any.
    pub volume_expr: Option<Expr>,
}

/// One random sound of a CfgSFX class.
#[derive(Debug, Clone, PartialEq)]
pub struct SfxSound {
    /// The sound.
    pub sound: LegacySound,
    /// Relative probability.
    pub probability: f32,
    /// Minimum, typical and maximum delay before the next sound, in seconds.
    pub delay: [f32; 3],
}

/// A CfgSFX class: random sounds played with delays.
#[derive(Debug, Clone, PartialEq)]
pub struct Sfx {
    /// Class name.
    pub name: String,
    /// The sounds.
    pub sounds: Vec<SfxSound>,
    /// The "silence" entry, if any.
    pub empty: Option<SfxSound>,
}

/// Every sound definition of a merged config.
#[derive(Debug, Clone, Default)]
pub struct SoundBank {
    shaders: HashMap<String, SoundShader>,
    sets: HashMap<String, SoundSet>,
    curves: HashMap<String, Curve>,
    filters: HashMap<String, DistanceFilterDef>,
    processors: HashMap<String, Processor3d>,
    sounds: HashMap<String, LegacySound>,
    music: HashMap<String, LegacySound>,
    sfx: HashMap<String, Sfx>,
    env_sounds: HashMap<String, LegacySound>,
    /// CfgEnvSounds `soundSetEnvironment[]`: the ambient sound sets.
    pub environment_sets: Vec<String>,
    /// CfgSoundGlobals.
    pub globals: SoundGlobals,
    /// Problems found while loading (bad expressions, malformed arrays).
    pub warnings: Vec<String>,
}

fn key(name: &str) -> String {
    name.to_ascii_lowercase()
}

impl SoundBank {
    /// Reads every sound class of the merged config.
    pub fn load(config: &ConfigTree) -> Self {
        let root = config.root();
        let mut bank = Self::default();
        let mut warnings = Vec::new();
        let mut warn = |w: String| warnings.push(w);

        for c in classes(root.get("CfgSoundCurves")) {
            if let Some(curve) = normalized_curve(&points(&c.get("points").array())) {
                bank.curves.insert(key(c.name()), curve);
            }
        }
        for c in classes(root.get("CfgSoundShaders")) {
            let volume_factor =
                number_or(&c.get("volumeFactor"), 1.0).clamp(0.0, MAX_VOLUME_FACTOR);
            let mut volume = expr_or(&c.get("volume"), 1.0, &mut warn);
            if let Some(v) = volume.as_constant() {
                volume = Expr::constant(v * volume_factor);
            }
            let shader = SoundShader {
                name: c.name().to_string(),
                samples: samples(&c.get("samples").array()),
                volume,
                frequency: expr_or(&c.get("frequency"), 1.0, &mut warn),
                range: c.get("range").number(),
                range_curve: curve_ref(&c.get("rangeCurve")),
                limitation: c.get("limitation").number() != 0.0,
            };
            bank.shaders.insert(key(c.name()), shader);
        }
        for c in classes(root.get("CfgSoundSets")) {
            let flag =
                |name: &str, default: bool| opt_number(&c.get(name)).map_or(default, |v| v != 0.0);
            let delay = opt_number(&c.get("delay")).filter(|&d| d >= 0.01);
            let set = SoundSet {
                name: c.name().to_string(),
                shaders: strings(&c.get("soundShaders").array()),
                volume: expr_or(&c.get("volume"), 1.0, &mut warn),
                volume_factor: number_or(&c.get("volumeFactor"), 1.0).clamp(0.0, MAX_VOLUME_FACTOR),
                frequency_factor: number_or(&c.get("frequencyFactor"), 1.0).clamp(0.5, 2.0),
                volume_curve: curve_ref(&c.get("volumeCurve")),
                spatial: flag("spatial", true),
                doppler: flag("doppler", true),
                looping: flag("loop", false) && delay.is_none(),
                shaders_limit: c.get("soundShadersLimit").number().clamp(0.0, 255.0) as u32,
                frequency_randomizer: c.get("frequencyRandomizer").number().clamp(0.0, 12.0),
                frequency_randomizer_min: c.get("frequencyRandomizerMin").number().clamp(0.0, 12.0),
                volume_randomizer: number_or(&c.get("volumeRandomizer"), 1.0)
                    .clamp(1.0, MAX_VOLUME_RANDOMIZER),
                volume_randomizer_min: number_or(&c.get("volumeRandomizerMin"), 1.0)
                    .clamp(1.0, MAX_VOLUME_RANDOMIZER),
                distance_filter: opt_text(&c.get("distanceFilter")),
                occlusion_factor: number_or(&c.get("occlusionFactor"), 0.96).clamp(0.0, 1.0),
                obstruction_factor: number_or(&c.get("obstructionFactor"), 0.7).clamp(0.0, 1.0),
                processing_type: opt_text(&c.get("sound3DProcessingType")),
                spatiality_range: number_or(&c.get("spatialityRange"), 0.5),
                spatiality_range_angle: number_or(
                    &c.get("spatialityRangeAngle"),
                    std::f32::consts::FRAC_PI_4,
                ),
                position_offset: vec3(&c.get("posOffset").array()),
                delay,
                delay_randomizer: c
                    .get("delayRandomizer")
                    .number()
                    .clamp(0.0, delay.unwrap_or(0.0)),
            };
            bank.sets.insert(key(c.name()), set);
        }
        for c in classes(root.get("CfgDistanceFilters")) {
            let inner_range = number_or(&c.get("innerRange"), 100.0).max(0.0);
            let filter = DistanceFilterDef {
                name: c.name().to_string(),
                kind: opt_text(&c.get("type")).unwrap_or_else(|| "lowPassFilter".into()),
                min_cutoff_hz: number_or(&c.get("minCutoffFrequency"), 44_100.0),
                q: number_or(&c.get("qFactor"), 1.0),
                inner_range,
                range: number_or(&c.get("range"), 1500.0).max(inner_range),
                power: number_or(&c.get("powerFactor"), 2.0).clamp(1e-4, 100.0),
            };
            bank.filters.insert(key(c.name()), filter);
        }
        for c in classes(root.get("CfgSound3DProcessors")) {
            let kind = c.get("type").text();
            let emitter = kind.eq_ignore_ascii_case("emitter");
            let inner_range =
                number_or(&c.get("innerRange"), if emitter { 1.0 } else { 0.0 }).max(0.0);
            let processor = Processor3d {
                name: c.name().to_string(),
                kind,
                inner_range,
                range: number_or(&c.get("range"), if emitter { 4.0 } else { 0.0 }).max(inner_range),
                radius: opt_number(&c.get("radius")).or(emitter.then_some(3.0)),
                range_curve: curve_ref(&c.get("rangeCurve")),
            };
            bank.processors.insert(key(c.name()), processor);
        }
        let globals = root.get("CfgSoundGlobals");
        bank.globals = SoundGlobals {
            default_distance_filter: opt_text(&globals.get("defaultDistanceFilter")),
            default_volume_curve: opt_text(&globals.get("defaultVolumeCurve")),
            default_weapon_volume_curve: opt_text(&globals.get("defaultWeaponVolumeCurve")),
            default_processing_type: opt_text(&globals.get("defaultSound3DProcessingType")),
            old_configuration_volume_factor: number_or(
                &globals.get("OldConfigurationVolumeFactor"),
                1.0,
            ),
        };

        for (cfg, map) in [
            ("CfgSounds", &mut bank.sounds),
            ("CfgMusic", &mut bank.music),
        ] {
            for c in classes(root.get(cfg)) {
                if let Some(sound) = legacy(c.name(), &c.get("sound").array(), None) {
                    map.insert(key(c.name()), sound);
                }
            }
        }
        let env = root.get("CfgEnvSounds");
        for c in classes(env.clone()) {
            let volume = c.get("volume");
            let volume_expr = (!volume.is_null()).then(|| expr_or(&volume, 1.0, &mut warn));
            if let Some(sound) = legacy(c.name(), &c.get("sound").array(), volume_expr) {
                bank.env_sounds.insert(key(c.name()), sound);
            }
        }
        bank.environment_sets = strings(&env.get("soundSetEnvironment").array());
        for c in classes(root.get("CfgSFX")) {
            let entry = |name: &str| sfx_sound(name, &c.get(name).array());
            let sounds = strings(&c.get("sounds").array())
                .iter()
                .filter_map(|name| entry(name))
                .collect();
            let empty = entry("empty");
            bank.sfx.insert(
                key(c.name()),
                Sfx {
                    name: c.name().to_string(),
                    sounds,
                    empty,
                },
            );
        }
        bank.warnings = warnings;
        bank
    }

    /// A sound shader by name.
    pub fn shader(&self, name: &str) -> Option<&SoundShader> {
        self.shaders.get(&key(name))
    }

    /// A sound set by name.
    pub fn set(&self, name: &str) -> Option<&SoundSet> {
        self.sets.get(&key(name))
    }

    /// A `CfgSoundCurves` curve by name.
    pub fn curve(&self, name: &str) -> Option<&Curve> {
        self.curves.get(&key(name))
    }

    /// A distance filter by name.
    pub fn distance_filter(&self, name: &str) -> Option<&DistanceFilterDef> {
        self.filters.get(&key(name))
    }

    /// A 3D processor by name.
    pub fn processor(&self, name: &str) -> Option<&Processor3d> {
        self.processors.get(&key(name))
    }

    /// A CfgSounds class (`playSound`, `say3D`).
    pub fn sound(&self, name: &str) -> Option<&LegacySound> {
        self.sounds.get(&key(name))
    }

    /// A CfgMusic class (`playMusic`).
    pub fn music(&self, name: &str) -> Option<&LegacySound> {
        self.music.get(&key(name))
    }

    /// A CfgSFX class.
    pub fn sfx(&self, name: &str) -> Option<&Sfx> {
        self.sfx.get(&key(name))
    }

    /// An old-style CfgEnvSounds class.
    pub fn env_sound(&self, name: &str) -> Option<&LegacySound> {
        self.env_sounds.get(&key(name))
    }

    /// Counts: shaders, sets, curves, filters, processors, sounds, music, sfx.
    pub fn counts(&self) -> [usize; 8] {
        [
            self.shaders.len(),
            self.sets.len(),
            self.curves.len(),
            self.filters.len(),
            self.processors.len(),
            self.sounds.len(),
            self.music.len(),
            self.sfx.len(),
        ]
    }

    /// Every sound set, in no particular order.
    pub fn sets(&self) -> impl Iterator<Item = &SoundSet> {
        self.sets.values()
    }

    /// Every sound shader, in no particular order.
    pub fn shaders(&self) -> impl Iterator<Item = &SoundShader> {
        self.shaders.values()
    }

    /// Resolves a curve reference to a curve over `0..=1`. Inline points are renormalised like
    /// named ones (the engine discards their absolute x scale). `None` for an unknown name or
    /// fewer than two points.
    pub fn resolve_curve(&self, curve: &CurveRef) -> Option<Curve> {
        match curve {
            CurveRef::Named(name) => self.curve(name).cloned(),
            CurveRef::Points(points) => normalized_curve(points),
        }
    }
}

/// Largest `volumeFactor`: +10 dB.
const MAX_VOLUME_FACTOR: f32 = 3.162_277_7;
/// Largest `volumeRandomizer`: +6 dB.
const MAX_VOLUME_RANDOMIZER: f32 = 1.995_262_3;

/// A curve with its x values rescaled to 0..1, as the engine stores every sound curve.
pub fn normalized_curve(points: &[(f32, f32)]) -> Option<Curve> {
    if points.len() < 2 {
        return None;
    }
    let min = points.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let max = points.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
    let span = max - min;
    Some(Curve::new(points.iter().map(|&(x, y)| {
        (if span > 0.0 { (x - min) / span } else { 0.0 }, y)
    })))
}

fn classes(parent: ConfigRef<'_>) -> Vec<ConfigRef<'_>> {
    parent
        .entries()
        .into_iter()
        .filter(ConfigRef::is_class)
        .collect()
}

fn opt_number(c: &ConfigRef) -> Option<f32> {
    if c.is_null() {
        return None;
    }
    if c.is_number() {
        return Some(c.number());
    }
    value_number(&Value::String(c.text()))
}

fn number_or(c: &ConfigRef, default: f32) -> f32 {
    opt_number(c).unwrap_or(default)
}

fn opt_text(c: &ConfigRef) -> Option<String> {
    let text = c.text();
    (!c.is_null() && !text.is_empty()).then_some(text)
}

/// A number, a numeric string, or a `db+N` / `db-N` string (decibels to linear).
pub fn value_number(v: &Value) -> Option<f32> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f32),
        Value::Int64(i) => Some(*i as f32),
        Value::String(s) | Value::Expression(s) => {
            let s = s.trim();
            if let Some(db) = s
                .strip_prefix("db")
                .or_else(|| s.strip_prefix("DB"))
                .or_else(|| s.strip_prefix("Db"))
            {
                return db.trim().parse::<f32>().ok().map(db_to_linear);
            }
            s.parse().ok()
        }
        Value::Array(_) => None,
    }
}

/// Converts decibels to a linear gain.
pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn expr_or(c: &ConfigRef, default: f32, warn: &mut impl FnMut(String)) -> Expr {
    if c.is_null() {
        return Expr::constant(default);
    }
    if c.is_number() {
        return Expr::constant(c.number());
    }
    let text = c.text();
    if let Some(v) = value_number(&Value::String(text.clone())) {
        return Expr::constant(v);
    }
    match Expr::parse(&text) {
        Ok(e) => e,
        Err(e) => {
            warn(format!("{}: {e}", c.path_string()));
            Expr::constant(default)
        }
    }
}

fn points(items: &[Value]) -> Vec<(f32, f32)> {
    items
        .iter()
        .filter_map(|p| match p {
            Value::Array(xy) if xy.len() >= 2 => {
                Some((value_number(&xy[0])?, value_number(&xy[1])?))
            }
            _ => None,
        })
        .collect()
}

fn curve_ref(c: &ConfigRef) -> Option<CurveRef> {
    if c.is_null() {
        return None;
    }
    if c.is_array() {
        return Some(CurveRef::Points(points(&c.array())));
    }
    opt_text(c).map(CurveRef::Named)
}

fn strings(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(|v| match v {
            Value::String(s) | Value::Expression(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

fn samples(items: &[Value]) -> Vec<Sample> {
    items
        .iter()
        .filter_map(|v| match v {
            Value::Array(pair) => Some(Sample {
                path: match pair.first()? {
                    Value::String(s) => s.clone(),
                    _ => return None,
                },
                probability: pair.get(1).and_then(value_number).unwrap_or(1.0),
            }),
            _ => None,
        })
        .collect()
}

fn vec3(items: &[Value]) -> Option<[f32; 3]> {
    if items.len() != 3 {
        return None;
    }
    Some([
        value_number(&items[0])?,
        value_number(&items[1])?,
        value_number(&items[2])?,
    ])
}

/// `{path, volume, pitch, distance?}`.
fn legacy(name: &str, items: &[Value], volume_expr: Option<Expr>) -> Option<LegacySound> {
    let path = match items.first()? {
        Value::String(s) => s.clone(),
        _ => return None,
    };
    Some(LegacySound {
        name: name.to_string(),
        path,
        volume: items.get(1).and_then(value_number).unwrap_or(1.0),
        pitch: items.get(2).and_then(value_number).unwrap_or(1.0),
        distance: items.get(3).and_then(value_number),
        volume_expr,
    })
}

/// CfgSFX entry: `{path, volume, pitch, distance, probability, minDelay, midDelay, maxDelay}`.
fn sfx_sound(name: &str, items: &[Value]) -> Option<SfxSound> {
    let sound = legacy(name, items, None)?;
    let n = |i: usize| items.get(i).and_then(value_number).unwrap_or(0.0);
    Some(SfxSound {
        sound,
        probability: items.get(4).and_then(value_number).unwrap_or(1.0),
        delay: [n(5), n(6), n(7)],
    })
}
