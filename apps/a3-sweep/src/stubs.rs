//! The commands the implementation only *stubs*, read from `docs/fidelity/sqf-verified.tsv`.
//!
//! A stub implements a command's contract (arguments, return value, errors) while its side effect
//! is a stand-in until the subsystem behind it exists, so it never raises `Unimplemented
//! command`. A scenario whose scripts use one can look clean while resting on a stand-in, which
//! is why the sweep reports `pass` and `pass with no stubs` side by side.

use std::collections::BTreeSet;
use std::path::Path;

use a3_sqf::Form;
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use sha1::{Digest as _, Sha1};

/// What a run's `pass with no stubs` count is based on, so two runs can be compared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StubIndex {
    /// The `sqf-verified.tsv` the records were read from.
    pub source: String,
    /// Records whose status is `stub`.
    pub records: usize,
    /// sha1 of the file, first eight hex digits.
    pub sha1: String,
}

/// The `stub` records of a verification ledger, as `(lower-case command, form)`.
#[derive(Debug, Clone, Default)]
pub struct Stubs {
    keys: BTreeSet<(String, String)>,
    index: Option<StubIndex>,
}

impl Stubs {
    /// Reads the ledger at `path`. Its columns are tab-separated (name, form, handler, status,
    /// source, note); `#` lines are comments and a short line is skipped.
    pub fn load(path: &Path) -> anyhow::Result<Stubs> {
        let bytes =
            std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        let text = String::from_utf8_lossy(&bytes);
        let mut keys = BTreeSet::new();
        for line in text.lines() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let mut columns = line.split('\t');
            let (Some(name), Some(form), _, Some(status)) = (
                columns.next(),
                columns.next(),
                columns.next(),
                columns.next(),
            ) else {
                continue;
            };
            if status.trim().eq_ignore_ascii_case("stub") && !name.trim().is_empty() {
                keys.insert((
                    name.trim().to_ascii_lowercase(),
                    form.trim().to_ascii_lowercase(),
                ));
            }
        }
        let index = StubIndex {
            source: path.display().to_string(),
            records: keys.len(),
            sha1: sha1_short(&bytes),
        };
        Ok(Stubs {
            keys,
            index: Some(index),
        })
    }

    /// What a run that could not read the ledger knows: with no records, no scenario's pass can
    /// be told apart from a stubbed one, and the summary says so.
    pub fn unknown() -> Stubs {
        Stubs::default()
    }

    /// Whether the record for `name` in `form` says `stub`.
    pub fn contains(&self, name: &str, form: Form) -> bool {
        self.keys
            .contains(&(name.to_ascii_lowercase(), form.as_str().to_owned()))
    }

    /// What the count is based on, or `None` when no ledger was read.
    pub fn index(&self) -> Option<&StubIndex> {
        self.index.as_ref()
    }

    /// Records read.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// sha1 of `bytes`, first eight hex digits.
fn sha1_short(bytes: &[u8]) -> String {
    let digest = Sha1::digest(bytes);
    digest
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger(text: &str) -> Stubs {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sqf-verified.tsv");
        std::fs::write(&path, text).unwrap();
        Stubs::load(&path).expect("reads")
    }

    const LEDGER: &str = "# name\tform\thandler\tstatus\tsource\tnote\nsetPos\tbinary\t0x1\tverified\tdecompiled 0x1\t\nallowDamage\tunary\t0x2\tstub\tdecompiled 0x2\tside effect pending\nallowDamage\tbinary\t0x3\tstub\tdecompiled 0x3\tside effect pending\nbroken line\nsetDir\tunary\t0x4\tverified\toracle\t\n";

    #[test]
    fn stub_records_are_read_per_name_and_form() {
        let stubs = ledger(LEDGER);
        assert_eq!(stubs.len(), 2, "{stubs:?}");
        assert!(stubs.contains("allowDamage", Form::Unary));
        assert!(
            stubs.contains("allowdamage", Form::Binary),
            "case-insensitive"
        );
        assert!(
            !stubs.contains("allowDamage", Form::Nular),
            "only the recorded forms"
        );
        assert!(
            !stubs.contains("setDir", Form::Unary),
            "verified is not a stub"
        );
        assert!(!stubs.contains("setPos", Form::Binary));
        let index = stubs.index().expect("read");
        assert_eq!(index.records, 2);
        assert_eq!(index.sha1.len(), 8);
        assert!(index.source.ends_with("sqf-verified.tsv"));
    }

    #[test]
    fn the_same_name_in_two_forms_counts_once_per_form() {
        let stubs = ledger(
            "double\tunary\t0x1\tstub\tdecompiled 0x1\t\ndouble\tbinary\t0x2\tstub\tdecompiled 0x2\t\n",
        );
        assert_eq!(stubs.len(), 2);
        assert_eq!(stubs.index().unwrap().records, 2);
    }

    #[test]
    fn an_unknown_ledger_knows_nothing_and_says_no_index() {
        let stubs = Stubs::unknown();
        assert!(stubs.is_empty());
        assert!(stubs.index().is_none());
        assert!(!stubs.contains("allowDamage", Form::Unary));
    }

    #[test]
    fn a_missing_ledger_is_an_error_the_caller_reports() {
        let dir = tempfile::tempdir().unwrap();
        let error = Stubs::load(&dir.path().join("nope.tsv")).unwrap_err();
        assert!(error.to_string().contains("nope.tsv"), "{error}");
    }
}
