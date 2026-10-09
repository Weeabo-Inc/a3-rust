//! Errors of the transport, the handshake and the A2S answers.

use crate::transport::{MAX_DATAGRAM, MAX_PAYLOAD};

/// Everything that can go wrong on the wire.
///
/// A datagram that fails a check is normally *dropped*, not fatal: the receive path of
/// `docs/re/net-transport.md` reads the size, de-obfuscates, then verifies the CRC, and a datagram
/// failing any of those never reaches the channel. `NetError` is how those drops are reported.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NetError {
    // ---- transport ------------------------------------------------------
    #[error("datagram of {len} bytes is outside the accepted {min}..={max} range")]
    DatagramLength {
        len: usize,
        min: usize,
        max: usize,
    },
    #[error("datagram header declares {declared} bytes but {actual} arrived")]
    DatagramSize { declared: u16, actual: usize },
    #[error("datagram crc: header {declared:#010x}, computed {computed:#010x}")]
    DatagramCrc { declared: u32, computed: u32 },
    #[error("payload of {len} bytes does not fit one datagram (max {max})")]
    PayloadTooLarge { len: usize, max: usize },
    #[error("serial {serial} refused: outside the receiver window or already seen")]
    SerialRefused { serial: u32 },

    // ---- handshake ------------------------------------------------------
    #[error("control message of {len} bytes, expected {expected}")]
    ControlLength { len: usize, expected: usize },
    #[error("control message id {0:#010x} is unknown")]
    UnknownControl(u32),
    #[error("connect message id {id:#010x} does not fit its {len}-byte payload")]
    ConnectId { id: u32, len: usize },
    #[error("connect net magic {found:#010x} is not the server's {expected:#010x}")]
    Magic { found: u32, expected: u32 },
    #[error("steam blob does not start with its marker byte")]
    SteamBlob,

    // ---- A2S ------------------------------------------------------------
    #[error("a2s request of {len} bytes is too short")]
    A2sShort { len: usize },
    #[error("a2s request header {header:x?} kind {kind:#04x} is not a known query")]
    A2sUnknownQuery { header: [u8; 4], kind: u8 },
    #[error("a2s reply: {field} is {len} bytes, longer than the {max} bytes the format allows")]
    A2sFieldTooLong {
        field: &'static str,
        len: usize,
        max: usize,
    },
    #[error("a2s rules: serialised block of {len} bytes exceeds the engine's {max}-byte limit")]
    RulesBlockTooLong { len: usize, max: usize },
    #[error("a2s rules: {chunks} chunks of {chunk}-byte escaped data exceed the 64 the client reads")]
    RulesChunks { chunks: usize, chunk: usize },

    // ---- io -------------------------------------------------------------
    #[error("socket: {0}")]
    Io(#[from] std::io::Error),
}

impl NetError {
    /// The size-check error for a datagram length, so the bounds live in one place.
    pub(crate) fn datagram_length(len: usize) -> Self {
        Self::DatagramLength {
            len,
            min: crate::transport::HEADER_SIZE,
            max: MAX_DATAGRAM,
        }
    }

    /// The error for a payload that does not fit a datagram.
    pub(crate) fn payload_too_large(len: usize) -> Self {
        Self::PayloadTooLarge {
            len,
            max: MAX_PAYLOAD,
        }
    }
}
