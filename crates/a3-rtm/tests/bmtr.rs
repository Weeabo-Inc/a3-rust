//! Binarized `BMTR` files, built byte by byte.

use a3_rtm::{Animation, BoneTransform, Encoding, Error};
use glam::{Quat, Vec3};

const LZO: u8 = 2;

/// One bone transform as stored: quaternion x, y, z, w as i16 / 16384, translation as f16.
fn transform(q: [i16; 4], t: [u16; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    for v in q {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in t {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// A compressed array: count, flag, then the bytes (wrapped in an LZO1X literal block for LZO).
fn array(out: &mut Vec<u8>, count: u32, flag: u8, bytes: &[u8]) {
    out.extend_from_slice(&count.to_le_bytes());
    out.push(flag);
    if flag == LZO {
        // One literal run (first byte 17 + length, for lengths up to 238) and the end marker.
        out.push(17 + u8::try_from(bytes.len()).unwrap());
        out.extend_from_slice(bytes);
        out.extend_from_slice(&[0x11, 0, 0]);
    } else {
        out.extend_from_slice(bytes);
    }
}

#[derive(Default)]
struct Builder {
    extra_names: Vec<&'static str>,
    keystones: Vec<(i32, &'static str, f32)>,
    /// Compression flag of every array: 0 raw (the default), 2 LZO.
    flag: u8,
}

impl Builder {
    fn build(&self) -> Vec<u8> {
        let mut out = b"BMTR".to_vec();
        out.extend_from_slice(&5u32.to_le_bytes());
        out.push(1);
        for v in [0.0f32, 0.0, 1.5] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&2u32.to_le_bytes()); // phases
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes()); // bones of the first phase
        out.extend_from_slice(&2u32.to_le_bytes()); // bone names
        out.extend_from_slice(b"pelvis\0spine\0");
        out.extend_from_slice(&(self.extra_names.len() as u32).to_le_bytes());
        for name in &self.extra_names {
            out.extend_from_slice(name.as_bytes());
            out.push(0);
        }
        out.extend_from_slice(&(self.keystones.len() as u32).to_le_bytes());
        for (kind, name, phase) in &self.keystones {
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            out.extend_from_slice(&phase.to_le_bytes());
            out.push(0);
        }
        let times: Vec<u8> = [0.0f32, 1.0].iter().flat_map(|t| t.to_le_bytes()).collect();
        array(&mut out, 2, self.flag, &times);
        // Phase 0: both bones at identity. Phase 1: spine turned 90 degrees about +Y, raised 1.
        let identity = transform([0, 0, 0, 16384], [0; 3]);
        let turned = transform([0, 11585, 0, 11585], [0, 0x3c00, 0]);
        array(
            &mut out,
            2,
            self.flag,
            &[identity.clone(), identity.clone()].concat(),
        );
        array(&mut out, 2, self.flag, &[identity, turned].concat());
        out
    }
}

fn plain() -> Builder {
    Builder::default()
}

#[test]
fn reads_header_bones_and_phases() {
    let anim = Animation::read(&plain().build()).unwrap();
    assert_eq!(anim.encoding, Encoding::Binarized { version: 5 });
    assert_eq!(anim.step, Vec3::new(0.0, 0.0, 1.5));
    assert_eq!(anim.bones, ["pelvis", "spine"]);
    let phases: Vec<f32> = anim.frames.iter().map(|f| f.phase).collect();
    assert_eq!(phases, [0.0, 1.0]);
}

#[test]
fn decodes_quantized_quaternions_and_half_float_translations() {
    let anim = Animation::read(&plain().build()).unwrap();
    assert_eq!(anim.frames[0].transforms[1], BoneTransform::IDENTITY);
    let spine = anim.frames[1].transforms[1];
    let expected = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    assert!(spine.rotation.abs_diff_eq(expected, 1e-4), "{spine:?}");
    assert_eq!(spine.translation, Vec3::new(0.0, 1.0, 0.0));
}

#[test]
fn reads_lzo_compressed_arrays() {
    let compressed = Builder {
        flag: LZO,
        ..Builder::default()
    };
    assert_eq!(
        Animation::read(&compressed.build()).unwrap(),
        Animation::read(&plain().build()).unwrap()
    );
}

#[test]
fn reads_step_sound_keystones() {
    let builder = Builder {
        keystones: vec![(0, "StepSound", 0.25), (-1, "StepSound", 0.75)],
        ..Builder::default()
    };
    let anim = Animation::read(&builder.build()).unwrap();
    let keystones: Vec<(i32, &str, f32)> = anim
        .keystones
        .iter()
        .map(|k| (k.kind, k.name.as_str(), k.phase))
        .collect();
    assert_eq!(keystones, [(0, "StepSound", 0.25), (-1, "StepSound", 0.75)]);
    assert_eq!(anim.frames.len(), 2);
}

#[test]
fn reads_the_name_list_before_the_keystones() {
    let builder = Builder {
        extra_names: vec!["first", "second"],
        keystones: vec![(0, "StepSound", 0.5)],
        ..Builder::default()
    };
    let anim = Animation::read(&builder.build()).unwrap();
    assert_eq!(anim.extra_names, ["first", "second"]);
    assert_eq!(anim.keystones.len(), 1);
}

#[test]
fn rejects_other_versions() {
    let mut bytes = plain().build();
    bytes[4] = 3;
    assert!(matches!(
        Animation::read(&bytes),
        Err(Error::UnsupportedVersion(3))
    ));
}

#[test]
fn rejects_unknown_compression_flags() {
    let bytes = Builder {
        flag: 7,
        ..Builder::default()
    }
    .build();
    assert!(matches!(Animation::read(&bytes), Err(Error::Malformed(_))));
}

#[test]
fn rejects_truncated_files_at_every_length() {
    let bytes = plain().build();
    for len in 0..bytes.len() {
        assert!(Animation::read(&bytes[..len]).is_err(), "length {len}");
    }
}

#[test]
fn reads_the_step_bones_and_keystones_without_the_frames() {
    let bytes = Builder {
        keystones: vec![(0, "StepSound", 0.25)],
        ..Builder::default()
    }
    .build();
    // Phase times (4 + 1 + 2 * 4 bytes) and two frames (4 + 1 + 2 * 14 bytes each) follow the
    // header; cut inside the phase times.
    let header_len = bytes.len() - 13 - 2 * 33;

    let header = Animation::read_header(&bytes[..header_len + 3]).unwrap();

    assert_eq!(header.step, Vec3::new(0.0, 0.0, 1.5));
    assert_eq!(header.bones, ["pelvis", "spine"]);
    let keystones: Vec<(&str, f32)> = header
        .keystones
        .iter()
        .map(|k| (k.name.as_str(), k.phase))
        .collect();
    assert_eq!(keystones, [("StepSound", 0.25)]);
}

#[test]
fn samples_between_keyframes() {
    let anim = Animation::read(&plain().build()).unwrap();
    let spine = anim.bone_index("Spine").unwrap();

    let half = anim.sample_bone(spine, 0.5);
    let expected = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
    assert!(half.rotation.abs_diff_eq(expected, 1e-4), "{half:?}");
    assert!(half.translation.abs_diff_eq(Vec3::new(0.0, 0.5, 0.0), 1e-6));

    assert_eq!(
        anim.sample_bone(spine, -1.0),
        anim.frames[0].transforms[spine]
    );
    assert_eq!(
        anim.sample_bone(spine, 2.0),
        anim.frames[1].transforms[spine]
    );
    assert_eq!(anim.sample(0.5).len(), 2);
}
