/// Errors from reading a stringtable.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The XML is not well-formed.
    #[error("stringtable XML: {0}")]
    Xml(String),

    /// The file is not valid UTF-8.
    #[error("stringtable is not UTF-8: {0}")]
    Encoding(String),

    /// A binarized stringtable ends before a field it declares.
    #[error("binarized stringtable is truncated at byte {offset}")]
    Truncated {
        /// Byte offset at which more data was expected.
        offset: usize,
    },

    /// A binarized stringtable with inconsistent counts or offsets.
    #[error("malformed binarized stringtable: {0}")]
    Malformed(String),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
