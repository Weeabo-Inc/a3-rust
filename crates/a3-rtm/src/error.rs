/// Errors from reading an RTM file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file does not start with `BMTR`, `RTM_0101` or `RTM_MDAT`.
    #[error("not an RTM file (signature {0:02x?})")]
    UnknownSignature(Vec<u8>),

    /// A binarized RTM of a version this crate does not read.
    #[error("unsupported BMTR version {0}")]
    UnsupportedVersion(u32),

    /// The file ends before a field it declares.
    #[error("RTM is truncated at byte {offset}")]
    Truncated {
        /// Byte offset at which more data was expected.
        offset: usize,
    },

    /// The file parses but holds inconsistent content.
    #[error("malformed RTM: {0}")]
    Malformed(String),

    /// A compressed array could not be decompressed.
    #[error("compressed array at byte {offset}: {reason}")]
    Decompress {
        /// Byte offset of the compressed data.
        offset: usize,
        /// What went wrong.
        reason: String,
    },
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
