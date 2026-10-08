/// Errors from reading, writing or decoding textures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The data ends before a field it declares.
    #[error("unexpected end of data at byte {offset} while reading {what}")]
    Truncated {
        /// Byte offset at which more data was expected.
        offset: usize,
        /// The field being read.
        what: &'static str,
    },

    /// The data parses but describes impossible content.
    #[error("malformed {format}: {reason}")]
    Malformed {
        /// `"PAA"`, `"texHeaders.bin"`, ...
        format: &'static str,
        /// What is wrong.
        reason: String,
    },

    /// The first two bytes are not a known pixel format tag.
    #[error("unknown PAA type tag {0:#06x}")]
    UnknownFormat(u16),

    /// A compressed mipmap could not be decompressed.
    #[error("cannot decompress mipmap {index} ({width}x{height}): {source}")]
    Decompress {
        /// Index of the mipmap in the chain (0 = largest).
        index: usize,
        /// Mipmap width.
        width: u16,
        /// Mipmap height.
        height: u16,
        /// The codec error.
        source: a3_compress::Error,
    },

    /// The operation does not support this pixel format.
    #[error("unsupported pixel format for this operation: {0:?}")]
    UnsupportedFormat(crate::PixelFormat),

    /// A procedural texture string could not be parsed or generated.
    #[error("procedural texture {text:?}: {reason}")]
    Procedural {
        /// The procedural texture string.
        text: String,
        /// What is wrong.
        reason: String,
    },
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn malformed<T>(format: &'static str, reason: impl Into<String>) -> Result<T> {
    Err(Error::Malformed {
        format,
        reason: reason.into(),
    })
}
