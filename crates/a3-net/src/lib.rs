//! The Arma 3 network protocol: UDP transport, connect handshake and Steam A2S queries.
//!
//! Byte-compatible with the official 2.22.0.154103 client and dedicated server, written from the
//! reverse engineering in `docs/re/net-transport.md`, `docs/re/net-handshake.md` and
//! `docs/re/net-a2s.md` (ADR 0004). Every layout, constant and algorithm here is taken from those
//! documents, which also carry the confidence notes; where a document leaves a detail open, the
//! item that needs it says so and the gap is tracked in the repository issues.
//!
//! ```text
//! a2s        Steam query protocol on the query port: info, player, rules, challenge
//! server     session state: pending challenges, players, replies (no sockets, so it is testable)
//! handshake  HELLO / CHALLENGE / CONNECT / RESULT control payloads and the version check
//! channel    serials, acknowledgements and the reliable outbox of one connection
//! packet     the 24-byte datagram header: size, flags, crc, serial, ack, ack_mask, extra
//! keys       the fixed obfuscation keys and payload XOR table derived from the net magic
//! ```
//!
//! Not implemented in this crate yet (all documented in `docs/re/net-transport.md` and
//! `docs/re/net-messages.md`, all needing a longer session than the transport alone):
//! fragmentation (flags `0x0020`/`0x0010`), the ordered/urgent lanes' delivery queues, the
//! extended ack block, RTT probes, bandwidth control, the `DContext` message encryption and the
//! game message catalogue.

mod bytes;

pub mod channel;
pub mod crc32;
pub mod crypto;
pub mod error;
pub mod handshake;
pub mod random;
pub mod server;
pub mod transport;

pub use error::NetError;
pub use transport::keys::MAGIC;
