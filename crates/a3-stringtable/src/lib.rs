//! Stringtables and the engine's localization lookup.
//!
//! A [`Stringtable`] maps keys (conventionally `STR_...`) to text in each language. It comes from
//! a `stringtable.xml` (keys in any `Project`/`Package`/`Container` nesting, one element per
//! language) or a binarized `stringtable.bin` (`BLMX`). [`Entry::resolve`] picks a key's text for
//! a language the way the engine does; a [`Localizer`] merges many tables for one language and
//! answers `localize` and `$STR_` config lookups.
//!
//! See `docs/re/stringtable.md` for the engine behaviour this follows.

mod bin;
mod error;
mod localizer;
mod xml;

pub use error::{Error, Result};
pub use localizer::{LoadReport, Localizer};

/// The language element holding the source text, preferred after the selected language.
pub const ORIGINAL: &str = "Original";
/// The fallback language after [`ORIGINAL`].
pub const ENGLISH: &str = "English";

/// One key and its text in each language, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The key as written (compare ignoring ASCII case).
    pub key: String,
    /// `(language, text)` pairs in file order.
    pub translations: Vec<(String, String)>,
}

impl Entry {
    /// The text in exactly `language` (ignoring ASCII case), if present.
    pub fn get(&self, language: &str) -> Option<&str> {
        self.translations
            .iter()
            .find(|(l, _)| l.eq_ignore_ascii_case(language))
            .map(|(_, text)| text.as_str())
    }

    /// The text the engine shows for `language`: that language, else `Original`, else
    /// `English`, else the first translation. `None` only for a key with no translations.
    pub fn resolve(&self, language: &str) -> Option<&str> {
        self.get(language)
            .or_else(|| self.get(ORIGINAL))
            .or_else(|| self.get(ENGLISH))
            .or_else(|| self.translations.first().map(|(_, text)| text.as_str()))
    }
}

/// The keys of one stringtable file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stringtable {
    /// Keys in file order.
    pub entries: Vec<Entry>,
}

impl Stringtable {
    /// Reads a stringtable file: binarized when it starts with `BLMX`, else XML (UTF-8, with or
    /// without a byte order mark).
    pub fn read(data: &[u8]) -> Result<Self> {
        if data.starts_with(bin::SIGNATURE) {
            return bin::read(data);
        }
        let data = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(data);
        let text = std::str::from_utf8(data).map_err(|e| Error::Encoding(e.to_string()))?;
        Self::from_xml(text)
    }

    /// Parses `stringtable.xml` text.
    pub fn from_xml(text: &str) -> Result<Self> {
        xml::parse(text)
    }

    /// Encodes the table as a binarized `stringtable.bin`. Every language that any key uses
    /// becomes a column; a key without that language gets its [`Entry::resolve`] text.
    pub fn to_bin(&self) -> Vec<u8> {
        bin::write(self)
    }

    /// Every language used by any key, in order of first appearance.
    pub fn languages(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for (language, _) in self.entries.iter().flat_map(|e| &e.translations) {
            if !out.iter().any(|l| l.eq_ignore_ascii_case(language)) {
                out.push(language);
            }
        }
        out
    }
}
