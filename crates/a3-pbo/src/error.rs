use crate::PackingMethod;

/// Errors from reading or writing a PBO.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The underlying file could not be opened, mapped or read.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The header ends before a field it declares (no terminating NUL, short entry record).
    #[error("PBO header is truncated at byte {offset}")]
    Truncated {
        /// Byte offset at which more data was expected.
        offset: u64,
    },

    /// The header parses but describes impossible content (data past end of file).
    #[error("malformed PBO: {0}")]
    Malformed(String),

    /// The archive is an encrypted EBO; only its properties can be read.
    #[error("PBO is encrypted (EBO); its entries cannot be read")]
    Encrypted,

    /// The entry is stored with a packing method this crate cannot decode.
    #[error("entry {name:?} uses unsupported packing method {method}")]
    Unsupported {
        /// Entry name as stored in the header.
        name: String,
        /// The packing method of the entry.
        method: PackingMethod,
    },

    /// No entry has the requested path.
    #[error("no entry {0:?} in PBO")]
    NotFound(String),

    /// The archive has no SHA-1 trailer to verify.
    #[error("PBO has no SHA-1 trailer")]
    MissingHash,

    /// The SHA-1 trailer does not match the archive content.
    #[error("PBO SHA-1 mismatch: stored {stored}, computed {computed}")]
    HashMismatch {
        /// Hex of the hash stored in the trailer.
        stored: String,
        /// Hex of the hash computed over the archive.
        computed: String,
    },
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
