//! Preprocesses every text script/config source shipped in the game's PBOs.
//!
//! Needs `A3_ROOT`; skipped otherwise. Run with `--nocapture` to see the report.

#[path = "support/pbo.rs"]
mod pbo;

use std::collections::BTreeMap;

use a3_preproc::{
    ErrorKind, IncludeError, IncludeResolver, Options, Preprocessor, ResolvedInclude,
    join_virtual_path,
};
use pbo::Vfs;

struct VfsResolver<'a>(&'a Vfs);

impl IncludeResolver for VfsResolver<'_> {
    fn resolve(&self, current_file: &str, include: &str) -> Result<ResolvedInclude, IncludeError> {
        let path = join_virtual_path(current_file, include);
        let entry = self
            .0
            .files
            .get(&path.to_ascii_lowercase())
            .ok_or_else(|| IncludeError::NotFound(path.clone()))?;
        let bytes = self.0.read(entry).ok_or_else(|| IncludeError::Io {
            path: path.clone(),
            message: "compressed or unreadable PBO entry".to_owned(),
        })?;
        Ok(ResolvedInclude {
            path,
            source: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }

    fn exists(&self, current_file: &str, include: &str) -> bool {
        self.0
            .files
            .contains_key(&join_virtual_path(current_file, include).to_ascii_lowercase())
    }
}

fn category(kind: &ErrorKind) -> String {
    match kind {
        ErrorKind::Include { source, .. } => match source {
            IncludeError::NotFound(_) => "include: not found".to_owned(),
            _ => "include: unreadable".to_owned(),
        },
        ErrorKind::UnknownDirective(d) => format!("unknown directive #{d}"),
        ErrorKind::MalformedDirective { directive, .. } => format!("malformed #{directive}"),
        ErrorKind::Evaluation { macro_name, .. } => format!("{macro_name} evaluation"),
        other => {
            let debug = format!("{other:?}");
            debug
                .split(['(', ' ', '{'])
                .next()
                .unwrap_or_default()
                .to_owned()
        }
    }
}

#[derive(Default)]
struct Stats {
    ok: usize,
    warnings: usize,
    skipped_binary: usize,
    skipped_compressed: usize,
    failures: BTreeMap<String, Vec<String>>,
}

impl Stats {
    fn failed(&self) -> usize {
        self.failures.values().map(Vec::len).sum()
    }
}

#[test]
fn preprocess_all_shipped_sources() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let vfs = Vfs::mount(std::path::Path::new(&root));
    let resolver = VfsResolver(&vfs);
    eprintln!(
        "mounted {} PBOs ({} unreadable), {} files",
        vfs.pbos.len(),
        vfs.broken.len(),
        vfs.files.len()
    );

    let mut paths: Vec<&String> = vfs.files.keys().collect();
    paths.sort();
    let mut by_ext: BTreeMap<&str, Stats> = BTreeMap::new();
    for path in paths {
        let ext = path.rsplit_once('.').map_or("", |(_, e)| e);
        let config = match ext {
            "sqf" => false,
            "hpp" | "inc" | "h" | "ext" | "cpp" | "sqm" => true,
            _ => continue,
        };
        let stats = by_ext.entry(ext).or_default();
        let entry = &vfs.files[path];
        let Some(bytes) = vfs.read(entry) else {
            stats.skipped_compressed += 1;
            continue;
        };
        if bytes.starts_with(b"\0raP") {
            stats.skipped_binary += 1;
            continue;
        }
        let source = String::from_utf8_lossy(&bytes);
        let options = Options {
            config_macros: config,
            ..Options::default()
        };
        let mut pp = Preprocessor::new(&resolver).with_options(options);
        match pp.preprocess_str(path, &source) {
            Ok(out) => {
                stats.ok += 1;
                stats.warnings += out.warnings.len();
            }
            Err(e) => stats
                .failures
                .entry(category(&e.kind))
                .or_default()
                .push(e.to_string()),
        }
    }

    let mut total_ok = 0;
    let mut total_failed = 0;
    for (ext, stats) in &by_ext {
        total_ok += stats.ok;
        total_failed += stats.failed();
        eprintln!(
            ".{ext}: {} ok ({} warnings), {} failed, {} rapified skipped, {} compressed skipped",
            stats.ok,
            stats.warnings,
            stats.failed(),
            stats.skipped_binary,
            stats.skipped_compressed
        );
        for (cat, examples) in &stats.failures {
            eprintln!("    {cat}: {}", examples.len());
            for example in examples.iter().take(3) {
                eprintln!("        {example}");
            }
        }
    }
    eprintln!("total: {total_ok} ok, {total_failed} failed");

    // Known, legitimate failure categories:
    // - includes of files that are not shipped (dead scripts in missions_f_oldman, e.g.
    //   fn_OM_createMediaObjects.sqf includes a missing OM_postersOffsets.hpp);
    // - `__EVAL`/`__EXEC` using SQF that `SimpleEvaluator` does not know (code blocks, arrays,
    //   `localize`, `uiNamespace`); these need the SQF VM.
    // Anything else is a preprocessor bug.
    let allowed = [
        "include: not found",
        "__EVAL evaluation",
        "__EXEC evaluation",
    ];
    for (ext, stats) in &by_ext {
        for (cat, examples) in &stats.failures {
            assert!(
                allowed.contains(&cat.as_str()),
                ".{ext}: unexpected failure category `{cat}`: {examples:#?}"
            );
        }
    }
    let sqf = &by_ext["sqf"];
    assert!(
        sqf.ok > 0 && sqf.failed() * 1000 < sqf.ok,
        "too many SQF failures"
    );
}
