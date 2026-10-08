//! Preprocesses every text script/config source shipped in the game's PBOs.
//!
//! Needs `A3_ROOT`; skipped otherwise. Run with `--nocapture` to see the report.

use std::collections::BTreeMap;
use std::path::PathBuf;

use a3_preproc::{
    ErrorKind, IncludeError, IncludeResolver, Options, Preprocessor, ResolvedInclude,
    join_virtual_path,
};
use a3_vfs::Vfs;

/// Resolves includes against the mounted game, as the engine does.
struct VfsResolver<'a>(&'a Vfs);

impl IncludeResolver for VfsResolver<'_> {
    fn resolve(&self, current_file: &str, include: &str) -> Result<ResolvedInclude, IncludeError> {
        let path = join_virtual_path(current_file, include);
        let bytes = self.0.open(&path).map_err(|e| match e {
            a3_vfs::Error::NotFound(_) => IncludeError::NotFound(path.clone()),
            other => IncludeError::Io {
                path: path.clone(),
                message: other.to_string(),
            },
        })?;
        Ok(ResolvedInclude {
            path,
            source: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }

    fn exists(&self, current_file: &str, include: &str) -> bool {
        self.0.exists(&join_virtual_path(current_file, include))
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
    unreadable: usize,
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
    let root = PathBuf::from(root);
    let vfs = Vfs::new();
    // Base game, default DLCs and the optional (creator) DLC folders.
    let report = vfs.mount_game(&root, &a3_vfs::optional_mod_dirs(&root));
    eprintln!(
        "mounted {} PBOs ({} failed, {} encrypted), {} files",
        report.pbos,
        report.failed.len(),
        report.encrypted.len(),
        vfs.len()
    );
    let resolver = VfsResolver(&vfs);

    let mut by_ext: BTreeMap<String, Stats> = BTreeMap::new();
    for path in vfs.walk("") {
        let path = format!("\\{}", path.as_str());
        let ext = path.rsplit_once('.').map_or("", |(_, e)| e).to_owned();
        let config = match ext.as_str() {
            "sqf" => false,
            "hpp" | "inc" | "h" | "ext" | "cpp" | "sqm" => true,
            _ => continue,
        };
        let stats = by_ext.entry(ext).or_default();
        let Ok(bytes) = vfs.open(&path) else {
            stats.unreadable += 1;
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
        match pp.preprocess_str(&path, &source) {
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
            ".{ext}: {} ok ({} warnings), {} failed, {} rapified skipped, {} unreadable",
            stats.ok,
            stats.warnings,
            stats.failed(),
            stats.skipped_binary,
            stats.unreadable
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
