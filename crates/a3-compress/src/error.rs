use std::io;

/// Errors from decoding a compressed block.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The compressed input ended before the block was complete.
    #[error(
        "compressed input ended after {consumed} bytes, with {produced} of {expected} output bytes decoded"
    )]
    UnexpectedEof {
        consumed: usize,
        produced: usize,
        expected: usize,
    },

    /// A back-reference points before the start of the output.
    #[error("back-reference at output offset {at} reaches {distance} bytes back, past the start")]
    InvalidDistance { at: usize, distance: usize },

    /// The block decodes to more bytes than the caller expected.
    #[error("block decodes to more than the expected {expected} bytes")]
    OutputOverrun { expected: usize },

    /// The block ended (end marker) before producing the expected number of bytes.
    #[error("block ended after {produced} of the expected {expected} bytes")]
    OutputUnderrun { produced: usize, expected: usize },

    /// The stored checksum does not match the decoded data.
    #[error("checksum mismatch: stored {stored:#010x}, computed {computed:#010x}")]
    ChecksumMismatch { stored: u32, computed: u32 },

    /// The LZ4 block is malformed.
    #[error("invalid LZ4 block: {0}")]
    Lz4(#[from] lz4_flex::block::DecompressError),

    /// Reading the compressed input from a stream failed.
    #[error(transparent)]
    Io(#[from] io::Error),
}
