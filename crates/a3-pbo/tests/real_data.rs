//! Parses every PBO of a real game install. Skipped when `A3_ROOT` is unset.
//!
//! The SHA-1 trailer is verified for every PBO up to 64 MiB; set `A3_VERIFY_ALL=1` to hash
//! every archive (tens of gigabytes).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use a3_pbo::{Pbo, read_properties};

const SAMPLE_HASH_LIMIT: u64 = 64 << 20;

fn archives(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            archives(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pbo") || e.eq_ignore_ascii_case("ebo"))
        {
            out.push(path);
        }
    }
}

#[test]
fn every_pbo_in_the_install_parses() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let verify_all = std::env::var_os("A3_VERIFY_ALL").is_some();
    let mut paths = Vec::new();
    archives(Path::new(&root), &mut paths);

    let start = Instant::now();
    let (mut pbos, mut ebos, mut entries, mut hashed, mut hashed_bytes) = (0, 0, 0u64, 0, 0u64);
    let mut methods = BTreeMap::new();
    let mut prefixes = BTreeSet::new();
    let mut no_prefix = Vec::new();
    let mut failures = Vec::new();
    for path in &paths {
        let is_ebo = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ebo"));
        if is_ebo {
            ebos += 1;
            match read_properties(path) {
                Ok(props) => match props.prefix() {
                    Some(prefix) => {
                        prefixes.insert(prefix);
                    }
                    None => no_prefix.push(path.clone()),
                },
                Err(e) => failures.push(format!("{}: {e}", path.display())),
            }
            continue;
        }
        pbos += 1;
        let pbo = match Pbo::open(path) {
            Ok(pbo) => pbo,
            Err(e) => {
                failures.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        entries += pbo.entries().len() as u64;
        for entry in pbo.entries() {
            *methods.entry(entry.method().to_string()).or_insert(0u64) += 1;
        }
        match pbo.prefix() {
            Some(prefix) => {
                prefixes.insert(prefix);
            }
            None => no_prefix.push(path.clone()),
        }
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if verify_all || len <= SAMPLE_HASH_LIMIT {
            hashed += 1;
            hashed_bytes += len;
            if let Err(e) = pbo.verify() {
                failures.push(format!("{}: {e}", path.display()));
            }
        }
    }

    eprintln!("PBOs: {pbos}, EBOs (properties only): {ebos}, entries: {entries}");
    eprintln!(
        "distinct prefixes: {}, without prefix: {no_prefix:?}",
        prefixes.len()
    );
    eprintln!("packing methods: {methods:?}");
    eprintln!(
        "SHA-1 verified: {hashed} PBOs, {:.1} GB; elapsed {:.1?}",
        hashed_bytes as f64 / 1e9,
        start.elapsed()
    );
    for failure in &failures {
        eprintln!("FAIL {failure}");
    }
    assert!(
        pbos > 0,
        "no PBOs found under {}",
        Path::new(&root).display()
    );
    assert!(failures.is_empty(), "{} archives failed", failures.len());
}
