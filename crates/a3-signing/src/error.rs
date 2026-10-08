use crate::Part;

/// Errors from reading keys and signatures or verifying a PBO.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file ends before a field it declares.
    #[error("key or signature file is truncated at byte {offset}")]
    Truncated {
        /// Byte offset at which more data was expected.
        offset: usize,
    },

    /// The file parses but holds values this crate does not accept.
    #[error("malformed key or signature: {0}")]
    Malformed(String),

    /// The PBO could not be read.
    #[error(transparent)]
    Pbo(#[from] a3_pbo::Error),

    /// The signature was made with a different key.
    #[error("signed with key {found:?}, not {expected:?}")]
    WrongKey {
        /// Authority of the key verified against.
        expected: String,
        /// Authority named in the signature.
        found: String,
    },

    /// One of the three signed hashes does not match the PBO.
    #[error("signature does not match the PBO ({0:?} hash)")]
    Mismatch(Part),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
