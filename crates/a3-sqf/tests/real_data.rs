//! Preprocesses, parses and compiles every `.sqf` file of a real game
//! install, as `preprocessFileLineNumbers` + `compile` would. Skipped when
//! `A3_ROOT` is unset. Run with `--nocapture` for the report;
//! `SQF_REAL_DATA_VERBOSE=1` lists every failing file.

use std::collections::BTreeMap;
use std::path::Path;

use a3_preproc::{IncludeError, IncludeResolver, Preprocessor, ResolvedInclude, join_virtual_path};
use a3_sqf::{CommandTable, NullHost, Registry, SourceFile, compile_source};
use a3_vfs::{Vfs, optional_mod_dirs};

/// Script text: UTF-16 LE when it starts with that byte order mark,
/// otherwise UTF-8 (a BOM is dropped, invalid bytes replaced).
fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// Resolves `#include` against the mounted game.
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
            source: decode(&bytes),
        })
    }

    fn exists(&self, current_file: &str, include: &str) -> bool {
        self.0.exists(&join_virtual_path(current_file, include))
    }
}

#[test]
fn compiles_game_sqf_files() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let root = Path::new(&root);
    let vfs = Vfs::new();
    vfs.mount_game(root, &optional_mod_dirs(root));
    let table: CommandTable = Registry::<NullHost>::with_core().table().clone();
    let resolver = VfsResolver(&vfs);
    let verbose = std::env::var_os("SQF_REAL_DATA_VERBOSE").is_some();

    let files = vfs.glob("**/*.sqf");
    let (mut ok, mut preprocess_failed) = (0usize, 0usize);
    let mut errors: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for path in &files {
        let Ok(bytes) = vfs.open(path.as_str()) else {
            continue;
        };
        let vpath = format!("\\{}", path.as_str());
        let mut pp = Preprocessor::new(&resolver);
        let text = match pp.preprocess_str(&vpath, &decode(&bytes)) {
            Ok(out) => out.with_line_directives(),
            Err(e) => {
                preprocess_failed += 1;
                if verbose {
                    eprintln!("PREPROCESS {vpath}: {e}");
                }
                continue;
            }
        };
        let src = SourceFile::new(vpath.as_str(), text);
        match compile_source(&src, &table) {
            Ok(_) => ok += 1,
            Err(e) => {
                let loc = src.locate(e.span.start);
                let snippet: String = src.text()[e.span.start as usize..]
                    .chars()
                    .take(40)
                    .collect();
                let entry = errors
                    .entry(e.message.clone())
                    .or_insert_with(|| (0, format!("{}:{} <{snippet}>", loc.file, loc.line)));
                entry.0 += 1;
                if verbose {
                    eprintln!("FAIL {}:{} {} <{snippet}>", loc.file, loc.line, e.message);
                }
            }
        }
    }
    let total = files.len();
    eprintln!(
        "compiled {ok}/{total} .sqf files ({:.2}%); preprocessor errors: {preprocess_failed}",
        ok as f64 * 100.0 / total.max(1) as f64
    );
    for (msg, (n, example)) in &errors {
        eprintln!("{n:6}  {msg}  e.g. {example}");
    }
    assert!(
        !files.is_empty(),
        "no .sqf files found under {}",
        root.display()
    );
    // The rest use commands removed from the game, macros defined only by
    // an including file, or are not valid SQF in the original either.
    assert!(
        ok as f64 >= total as f64 * 0.98,
        "only {ok}/{total} files compiled"
    );
}
