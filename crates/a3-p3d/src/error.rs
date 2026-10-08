/// Errors from reading a P3D file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file does not start with a known P3D signature (`MLOD`, `ODOL`).
    #[error("not a P3D model: signature {0:?}")]
    UnknownSignature([u8; 4]),

    /// The file uses a format version this crate cannot read.
    #[error("unsupported {format} version {version}")]
    UnsupportedVersion {
        /// `"ODOL"`, `"MLOD"` or a LOD tag such as `"P3DM"`.
        format: &'static str,
        /// The version number found in the file.
        version: u32,
    },

    /// The data ends before a field it declares.
    #[error("P3D data truncated at byte {offset:#x} (needed {needed} more bytes)")]
    Truncated {
        /// Byte offset at which the read started.
        offset: usize,
        /// Bytes that were missing.
        needed: usize,
    },

    /// A field holds a value that cannot be right (count larger than the file, bad index).
    #[error("malformed P3D at byte {offset:#x}: {message}")]
    Malformed {
        /// Byte offset of the offending field.
        offset: usize,
        /// What is wrong.
        message: String,
    },

    /// A compressed array could not be decompressed.
    #[error("cannot decompress array at byte {offset:#x}: {source}")]
    Decompress {
        /// Byte offset of the array data.
        offset: usize,
        /// The codec error.
        source: a3_compress::Error,
    },
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
