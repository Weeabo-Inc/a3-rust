//! The three SHA-1 hashes a PBO signature covers.

use a3_pbo::Pbo;
use sha1::{Digest, Sha1};

use crate::{Result, SignatureVersion};

/// Extensions a version 2 signature leaves out of the content hash.
const V2_EXCLUDED: [&str; 13] = [
    "paa", "jpg", "p3d", "tga", "rvmat", "lip", "ogg", "wss", "png", "rtm", "pac", "fxy", "wrp",
];
/// Extensions a version 3 signature puts into the content hash (all others are left out).
const V3_INCLUDED: [&str; 11] = [
    "sqf", "inc", "bikb", "ext", "fsm", "sqm", "hpp", "cfg", "sqs", "h", "sqfc",
];

/// Whether a version 2 signature hashes the content of a file named `name`.
pub fn hashed_by_v2(name: &str) -> bool {
    !V2_EXCLUDED.contains(&extension(name).as_str())
}

/// Whether a version 3 signature hashes the content of a file named `name`.
pub fn hashed_by_v3(name: &str) -> bool {
    V3_INCLUDED.contains(&extension(name).as_str())
}

fn extension(name: &str) -> String {
    let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
    file.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

/// The hashes a signature signs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PboHashes {
    /// The archive's SHA-1 trailer (computed when the archive has none).
    pub archive: [u8; 20],
    /// SHA-1 of `archive`, the name hash and the prefix.
    pub names: [u8; 20],
    /// SHA-1 of the content hash, the name hash and the prefix.
    pub content: [u8; 20],
}

impl PboHashes {
    /// Computes the hashes of `pbo` for a signature version.
    ///
    /// - name hash: SHA-1 of the lower-case names of every entry with data, in header order;
    /// - content hash: SHA-1 of the stored data of those entries the version hashes, or of the
    ///   text `nothing` (v2) / `gnihton` (v3) when there are none;
    /// - prefix: the `prefix` property with a trailing backslash, or empty.
    pub fn of(pbo: &Pbo, version: SignatureVersion) -> Result<Self> {
        let archive = pbo.stored_hash().unwrap_or_else(|| pbo.compute_hash());
        let mut names = Sha1::new();
        let mut content = Sha1::new();
        let mut hashed_any = false;
        for entry in pbo.entries().iter().filter(|e| e.data_size() > 0) {
            names.update(entry.name().to_ascii_lowercase().as_bytes());
            let hashed = match version {
                SignatureVersion::V2 => hashed_by_v2(entry.name()),
                SignatureVersion::V3 => hashed_by_v3(entry.name()),
            };
            if hashed {
                content.update(pbo.raw(entry));
                hashed_any = true;
            }
        }
        if !hashed_any {
            content.update(match version {
                SignatureVersion::V2 => b"nothing".as_slice(),
                SignatureVersion::V3 => b"gnihton".as_slice(),
            });
        }
        let mut prefix = pbo
            .properties()
            .get("prefix")
            .unwrap_or_default()
            .to_string();
        if !prefix.is_empty() && !prefix.ends_with('\\') {
            prefix.push('\\');
        }
        let names: [u8; 20] = names.finalize().into();
        let content: [u8; 20] = content.finalize().into();
        let with_names_and_prefix = |first: &[u8]| -> [u8; 20] {
            let mut h = Sha1::new();
            h.update(first);
            h.update(names);
            h.update(prefix.as_bytes());
            h.finalize().into()
        };
        Ok(Self {
            archive,
            names: with_names_and_prefix(&archive),
            content: with_names_and_prefix(&content),
        })
    }
}
