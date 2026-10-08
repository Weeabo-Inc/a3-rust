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
    /// Linear volume expression (default 1).
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
    /// Linear gain applied to every shader.
    pub volume_factor: f32,
    /// Pitch factor applied to every shader.
    pub frequency_factor: f32,
    /// Gain over distance (`volumeCurve`), if given.
    pub volume_curve: Option<CurveRef>,
    /// 3D (`spatial = 1`) or 2D; `None` when not given.
    pub spatial: Option<bool>,
    /// Doppler on; `None` when not given.
    pub doppler: Option<bool>,
    /// Loop the samples.
    pub looping: bool,
    /// At most this many shaders with `limitation = 1` play at once (0 = no limit).
    pub shaders_limit: u32,
    /// Random pitch variation (see `docs/re/audio.md` for units).
    pub frequency_randomizer: f32,
    /// Lower bound of the pitch variation, if given separately.
    pub frequency_randomizer_min: Option<f32>,
    /// Random volume variation.
    pub volume_randomizer: f32,
    /// Lower bound of the volume variation, if given separately.
    pub volume_randomizer_min: Option<f32>,
    /// `CfgDistanceFilters` class name.
    pub distance_filter: Option<String>,
    /// How much occlusion lowers this set (engine default when `None`).
    pub occlusion_factor: Option<f32>,
    /// How much obstruction lowers this set (engine default when `None`).
    pub obstruction_factor: Option<f32>,
    /// `CfgSound3DProcessors` class name.
    pub processing_type: Option<String>,
    /// Offset of the source from its object, in metres (model space).
    pub position_offset: Option<[f32; 3]>,
    /// Delay before playing, in seconds.
    pub delay: f32,
    /// Random addition to the delay, in seconds.
    pub delay_randomizer: f32,
    /// Speed of sound override for this set, in m/s.
    pub speed_of_sound: Option<f32>,
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
            let points = points(&c.get("points").array());
            bank.curves.insert(key(c.name()), Curve::new(points));
        }
        for c in classes(root.get("CfgSoundShaders")) {
            let shader = SoundShader {
                name: c.name().to_string(),
                samples: samples(&c.get("samples").array()),
                volume: expr_or(&c.get("volume"), 1.0, &mut warn),
                frequency: expr_or(&c.get("frequency"), 1.0, &mut warn),
                range: c.get("range").number(),
                range_curve: curve_ref(&c.get("rangeCurve")),
                limitation: c.get("limitation").number() != 0.0,
            };
            bank.shaders.insert(key(c.name()), shader);
        }
        for c in classes(root.get("CfgSoundSets")) {
            let set = SoundSet {
                name: c.name().to_string(),
                shaders: strings(&c.get("soundShaders").array()),
                volume_factor: number_or(&c.get("volumeFactor"), 1.0),
                frequency_factor: number_or(&c.get("frequencyFactor"), 1.0),
                volume_curve: curve_ref(&c.get("volumeCurve")),
                spatial: opt_number(&c.get("spatial")).map(|v| v != 0.0),
                doppler: opt_number(&c.get("doppler")).map(|v| v != 0.0),
                looping: c.get("loop").number() != 0.0,
                shaders_limit: c.get("soundShadersLimit").number().max(0.0) as u32,
                frequency_randomizer: c.get("frequencyRandomizer").number(),
                frequency_randomizer_min: opt_number(&c.get("frequencyRandomizerMin")),
                volume_randomizer: c.get("volumeRandomizer").number(),
                volume_randomizer_min: opt_number(&c.get("volumeRandomizerMin")),
                distance_filter: opt_text(&c.get("distanceFilter")),
                occlusion_factor: opt_number(&c.get("occlusionFactor")),
                obstruction_factor: opt_number(&c.get("obstructionFactor")),
                processing_type: opt_text(&c.get("sound3DProcessingType")),
                position_offset: vec3(&c.get("posOffset").array()),
                delay: c.get("delay").number(),
                delay_randomizer: c.get("delayRandomizer").number(),
                speed_of_sound: opt_number(&c.get("speedOfSound")),
            };
            bank.sets.insert(key(c.name()), set);
        }
        for c in classes(root.get("CfgDistanceFilters")) {
            let filter = DistanceFilterDef {
                name: c.name().to_string(),
                kind: c.get("type").text(),
                min_cutoff_hz: c.get("minCutoffFrequency").number(),
                q: number_or(&c.get("qFactor"), 1.0),
                inner_range: c.get("innerRange").number(),
                range: c.get("range").number(),
                power: number_or(&c.get("powerFactor"), 1.0),
            };
            bank.filters.insert(key(c.name()), filter);
        }
        for c in classes(root.get("CfgSound3DProcessors")) {
            let processor = Processor3d {
                name: c.name().to_string(),
                kind: c.get("type").text(),
                inner_range: c.get("innerRange").number(),
                range: c.get("range").number(),
                radius: opt_number(&c.get("radius")),
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

    /// Resolves a curve reference: named curves come back as stored (x in 0..1), inline points
    /// as given.
    pub fn resolve_curve(&self, curve: &CurveRef) -> Option<Curve> {
        match curve {
            CurveRef::Named(name) => self.curve(name).cloned(),
            CurveRef::Points(points) => Some(Curve::new(points.iter().copied())),
        }
    }
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
