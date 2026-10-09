//! Listener, emitters and the 3D processing of one voice: distance gain, panning, doppler,
//! distance low-pass, occlusion.
//!
//! World space is RV's (ADR 0003): left-handed, X east, Y up, Z north, positions in `f64`.

use glam::{DVec3, Vec3};

use crate::Curve;

/// Speed of sound used for doppler, in m/s (the engine's X3DAudio setting).
pub const SPEED_OF_SOUND: f32 = 343.5;

/// The listener (usually the camera).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    /// World position.
    pub position: DVec3,
    /// Unit vector the listener faces.
    pub forward: Vec3,
    /// Unit vector pointing up from the listener's head.
    pub up: Vec3,
    /// Velocity in m/s (for doppler).
    pub velocity: Vec3,
}

impl Default for Listener {
    /// At the origin, looking north (+Z), up +Y, at rest.
    fn default() -> Self {
        Self {
            position: DVec3::ZERO,
            forward: Vec3::Z,
            up: Vec3::Y,
            velocity: Vec3::ZERO,
        }
    }
}

impl Listener {
    /// The listener's right-hand direction. In RV's left-handed space this is `up x forward`
    /// (looking north, right is east).
    pub fn right(&self) -> Vec3 {
        self.up.cross(self.forward).normalize_or_zero()
    }
}

/// A distance low-pass (`CfgDistanceFilters`): the cutoff falls from the sample rate at
/// `inner_range` to `min_cutoff_hz` at `range`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistanceFilter {
    /// Cutoff at and beyond `range`, in Hz.
    pub min_cutoff_hz: f32,
    /// `qFactor`, used as the filter's `1/Q`.
    pub q: f32,
    /// Distance up to which the filter is open, in metres.
    pub inner_range: f32,
    /// Distance at which the cutoff reaches `min_cutoff_hz`, in metres.
    pub range: f32,
    /// Shape of the fall-off: higher values close the filter faster near the source.
    pub power: f32,
}

impl DistanceFilter {
    /// Cutoff frequency at `distance` metres for a voice at `sample_rate`, as the engine
    /// computes it: the sample rate inside `inner_range`, `min_cutoff_hz` beyond `range`, and in
    /// between `fs + (min - fs) * t` with `t = ((d - inner) / (range - inner))^(1 / power)`.
    pub fn cutoff_at(&self, distance: f32, sample_rate: f32) -> f32 {
        if distance > self.range {
            return self.min_cutoff_hz;
        }
        if distance < self.inner_range {
            return sample_rate;
        }
        let span = self.range - self.inner_range;
        let t = if span > 0.0 {
            ((distance - self.inner_range) / span).powf(1.0 / self.power.max(1e-4))
        } else {
            1.0
        };
        sample_rate + (self.min_cutoff_hz - sample_rate) * t
    }
}

/// How a voice is placed in the world.
#[derive(Debug, Clone, PartialEq)]
pub struct Emitter {
    /// World position.
    pub position: DVec3,
    /// Velocity in m/s (for doppler).
    pub velocity: Vec3,
    /// Gain over distance in metres.
    pub attenuation: Curve,
    /// Within this distance (metres) the sound surrounds the listener: panning fades to the
    /// centre as the distance falls to 0 (the sound set's `spatialityRange`, X3DAudio's inner
    /// radius).
    pub spread_radius: f32,
    /// Doppler strength: 0 off, 1 physical.
    pub doppler: f32,
    /// Optional distance low-pass.
    pub distance_filter: Option<DistanceFilter>,
    /// Occlusion by geometry between source and listener, 0 (none) to 1 (full). A hook for the
    /// world: it lowers the gain and closes a low-pass.
    pub occlusion: f32,
    /// Obstruction of the direct path only, 0 to 1 (hook, as `occlusion` but milder).
    pub obstruction: f32,
}

impl Emitter {
    /// An emitter at `position` with the given attenuation and everything else neutral.
    pub fn at(position: DVec3, attenuation: Curve) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
            attenuation,
            spread_radius: 0.5,
            doppler: 1.0,
            distance_filter: None,
            occlusion: 0.0,
            obstruction: 0.0,
        }
    }
}

