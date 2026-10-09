//! Playing a config sound set: one voice per sound shader, with gain and pitch driven by the
//! shaders' expressions, combined as the engine does (`docs/re/audio.md` section 2).
//!
//! - Shader volume: its `volume` (a constant already includes the shader's `volumeFactor`),
//!   times the set's `volume` expression, times the distance gain of 3D sets:
//!   `rangeCurve(d / range)` (1 without a curve), 0 beyond `range`.
//! - `soundShadersLimit`: shaders with `limitation = 1` above -50 dB are sorted loudest first
//!   and only the first N play; other shaders play whenever they are above -50 dB.
//! - Voice gain: shader volume times the set volume drawn at start
//!   (`volumeFactor * 10^(±randomizer dB / 20)`); pitch: shader `frequency` times
//!   `frequencyFactor * 2^(±randomizer semitones / 12)`.

use crate::config::{SoundBank, SoundSet, SoundShader, db_to_linear};
use crate::{AudioEngine, Curve, PlayParams, SoundLoader, VoiceId};

/// Shaders below this volume (-50 dB) do not play.
pub const AUDIBLE_THRESHOLD: f32 = 0.003_162_277_7;

/// The engine's random generator: an LCG returning values in `0..1`.
#[derive(Debug, Clone)]
pub struct Rng(u32);

impl Rng {
    /// A generator from a seed.
    pub fn new(seed: u32) -> Self {
        Self(seed & 0x7fff_ffff)
    }

    /// A uniform value in `0..1`.
    pub fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12_345) & 0x7fff_ffff;
        self.0 as f32 * 4.656_613e-10
    }
}

/// Picks a sample index the engine's way: probabilities are normalised to sum 1, a uniform
/// draw walks down them, and a repeat of `previous` moves on to the next sample.
pub fn choose_sample(weights: &[f32], previous: Option<usize>, rng: &mut Rng) -> Option<usize> {
    if weights.is_empty() {
        return None;
    }
    let total: f32 = weights.iter().map(|w| w.max(0.0)).sum();
    let mut r = rng.next_f32();
    let mut index = weights.len() - 1;
    for (i, w) in weights.iter().enumerate() {
        let p = if total > 0.0 {
            w.max(0.0) / total
        } else {
            1.0 / weights.len() as f32
        };
        r -= p;
        if r < 0.0 {
            index = i;
            break;
        }
    }
    if previous == Some(index) {
        index = (index + 1) % weights.len();
    }
    Some(index)
}

/// `±` a random amount between `min` and `max`, sign chosen at random.
fn signed_random(max: f32, min: f32, rng: &mut Rng) -> f32 {
    let magnitude = min + rng.next_f32() * (max - min);
    if rng.next_f32() > 0.5 {
        magnitude
    } else {
        -magnitude
    }
}

fn lin_to_db(x: f32) -> f32 {
    (20.0 * (x + 1e-25).log10()).max(-100.0)
}

/// The volume and pitch a set starts with: `volumeFactor` and `frequencyFactor` with their
/// random offsets.
pub fn start_levels(set: &SoundSet, rng: &mut Rng) -> (f32, f32) {
    let db = signed_random(
        lin_to_db(set.volume_randomizer),
        lin_to_db(set.volume_randomizer_min),
        rng,
    );
    let semitones = signed_random(set.frequency_randomizer, set.frequency_randomizer_min, rng);
    (
        db_to_linear(db) * set.volume_factor,
        2f32.powf(semitones / 12.0) * set.frequency_factor,
    )
}

/// The distance gain of one shader of a 3D set at `distance` metres: `curve(d / range)`
/// (1 without a curve) within `range`, 0 beyond it.
pub fn shader_distance_gain(range: f32, curve: Option<&Curve>, distance: f32) -> f32 {
    if distance > range {
        return 0.0;
    }
    match curve {
        Some(curve) if range > 0.0 => curve.eval(distance / range),
        _ => 1.0,
    }
}

/// Applies `soundShadersLimit` to shader volumes: returns, per shader, whether it plays.
pub fn audible_shaders(volumes: &[f32], limited: &[bool], limit: u32) -> Vec<bool> {
    let mut plays: Vec<bool> = volumes.iter().map(|&v| v > AUDIBLE_THRESHOLD).collect();
    if limit == 0 {
        return plays;
    }
    let mut candidates: Vec<usize> = (0..volumes.len())
        .filter(|&i| limited[i] && plays[i])
        .collect();
    candidates.sort_by(|&a, &b| volumes[b].total_cmp(&volumes[a]));
    for &i in candidates.iter().skip(limit as usize) {
        plays[i] = false;
    }
    plays
}

