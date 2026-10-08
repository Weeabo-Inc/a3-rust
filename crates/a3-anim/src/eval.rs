//! One animation's transform for a source value.

use a3_p3d::{Animation, AnimationAxis, AnimationTransform, SourceAddress};
use glam::{Affine3A, Mat3A, Vec3, Vec3A};

/// What one animation does to its bone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimTransform {
    /// A model-space transform applied to the bone.
    Matrix(Affine3A),
    /// A `hide` animation in its hidden range: the bone (and its children) are not drawn.
    Hidden,
}

/// Maps a source `value` to the range `a..b` the way the engine does.
///
/// The value is clamped to `minValue..maxValue` and then interpolated linearly from `a` at
/// `minPhase` to `b` at `maxPhase` (outside that range it holds `a` or `b`). With
/// `sourceAddress = loop` the value first wraps into `minPhase..maxPhase`; with `mirror` it
/// runs back and forth over that range.
pub fn interpolate(anim: &Animation, value: f32, a: f32, b: f32) -> f32 {
    let (min_value, max_value) = (anim.min_value, anim.max_value);
    let (min_phase, max_phase) = (anim.min_phase, anim.max_phase);
    let range = max_phase - min_phase;
    let lerp = |v: f32| a + (v - min_phase) / range * (b - a);
    // `t - round(t - 0.5)`: the fractional part as the engine computes it.
    let wrap = |v: f32, period: f32| {
        let t = (v - min_phase) / period;
        (t - (t - 0.5).round_ties_even()) * period + min_phase
    };
    let mut v = value;
    match anim.source_address {
        SourceAddress::Mirror if range != 0.0 => {
            v = wrap(v, 2.0 * range);
            if v > max_phase {
                v = max_phase - (v - max_phase);
            }
            v = clamp(v, min_value, max_value);
            if min_phase < v {
                if v < max_phase { lerp(v) } else { b }
            } else {
                a
            }
        }
        address => {
            if address == SourceAddress::Loop && range != 0.0 {
                v = wrap(v, range);
            }
            v = clamp(v, min_value, max_value);
            if min_phase < v {
                if max_phase <= v { b } else { lerp(v) }
            } else {
                a
            }
        }
    }
}

/// The engine's clamp: `<= min` gives `min`, then `>= max` gives `max`.
fn clamp(v: f32, min: f32, max: f32) -> f32 {
    let v = if v <= min { min } else { v };
    if max <= v { max } else { v }
}

/// A rotation by `angle` (right-handed in the engine's coordinates, as `glam` computes it) about
/// the line through `pos` along `dir`.
fn rotation_about(pos: Vec3, dir: Vec3, angle: f32) -> Affine3A {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return Affine3A::IDENTITY;
    }
    let rotation = Mat3A::from_axis_angle(dir, angle);
    let pos = Vec3A::from(pos);
    Affine3A::from_mat3_translation(rotation.into(), (pos - rotation * pos).into())
}

/// The transform of `anim` in LOD `lod` for source `value`. Identity when the animation does
/// not act on any bone of that LOD.
pub fn animation_transform(anim: &Animation, lod: usize, value: f32) -> AnimTransform {
    let Some(binding) = anim.bindings.get(lod).copied().flatten() else {
        return AnimTransform::Matrix(Affine3A::IDENTITY);
    };
    let (axis_pos, axis_dir) = binding.axis.unwrap_or((Vec3::ZERO, Vec3::ZERO));
    let unit_axis = |axis: AnimationAxis| match axis {
        AnimationAxis::X => Vec3::X,
        AnimationAxis::Y => Vec3::Y,
        AnimationAxis::Z => Vec3::Z,
        AnimationAxis::Custom => axis_dir,
    };
    let matrix = match anim.transform {
        AnimationTransform::Rotation {
            axis,
            angle0,
            angle1,
        } => {
            let angle = interpolate(anim, value, angle0, angle1);
            match axis {
                // Custom axis: +angle about the stored axis direction.
                AnimationAxis::Custom => rotation_about(axis_pos, axis_dir, angle),
                // rotationX/Y/Z: the engine rotates by -angle about the model axis.
                fixed => rotation_about(axis_pos, unit_axis(fixed), -angle),
            }
        }
        AnimationTransform::Translation {
            axis,
            offset0,
            offset1,
        } => {
            let offset = interpolate(anim, value, offset0, offset1);
            // Custom axis: the offset is in units of the (unnormalised) axis vector.
            Affine3A::from_translation(unit_axis(axis) * offset)
        }
        AnimationTransform::Direct {
            axis_pos,
            axis_dir,
            angle,
            axis_offset,
        } => {
            let phase = interpolate(anim, value, 0.0, 1.0);
            let shift =
                Affine3A::from_translation(axis_dir.normalize_or_zero() * axis_offset * phase);
            shift * rotation_about(axis_pos, axis_dir, -angle * phase)
        }
        AnimationTransform::Hide {
            hide_value,
            unhide_value,
        } => {
            let phase = interpolate(anim, value, 0.0, 1.0);
            let shown = phase < hide_value || (0.0 <= unhide_value && unhide_value <= phase);
            return if shown {
                AnimTransform::Matrix(Affine3A::IDENTITY)
            } else {
                AnimTransform::Hidden
            };
        }
    };
    AnimTransform::Matrix(matrix)
}
