//! The stringtable-backed [`Localizer`]: many `stringtable.xml`/`stringtable.bin` files merged
//! for one language, attached to a [`GameData`](crate::GameData) at load time.
//!
//! `localize`, `isLocalized`, `$STR_` config text and the `language` command all read this.

use std::sync::Arc;

use a3_stringtable::Localizer as Tables;
use a3_vfs::Vfs;

use crate::Localizer;

/// One language's key → text table.
pub struct Stringtables {
    tables: Tables,
}

impl Localizer for Stringtables {
    fn localize(&self, key: &str) -> Option<String> {
        self.tables.get(key).map(str::to_owned)
    }

    fn language(&self) -> &str {
        self.tables.language()
    }
}

/// Loads every stringtable of `vfs` for `language`.
pub fn load(vfs: &Vfs, language: &str) -> (Arc<dyn Localizer>, a3_stringtable::LoadReport) {
    let (tables, report) = Tables::load_vfs(vfs, language);
    (Arc::new(Stringtables { tables }), report)
}

/// Loads the English stringtables of `vfs`.
pub fn load_english(vfs: &Vfs) -> (Arc<dyn Localizer>, a3_stringtable::LoadReport) {
    load(vfs, a3_stringtable::ENGLISH)
}