struct ShaderVoice {
    shader: SoundShader,
    range_curve: Option<Curve>,
    voice: Option<VoiceId>,
}

/// A playing sound set.
pub struct SoundSetPlayer {
    set: SoundSet,
    volume: f32,
    frequency: f32,
    voices: Vec<ShaderVoice>,
}

impl SoundSetPlayer {
    /// Starts every shader of the set `name` as a looping 2D voice at gain 0 (call
    /// [`SoundSetPlayer::update`] to set the levels). Shaders without a playable sample are
    /// skipped; unknown shader names are ignored. `None` when the set is unknown.
    pub fn start_loop(
        bank: &SoundBank,
        loader: &SoundLoader,
        engine: &AudioEngine,
        name: &str,
        rng: &mut Rng,
    ) -> Option<Self> {
        let set = bank.set(name)?.clone();
        let (volume, frequency) = start_levels(&set, rng);
        let mut voices = Vec::new();
        for shader_name in &set.shaders {
            let Some(shader) = bank.shader(shader_name) else {
                continue;
            };
            let weights: Vec<f32> = shader.samples.iter().map(|s| s.probability).collect();
            let voice = choose_sample(&weights, None, rng)
                .and_then(|i| loader.source(&shader.samples[i].path, true))
                .map(|source| {
                    engine.play(
                        source,
                        PlayParams {
                            gain: 0.0,
                            looping: true,
                            ..PlayParams::default()
                        },
                    )
                });
            voices.push(ShaderVoice {
                range_curve: shader
                    .range_curve
                    .as_ref()
                    .and_then(|c| bank.resolve_curve(c)),
                shader: shader.clone(),
                voice,
            });
        }
        Some(Self {
            set,
            volume,
            frequency,
            voices,
        })
    }

    /// The set's class name.
    pub fn name(&self) -> &str {
        &self.set.name
    }

    /// Per-shader volumes for the variables `vars` (and, for 3D sets, `distance` metres), before
    /// the set's start volume and the shader limit.
    pub fn shader_volumes(
        &self,
        vars: &impl Fn(&str) -> Option<f32>,
        distance: Option<f32>,
    ) -> Vec<f32> {
        let set_volume = self.set.volume.eval(vars);
        self.voices
            .iter()
            .map(|v| {
                let mut volume = v.shader.volume.eval(vars) * set_volume;
                if let Some(d) = distance.filter(|_| self.set.spatial) {
                    volume *= shader_distance_gain(v.shader.range, v.range_curve.as_ref(), d);
                }
                volume.max(0.0)
            })
            .collect()
    }

    /// Re-evaluates the shaders with `vars` and applies gains (times `gain`) and pitches.
    /// Returns the loudest voice gain.
    pub fn update(
        &self,
        engine: &AudioEngine,
        vars: &impl Fn(&str) -> Option<f32>,
        gain: f32,
    ) -> f32 {
        let volumes = self.shader_volumes(vars, None);
        let limited: Vec<bool> = self.voices.iter().map(|v| v.shader.limitation).collect();
        let plays = audible_shaders(&volumes, &limited, self.set.shaders_limit);
        let mut loudest = 0.0f32;
        for ((v, &volume), &play) in self.voices.iter().zip(&volumes).zip(&plays) {
            let voice_gain = if play {
                volume * self.volume * gain
            } else {
                0.0
            };
            loudest = loudest.max(voice_gain);
            if let Some(id) = v.voice {
                engine.set_gain(id, voice_gain);
                let pitch = v.shader.frequency.eval(vars) * self.frequency;
                engine.set_pitch(id, pitch.clamp(0.05, 8.0));
            }
        }
        loudest
    }

    /// Fades every voice out over `fade` seconds.
    pub fn stop(&self, engine: &AudioEngine, fade: f32) {
        for id in self.voices.iter().filter_map(|v| v.voice) {
            engine.stop(id, fade);
        }
    }

    /// Number of shaders that have a playing voice.
    pub fn playing_shaders(&self) -> usize {
        self.voices.iter().filter(|v| v.voice.is_some()).count()
    }
}
