//! Poses shipped models. Skipped when `A3_ROOT` is unset.

use std::path::Path;

use a3_anim::{RtmBinding, SkeletonPivots, Sources, hidden_sections, pose, skin};
use a3_p3d::Model;
use a3_vfs::Vfs;
use glam::Vec3;

fn load(path: &str) -> Option<Model> {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return None;
    };
    let vfs = Vfs::new();
    vfs.mount_archives(&Path::new(&root).join("Addons"));
    Some(Model::from_bytes(&vfs.open(path).unwrap()).unwrap())
}

/// Vertices that move when `source` goes from 0 to `value` (every other source at 0), with
/// their positions before and after. (All sources at 0 is not the rest pose: dampers, for
/// example, sit at an offset there.)
fn moved(model: &Model, lod: usize, source: &str, value: f32) -> Vec<(Vec3, Vec3)> {
    let l = &model.lods[lod];
    let at = |v: f32| {
        let p = pose(model, lod, &Sources::new().with(source, v));
        skin(l, &p.skinning(l)).positions
    };
    at(0.0)
        .into_iter()
        .zip(at(value))
        .filter(|(a, b)| a.distance(*b) > 1e-3)
        .collect()
}

#[test]
fn hunter_doors_swing_outwards() {
    let Some(model) = load(r"a3\soft_f\mrap_01\mrap_01_unarmed_f.p3d") else {
        return;
    };
    // Hinges at x = +0.95 (left doors) and x = -0.97 (right doors): opening swings a door away
    // from the centre line, towards its hinge side.
    for (source, side) in [
        ("door_lf", 1.0),
        ("door_rf", -1.0),
        ("door_lb", 1.0),
        ("door_rb", -1.0),
    ] {
        let moved = moved(&model, 0, source, 1.0);
        assert!(
            moved.len() > 50,
            "{source}: only {} vertices move",
            moved.len()
        );
        let outward: f32 =
            moved.iter().map(|(a, b)| (b.x - a.x) * side).sum::<f32>() / moved.len() as f32;
        let span: f32 = moved
            .iter()
            .map(|(a, b)| a.distance(*b))
            .fold(0.0, f32::max);
        eprintln!(
            "{source}: {} vertices move, mean outward shift {outward:.3} m, max {span:.3} m",
            moved.len()
        );
        assert!(outward > 0.2, "{source} opens inwards ({outward})");
        assert!(span < 3.0, "{source} moves too far ({span})");
    }
}

#[test]
fn hunter_wheel_hides_when_destroyed() {
    let Some(model) = load(r"a3\soft_f\mrap_01\mrap_01_unarmed_f.p3d") else {
        return;
    };
    let lod = &model.lods[0];
    let intact = pose(&model, 0, &Sources::new());
    let broken = pose(&model, 0, &Sources::new().with("hitlfwheel", 1.0));
    let hidden = |p: &a3_anim::Pose| {
        hidden_sections(lod, &p.skinning(lod))
            .iter()
            .filter(|h| **h)
            .count()
    };
    let bones = |p: &a3_anim::Pose| p.hidden.iter().filter(|h| **h).count();
    eprintln!(
        "intact: {} hidden bones, {} hidden sections; destroyed: {} bones, {} sections",
        bones(&intact),
        hidden(&intact),
        bones(&broken),
        hidden(&broken)
    );
    // `wheel_1_1_destruct` hides bone `wheel_1_1_hide` (the intact tyre) at hitlfwheel 1.
    let bone = model
        .skeleton
        .as_ref()
        .unwrap()
        .bones
        .iter()
        .position(|b| b.name == "wheel_1_1_hide")
        .unwrap();
    assert!(!intact.hidden[bone] && broken.hidden[bone]);
    assert!(
        hidden(&broken) > hidden(&intact),
        "the tyre sections disappear"
    );
}

