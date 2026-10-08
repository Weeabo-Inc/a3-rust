//! Probes every sound file of a real game install and decodes a sample of them. Skipped when
//! `A3_ROOT` is unset. Set `A3_DECODE_ALL=1` to fully decode every Ogg file too (slow).

use std::collections::BTreeMap;
use std::path::Path;

use a3_audio_formats::{Format, decode, probe};
use a3_vfs::{Vfs, optional_mod_dirs};

#[test]
fn every_sound_in_the_install_probes_and_wss_decodes() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let decode_all = std::env::var_os("A3_DECODE_ALL").is_some();
    let root = Path::new(&root);
    let vfs = Vfs::new();
    vfs.mount_game(root, &optional_mod_dirs(root));

    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut decoded = 0usize;
    let mut full_scale: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for pattern in ["**/*.wss", "**/*.ogg", "**/*.wav"] {
        let paths = vfs.glob(pattern);
        eprintln!("{pattern}: {} files", paths.len());
        for (i, path) in paths.iter().enumerate() {
            let data = vfs.open(path.as_str()).unwrap();
            let info = match probe(&data) {
                Ok(info) => info,
                Err(e) => {
                    failures.push(format!("{path}: {e}"));
                    continue;
                }
            };
            let key = format!(
                "{:?} {}ch {}Hz {:?}bit",
                info.format, info.channels, info.sample_rate, info.bits_per_sample
            );
            *kinds.entry(key).or_default() += 1;

            let is_wss = matches!(info.format, Format::Wss(_));
            if is_wss || decode_all || i % 2000 == 0 {
                match decode(&data) {
                    Ok(sound) => {
                        decoded += 1;
                        assert_eq!(sound.channels, info.channels, "{path}");
                        assert_eq!(Some(sound.frames() as u64), info.frames, "{path}");
                        let clipped = sound.samples.iter().any(|&s| s == i16::MAX || s == i16::MIN);
                        if is_wss {
                            let entry = full_scale.entry(format!("{:?}", info.format)).or_default();
                            entry.0 += usize::from(clipped);
                            entry.1 += 1;
                        }
                    }
                    Err(e) => failures.push(format!("{path}: decode: {e}")),
                }
            }
        }
    }

    for (kind, count) in &kinds {
        eprintln!("{count:>7}  {kind}");
    }
    eprintln!("fully decoded {decoded}; WSS files reaching full scale (of all): {full_scale:?}");
    for failure in &failures {
        eprintln!("FAIL {failure}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
    assert!(kinds.values().sum::<usize>() > 200_000, "expected the full install");
}