/// The result of 3D processing for one voice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spatialized {
    /// Gain of the left and right output channels (attenuation and panning).
    pub gains: [f32; 2],
    /// Pitch factor from doppler.
    pub pitch: f32,
    /// Low-pass cutoff to apply, if any.
    pub cutoff_hz: Option<f32>,
    /// `1/Q` of that low-pass.
    pub filter_q: f32,
    /// Distance between listener and emitter in metres.
    pub distance: f32,
}

/// Equal-power stereo gains for `pan` from -1 (left) to 1 (right).
pub fn equal_power_pan(pan: f32) -> [f32; 2] {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    [angle.cos(), angle.sin()]
}

/// Doppler pitch factor for a source seen along `direction` (unit, listener to source):
/// `(c + v_listener . u) / (c + v_source . u)`, limited to 0.5..2 as in the engine.
pub fn doppler_factor(listener_velocity: Vec3, source_velocity: Vec3, direction: Vec3) -> f32 {
    let c = SPEED_OF_SOUND;
    let toward_source = listener_velocity.dot(direction);
    let away_from_listener = source_velocity.dot(direction);
    ((c + toward_source) / (c + away_from_listener).max(1.0)).clamp(0.5, 2.0)
}

// Our own HRTF-less spatial cues (not from the engine, which leaves this to X3DAudio).
/// Gain lost behind the listener's head.
const REAR_GAIN: f32 = 0.8;
/// Low-pass applied to sources directly behind the listener, in Hz.
const REAR_CUTOFF_HZ: f32 = 5_000.0;
/// Low-pass at full occlusion, in Hz.
const OCCLUDED_CUTOFF_HZ: f32 = 800.0;

/// 3D processing of an emitter heard by a listener, for a voice mixed at `sample_rate`.
pub fn spatialize(listener: &Listener, emitter: &Emitter, sample_rate: f32) -> Spatialized {
    let offset = (emitter.position - listener.position).as_vec3();
    let distance = offset.length();
    let direction = if distance > 1e-4 {
        offset / distance
    } else {
        Vec3::ZERO
    };

    let mut gain = emitter.attenuation.eval(distance).max(0.0);
    gain *= 1.0 - 0.7 * emitter.occlusion.clamp(0.0, 1.0);
    gain *= 1.0 - 0.3 * emitter.obstruction.clamp(0.0, 1.0);

    // Pan by the sideways component; fade to the centre inside the spread radius.
    let side = direction.dot(listener.right());
    let front = direction.dot(listener.forward);
    let spread = if emitter.spread_radius > 0.0 {
        (distance / emitter.spread_radius).min(1.0)
    } else {
        1.0
    };
    let pan_gains = equal_power_pan(side * spread);
    let behind = (-front).max(0.0) * spread;
    let rear = 1.0 - (1.0 - REAR_GAIN) * behind;
    let gains = pan_gains.map(|g| g * gain * rear);

    let pitch = if emitter.doppler > 0.0 && distance > 1e-4 {
        let physical = doppler_factor(listener.velocity, emitter.velocity, direction);
        1.0 + (physical - 1.0) * emitter.doppler
    } else {
        1.0
    };

    let open = sample_rate;
    let mut cutoff = emitter
        .distance_filter
        .map_or(open, |f| f.cutoff_at(distance, sample_rate));
    // Cues interpolate in octaves from fully open (a sixth of the rate, where the filter
    // switches off) down to their cutoff.
    let toward = |target: f32, amount: f32| (open / 6.0) * (target / (open / 6.0)).powf(amount);
    if behind > 0.0 {
        cutoff = cutoff.min(toward(REAR_CUTOFF_HZ, behind));
    }
    let occluded = (emitter.occlusion + 0.5 * emitter.obstruction).clamp(0.0, 1.0);
    if occluded > 0.0 {
        cutoff = cutoff.min(toward(OCCLUDED_CUTOFF_HZ, occluded));
    }

    Spatialized {
        gains,
        pitch,
        cutoff_hz: (cutoff * 6.0 < sample_rate).then_some(cutoff),
        filter_q: emitter.distance_filter.map_or(1.0, |f| f.q),
        distance,
    }
}
