//! Verifies every signed PBO of a real game install against the shipped keys. Skipped when
//! `A3_ROOT` is unset.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use a3_pbo::Pbo;
use a3_signing::{PublicKey, Signature, verify};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

#[test]
fn every_vanilla_pbo_verifies_against_its_key() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let mut files = Vec::new();
    walk(Path::new(&root), &mut files);

    let keys: Vec<PublicKey> = files
        .iter()
        .filter(|p| has_extension(p, "bikey"))
        .map(|p| PublicKey::read(&std::fs::read(p).unwrap()).unwrap())
        .collect();
    assert!(keys.iter().any(|k| k.authority == "a3"), "Keys/a3.bikey");

    // Every signature file parses, including those of encrypted creator-DLC archives.
    let all_signatures = files.iter().filter(|p| has_extension(p, "bisign")).count();
    for path in files.iter().filter(|p| has_extension(p, "bisign")) {
        Signature::read(&std::fs::read(path).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
    eprintln!("{all_signatures} .bisign files parse");

    let mut by_key: BTreeMap<String, usize> = BTreeMap::new();
    let (mut verified, mut unsigned, mut unknown_key) = (0, Vec::new(), Vec::new());
    let mut failures = Vec::new();
    for pbo_path in files.iter().filter(|p| has_extension(p, "pbo")) {
        let file_name = pbo_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_lowercase();
        let signatures: Vec<&PathBuf> = files
            .iter()
            .filter(|p| {
                let name = p.file_name().unwrap().to_string_lossy().to_lowercase();
                p.parent() == pbo_path.parent()
                    && name.starts_with(&format!("{file_name}."))
                    && name.ends_with(".bisign")
            })
            .collect();
        if signatures.is_empty() {
            unsigned.push(pbo_path.clone());
            continue;
        }
        let pbo = Pbo::open(pbo_path).unwrap();
        for sig_path in signatures {
            let signature = Signature::read(&std::fs::read(sig_path).unwrap()).unwrap();
            let Some(key) = keys.iter().find(|k| k.modulus == signature.key.modulus) else {
                unknown_key.push(sig_path.clone());
                continue;
            };
            match verify(key, &signature, &pbo) {
                Ok(()) => {
                    verified += 1;
                    *by_key
                        .entry(format!("{} v{}", key.authority, signature.version.number()))
                        .or_default() += 1;
                }
                Err(e) => failures.push(format!("{}: {e}", sig_path.display())),
            }
        }
    }

    eprintln!(
        "{} keys; {verified} signatures verified {by_key:?}; {} unsigned PBOs {unsigned:?}; \
         {} signatures with no shipped key",
        keys.len(),
        unsigned.len(),
        unknown_key.len()
    );
    for failure in &failures {
        eprintln!("FAIL {failure}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
    assert!(verified >= 500, "expected the full install");
}
