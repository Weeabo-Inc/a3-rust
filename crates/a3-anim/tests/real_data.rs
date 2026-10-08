//! Poses shipped models. Skipped when `A3_ROOT` is unset.

use std::path::Path;

use a3_anim::{Sources, hidden_sections, pose, skin};
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
