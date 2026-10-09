//! The datagram layer: the 24-byte header, its obfuscation and the payload.
//!
//! One UDP datagram is one `NetMessage` (`docs/re/net-transport.md`, "Layering"): a plaintext
//! 16-bit length, an obfuscated header, a CRC-32 over the plaintext, and the payload XORed with a
//! keystream selected by the plaintext serial.

pub mod flags;
pub mod keys;
pub mod packet;

pub use flags::{
    ACK_ONLY, ACK_RTT, BANDWIDTH, BW_PROBE, CONTROL, CONTROL_MESSAGE, FRAGMENT, LAST_FRAGMENT,
    NOCHANNEL, ORDERED, RELIABLE, RTT_REQ, URGENT, USES_EXTRA,
};
pub use keys::{Keys, MAGIC};
pub use packet::{HEADER_SIZE, Header, MAX_DATAGRAM, MAX_PAYLOAD, pack, unpack};

/// The offset of the payload inside a datagram; the header is fixed at 24 bytes.
pub const PAYLOAD_OFFSET: usize = HEADER_SIZE;
