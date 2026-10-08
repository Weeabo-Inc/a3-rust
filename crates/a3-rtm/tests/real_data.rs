//! Decodes every RTM of a real game install. Skipped when `A3_ROOT` is unset.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use a3_rtm::{Animation, Encoding};
use a3_vfs::{Vfs, optional_mod_dirs};

#[test]
fn every_rtm_in_the_install_decodes() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let root = Path::new(&root);
    let vfs = Vfs::new();
    vfs.mount_game(root, &optional_mod_dirs(root));

    let paths = vfs.glob("**/*.rtm");
    let mut encodings = BTreeMap::new();
    let mut failures = Vec::new();
    let mut non_unit_files = BTreeSet::new();
    let (mut transforms, mut non_unit) = (0usize, 0usize);
    let (mut frames, mut events) = (0usize, 0usize);
    for path in &paths {
        let data = vfs.open(path.as_str()).unwrap();
        match Animation::read(&data) {
            Ok(anim) => {
                let key = match anim.encoding {
                    Encoding::Plain => "RTM_0101".to_string(),
                    Encoding::Binarized { version } => format!("BMTR v{version}"),
                };
                *encodings.entry(key).or_insert(0) += 1;
                frames += anim.frames.len();
                events += anim.events.len();
                for frame in &anim.frames {
                    assert_eq!(frame.transforms.len(), anim.bones.len(), "{path}");
                    for t in &frame.transforms {
                        transforms += 1;
                        if (t.rotation.length() - 1.0).abs() > 1e-2 {
                            non_unit += 1;
                            non_unit_files.insert(path.to_string());
                        }
                    }
                }
                // Sampling never panics and yields one pose per bone.
                assert_eq!(anim.sample(0.37).len(), anim.bones.len());
            }
            Err(e) => failures.push(format!("{path}: {e}")),
        }
    }

    eprintln!(
        "{} RTM files: {encodings:?}; {frames} frames, {events} events",
        paths.len()
    );
    eprintln!(
        "{non_unit} of {transforms} rotations are not unit length (scaled or degenerate bones), \
         in {} files: {non_unit_files:?}",
        non_unit_files.len()
    );
    for failure in &failures {
        eprintln!("FAIL {failure}");
    }
    assert!(paths.len() > 5000, "expected the full install");
    assert!(failures.is_empty(), "{} failures", failures.len());
    assert!(
        non_unit * 10_000 < transforms,
        "more than 0.01% non-unit rotations"
    );

    // A known character move: the standard rifle walk cycle with footstep events.
    let walk = vfs
        .open(r"a3\anims_f\data\anim\sdr\mov\erc\wlk\ras\rfl\amovpercmwlksraswrfldf.rtm")
        .unwrap();
    let walk = Animation::read(&walk).unwrap();
    assert!(walk.bone_index("pelvis").is_some());
    assert!(walk.bone_index("RightHand").is_some());
    // Forward moves store a negative Z step (about -1.62 m per cycle for this walk).
    assert!(walk.step.z < -1.0, "{:?}", walk.step);
    assert!(walk.events.iter().any(|e| e.name == "StepSound"));
}
