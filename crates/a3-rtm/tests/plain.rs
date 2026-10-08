//! Plain `RTM_0101` files, built byte by byte.

use a3_rtm::{Animation, Encoding, Error};
use glam::{Quat, Vec3};

fn name32(out: &mut Vec<u8>, name: &str) {
    let mut field = [0u8; 32];
    field[..name.len()].copy_from_slice(name.as_bytes());
    out.extend_from_slice(&field);
}

fn floats(out: &mut Vec<u8>, values: &[f32]) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// 4x3 matrix of a rotation by `angle` about +Z (columns aside, up, dir) and a translation.
fn rot_z(angle: f32, t: [f32; 3]) -> [f32; 12] {
    let (s, c) = angle.sin_cos();
    [c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0, t[0], t[1], t[2]]
}

/// Two bones, two frames: `body` stays at identity, `wing` turns 90 degrees and moves up.
fn two_frame_rtm() -> Vec<u8> {
    let mut out = b"RTM_0101".to_vec();
    floats(&mut out, &[0.0, 0.0, -2.0]);
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    name32(&mut out, "body");
    name32(&mut out, "Wing");
    for (phase, angle, y) in [(0.0, 0.0, 0.0), (1.0, std::f32::consts::FRAC_PI_2, 1.0)] {
        floats(&mut out, &[phase]);
        name32(&mut out, "body");
        floats(&mut out, &rot_z(0.0, [0.0; 3]));
        name32(&mut out, "wing");
        floats(&mut out, &rot_z(angle, [0.0, y, 0.0]));
    }
    out
}

#[test]
fn reads_bones_step_and_frames_of_a_plain_rtm() {
    let anim = Animation::read(&two_frame_rtm()).unwrap();

    assert_eq!(anim.encoding, Encoding::Plain);
    assert_eq!(anim.step, Vec3::new(0.0, 0.0, -2.0));
    assert_eq!(anim.bones, ["body", "Wing"]);
    assert_eq!(anim.frames.len(), 2);
    assert_eq!(anim.frames[1].phase, 1.0);

    let wing = anim.frames[1].transforms[1];
    let expected = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    assert!(wing.rotation.abs_diff_eq(expected, 1e-6), "{wing:?}");
    assert_eq!(wing.translation, Vec3::new(0.0, 1.0, 0.0));
}

#[test]
fn bone_lookup_ignores_case() {
    let anim = Animation::read(&two_frame_rtm()).unwrap();
    assert_eq!(anim.bone_index("WING"), Some(1));
    assert_eq!(anim.bone_index("tail"), None);
}

#[test]
fn rejects_a_plain_rtm_cut_short() {
    let bytes = two_frame_rtm();
    let err = Animation::read(&bytes[..bytes.len() - 1]).unwrap_err();
    assert!(matches!(err, Error::Truncated { .. }), "{err:?}");
}

#[test]
fn rejects_unknown_signatures() {
    let err = Animation::read(b"NOTANRTM and some bytes").unwrap_err();
    assert!(matches!(err, Error::UnknownSignature(_)), "{err:?}");
}

#[test]
fn reads_mdat_keystones_before_the_plain_data() {
    let mut out = b"RTM_MDAT".to_vec();
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    floats(&mut out, &[0.25]);
    for text in ["StepSound", "left"] {
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(text.as_bytes());
    }
    out.extend_from_slice(&two_frame_rtm());

    let anim = Animation::read(&out).unwrap();
    assert_eq!(anim.keystones.len(), 1);
    assert_eq!(anim.keystones[0].phase, 0.25);
    assert_eq!(anim.keystones[0].name, "StepSound");
    assert_eq!(anim.keystones[0].value, "left");
    assert_eq!(anim.keystones[0].kind, -1, "looked up by name");
    assert_eq!(anim.frames.len(), 2);
}
