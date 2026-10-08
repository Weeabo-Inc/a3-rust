//! Errors of the WRP reader and writer.

/// Errors from reading or writing a WRP file. Offsets are byte offsets into the file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file does not start with `OPRW`.
    #[error("not a binarized WRP file (signature {0:?})")]
    BadSignature([u8; 4]),

    /// The format version is outside the supported range.
    #[error("unsupported WRP version {0}")]
    UnsupportedVersion(u32),

    /// The data ended in the middle of a field.
    #[error("unexpected end of data at offset {offset} reading {what}")]
    Truncated {
        /// Where the field starts.
        offset: usize,
        /// The field.
        what: &'static str,
    },

    /// A field holds an impossible value.
    #[error("invalid {what} at offset {offset}: {detail}")]
    Invalid {
        /// Where the field starts.
        offset: usize,
        /// The field.
        what: &'static str,
        /// What is wrong.
        detail: String,
    },

    /// A map object has a type number the engine does not know.
    #[error("unknown map object type {kind} at offset {offset}")]
    UnknownMapType {
        /// The type number.
        kind: u32,
        /// Where the record starts.
        offset: usize,
    },

    /// A compressed array failed to decompress.
    #[error("cannot decompress {what} at offset {offset}: {detail}")]
    Decompress {
        /// Where the compressed data starts.
        offset: usize,
        /// The array.
        what: &'static str,
        /// The codec's error.
        detail: String,
    },

    /// The terrain cannot be serialised.
    #[error("cannot write terrain: {0}")]
    Write(String),
}

impl Error {
    /// The same error with offsets moved by `base` (for errors from a sub-block reader).
    pub(crate) fn shifted(self, base: usize) -> Self {
        match self {
            Error::Truncated { offset, what } => Error::Truncated {
                offset: offset + base,
                what,
            },
            Error::Invalid {
                offset,
                what,
                detail,
            } => Error::Invalid {
                offset: offset + base,
                what,
                detail,
            },
            Error::UnknownMapType { kind, offset } => Error::UnknownMapType {
                kind,
                offset: offset + base,
            },
            Error::Decompress {
                offset,
                what,
                detail,
            } => Error::Decompress {
                offset: offset + base,
                what,
                detail,
            },
            other => other,
        }
    }
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