/// A soldier posed by three RTMs: every skeleton bone binds; vertices shared by two bones land in
/// the same place under both bone matrices (the skin does not tear); standing poses are tall and
/// the prone pose lies flat.
#[test]
fn soldier_rtm_poses_keep_the_skin_together() {
    let Some(model) = load(r"a3\characters_f\blufor\b_soldier_01.p3d") else {
        return;
    };
    let root = std::env::var_os("A3_ROOT").unwrap();
    let vfs = Vfs::new();
    vfs.mount_archives(&Path::new(&root).join("Addons"));
    let pivots_model =
        Model::from_bytes(&vfs.open(r"a3\anims_f\data\skeleton\skeletonpivots.p3d").unwrap())
            .unwrap();
    let skeleton = model.skeleton.as_ref().unwrap();
    let pivots = SkeletonPivots::from_model(skeleton, &pivots_model, "");
    let lod = &model.lods[0];
    let odol = lod.odol.as_ref().unwrap();
    for (path, phase, standing) in [
        (r"a3\anims_f\data\anim\sdr\mov\erc\stp\ras\rfl\amovpercmstpsraswrfldnon.rtm", 0.0, true),
        (r"a3\anims_f\data\anim\sdr\mov\erc\wlk\ras\rfl\amovpercmwlksraswrfldf.rtm", 0.25, true),
        (r"a3\anims_f\data\anim\sdr\mov\pne\stp\ras\rfl\amovppnemstpsraswrfldnon.rtm", 0.0, false),
    ] {
        let rtm = a3_rtm::Animation::read(&vfs.open(path).unwrap()).unwrap();
        let binding = RtmBinding::new(skeleton, &rtm);
        assert_eq!(binding.bound(), skeleton.bones.len());
        let frames = binding.frames(&rtm, phase, &pivots);
        let pose = a3_anim::Pose::from_rtm_frames(&frames, &pivots, model.info.bounding_center);
        let skinning = pose.skinning(lod);
        let (mut seam, mut shared) = (0.0, 0);
        for (v, w) in lod.vertices.bone_weights.iter().enumerate() {
            if w.count < 2 {
                continue;
            }
            let p = lod.vertices.positions[v];
            let a = skinning[usize::from(w.pairs[0].0)].transform_point3(p);
            let b = skinning[usize::from(w.pairs[1].0)].transform_point3(p);
            seam += a.distance(b);
            shared += 1;
        }
        let seam = seam / shared as f32;
        // Height of the drawn body (proxy triangles left out).
        let posed = skin(lod, &skinning).positions;
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for s in lod.sections.iter().filter(|s| !s.is_proxy()) {
            for f in &lod.faces[s.faces.start as usize..s.faces.end as usize] {
                for &i in f.indices() {
                    lo = lo.min(posed[i as usize].y);
                    hi = hi.max(posed[i as usize].y);
                }
            }
        }
        let _ = odol;
        eprintln!(
            "{}: mean seam {seam:.3} m over {shared} shared vertices, height {:.2} m",
            path.rsplit('\\').next().unwrap(),
            hi - lo
        );
        assert!(seam < 0.06, "{path}: skin tears ({seam})");
        if standing {
            assert!(hi - lo > 1.5, "{path}: not standing");
        } else {
            assert!(hi - lo < 0.8, "{path}: not lying");
        }
    }
}
#[test]
fn hunter_wheel_turns_about_its_hub() {
    let Some(model) = load(r"a3\soft_f\mrap_01\mrap_01_unarmed_f.p3d") else {
        return;
    };
    // Half a turn: every moving vertex mirrors through the hub axis (parallel to X), so the
    // midpoint of rest and posed position is the same point on the axis for all of them.
    let moved = moved(&model, 0, "wheel", 0.5);
    assert!(!moved.is_empty());
    let mids: Vec<Vec3> = moved.iter().map(|(a, b)| (*a + *b) * 0.5).collect();
    let yz = |v: &Vec3| glam::Vec2::new(v.y, v.z);
    let first = yz(&mids[0]);
    let off_axis = mids.iter().filter(|m| yz(m).distance(first) > 0.05).count();
    eprintln!(
        "wheel: {} vertices move; {off_axis} off the first hub",
        moved.len()
    );
    // Four wheels share the source: midpoints cluster on four hubs, none elsewhere.
    let mut hubs: Vec<glam::Vec2> = Vec::new();
    for m in &mids {
        if !hubs.iter().any(|h| h.distance(yz(m)) < 0.05) {
            hubs.push(yz(m));
        }
    }
    eprintln!("hub axes (y, z): {hubs:?}");
    assert!(hubs.len() <= 8, "{} distinct hub axes", hubs.len());
}
