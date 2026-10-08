//! Bikeys, bisigns and PBO signature verification.
//!
//! A publisher's [`PublicKey`] (`.bikey`) verifies a [`Signature`] (`.bisign`) of one PBO. A
//! signature holds three RSA signatures over three SHA-1 hashes of the PBO ([`PboHashes`]):
//! the archive itself, its file names and prefix, and the content of its script-like files.
//! [`PrivateKey`] (`.biprivatekey`) signs.
//!
//! Keys are Microsoft CryptoAPI key blobs (little-endian numbers); signatures are PKCS#1 v1.5
//! with SHA-1. See `docs/re/signing.md`.

mod error;
mod hashes;
mod keys;
mod read;

pub use error::{Error, Result};
pub use hashes::{PboHashes, hashed_by_v2, hashed_by_v3};
pub use keys::{PrivateKey, PublicKey, Signature, SignatureVersion};

use a3_pbo::Pbo;

/// Which of a signature's three hashes did not match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The whole archive (its SHA-1 trailer).
    Archive,
    /// The archive hash with the file names and prefix.
    Names,
    /// The content of the hashed files with the file names and prefix.
    Content,
}

/// Checks `signature` of `pbo` against `key`.
///
/// Fails with [`Error::WrongKey`] when the signature was made with another key, and with
/// [`Error::Mismatch`] naming the first of the three hashes that does not match.
pub fn verify(key: &PublicKey, signature: &Signature, pbo: &Pbo) -> Result<()> {
    if signature.key.modulus != key.modulus || signature.key.exponent != key.exponent {
        return Err(Error::WrongKey {
            expected: key.authority.clone(),
            found: signature.key.authority.clone(),
        });
    }
    let hashes = PboHashes::of(pbo, signature.version)?;
    let parts = [
        (Part::Archive, hashes.archive),
        (Part::Names, hashes.names),
        (Part::Content, hashes.content),
    ];
    for ((part, hash), sig) in parts.into_iter().zip(&signature.signatures) {
        if key.recover_digest(sig) != Some(hash) {
            return Err(Error::Mismatch(part));
        }
    }
    Ok(())
}
