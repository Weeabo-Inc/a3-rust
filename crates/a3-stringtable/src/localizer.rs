//! The global key -> text table for one language, and the `localize` lookup.

use std::collections::HashMap;

use a3_vfs::{Vfs, VfsPath};

use crate::{ENGLISH, Stringtable};

/// Every key of many stringtables, resolved for one language.
///
/// Keys compare ignoring ASCII case. When two tables define the same key, the first one added
/// wins, as in the engine (which logs "Item %s listed twice").
#[derive(Debug, Clone)]
pub struct Localizer {
    language: String,
    texts: HashMap<String, String>,
}

impl Default for Localizer {
    /// A localizer for English.
    fn default() -> Self {
        Self::new(ENGLISH)
    }
}

/// What [`Localizer::load_vfs`] found.
#[derive(Debug, Default)]
pub struct LoadReport {
    /// Stringtable files read.
    pub tables: Vec<VfsPath>,
    /// Keys defined again by a later table (ignored), with the table that repeated them.
    pub duplicates: Vec<(VfsPath, String)>,
    /// Files that failed to read or parse.
    pub failed: Vec<(VfsPath, String)>,
    /// `stringtable.csv` files, which this crate does not read.
    pub skipped_csv: Vec<VfsPath>,
}

impl Localizer {
    /// An empty localizer for `language` (a stringtable element name such as `German`).
    pub fn new(language: &str) -> Self {
        Self {
            language: language.to_string(),
            texts: HashMap::new(),
        }
    }

    /// Loads every `stringtable.xml` and `stringtable.bin` in the VFS, in path order.
    pub fn load_vfs(vfs: &Vfs, language: &str) -> (Self, LoadReport) {
        let mut localizer = Self::new(language);
        let mut report = LoadReport::default();
        let mut paths = vfs.glob("**/stringtable.*");
        paths.sort();
        for path in paths {
            let extension = path.extension().unwrap_or_default().to_ascii_lowercase();
            if extension == "csv" {
                report.skipped_csv.push(path);
                continue;
            }
            if extension != "xml" && extension != "bin" {
                continue;
            }
            let table = vfs
                .open(path.as_str())
                .map_err(|e| e.to_string())
                .and_then(|data| Stringtable::read(&data).map_err(|e| e.to_string()));
            match table {
                Ok(table) => {
                    for key in localizer.add(&table) {
                        report.duplicates.push((path.clone(), key));
                    }
                    report.tables.push(path);
                }
                Err(e) => report.failed.push((path, e)),
            }
        }
        (localizer, report)
    }

    /// The language texts are resolved for.
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Adds every key of `table` not already present; returns the keys that were (as written in
    /// `table`).
    pub fn add(&mut self, table: &Stringtable) -> Vec<String> {
        let mut duplicates = Vec::new();
        for entry in &table.entries {
            let text = entry.resolve(&self.language).unwrap_or_default();
            match self.texts.entry(entry.key.to_ascii_lowercase()) {
                std::collections::hash_map::Entry::Occupied(_) => {
                    duplicates.push(entry.key.clone());
                }
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(text.to_string());
                }
            }
        }
        duplicates
    }

    /// The text of `key` (ignoring ASCII case), if any table defines it.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.texts
            .get(&key.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Whether any table defines `key`.
    pub fn contains(&self, key: &str) -> bool {
        self.texts.contains_key(&key.to_ascii_lowercase())
    }

    /// Number of distinct keys.
    pub fn len(&self) -> usize {
        self.texts.len()
    }

    /// Whether no keys are loaded.
    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }

    /// The `localize` script command: drops a leading `$` before `STR` (any case), looks the
    /// key up, and gives an empty string when it is missing.
    pub fn localize(&self, text: &str) -> &str {
        let key = match text.strip_prefix('$') {
            Some(rest) if starts_with_str(rest) => rest,
            _ => text,
        };
        if key.is_empty() {
            return "";
        }
        self.get(key).unwrap_or("")
    }

    /// Resolves a config string value: `$STR...` references are looked up (empty when missing),
    /// anything else is literal text.
    pub fn config_text<'a>(&'a self, text: &'a str) -> &'a str {
        match text.strip_prefix('$') {
            Some(key) if starts_with_str(key) => self.get(key).unwrap_or(""),
            _ => text,
        }
    }
}

fn starts_with_str(text: &str) -> bool {
    text.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("STR"))
}
