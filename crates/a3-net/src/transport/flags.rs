//! The `flags` field of the datagram header (u16 at offset 2), from `docs/re/net-transport.md`.
//!
//! The names are ours: the retail binary strips its own, so only the bit meanings are evidence.

/// Guaranteed delivery: kept in the send window and resent until acknowledged.
pub const RELIABLE: u16 = 0x8000;
/// The second reliable lane ("urgent"), with its own ordering chain, queued ahead of normal traffic.
pub const URGENT: u16 = 0x4000;
/// The payload must be delivered after the reliable message whose serial is in `extra`.
pub const ORDERED: u16 = 0x2000;
/// Sent without a channel (connect reject); the receiver skips acknowledgement bookkeeping.
pub const NOCHANNEL: u16 = 0x1000;
/// A connection-less control message, routed to the peer's default handler instead of a channel.
pub const CONTROL: u16 = 0x0800;
/// `ack` names a datagram the sender received `extra` microseconds before sending this one.
pub const ACK_RTT: u16 = 0x0400;
/// Asks the peer to answer with an [`ACK_RTT`] datagram.
pub const RTT_REQ: u16 = 0x0200;
/// `extra` carries the sender's bandwidth estimate.
pub const BANDWIDTH: u16 = 0x0100;
/// A bandwidth probe; the receiver measures the inter-arrival time.
pub const BW_PROBE: u16 = 0x0080;
/// No user data; the payload may hold the extended ack block.
pub const ACK_ONLY: u16 = 0x0040;
/// Part of a split guaranteed message.
pub const FRAGMENT: u16 = 0x0020;
/// The final part of a split guaranteed message (set together with [`FRAGMENT`]).
pub const LAST_FRAGMENT: u16 = 0x0010;
/// Set on every handshake control message (`0x801`, `0x8001`, `0x1001` in the sequence diagram).
pub const CONTROL_MESSAGE: u16 = 0x0001;

/// `flags & 0x2500 != 0`: `extra` is meaningful and the ack mask narrows to 32 bits.
pub const USES_EXTRA: u16 = ORDERED | ACK_RTT | BANDWIDTH;

/// The handshake's control flags: connection-less, and the "control message" bit.
pub const FLAGS_CONTROL: u16 = CONTROL | CONTROL_MESSAGE;
/// The handshake's accepted reply flags: reliable, on the new channel.
pub const FLAGS_ACCEPTED: u16 = RELIABLE | CONTROL_MESSAGE;
/// The handshake's rejected reply flags: no channel exists to be reliable on.
pub const FLAGS_REJECTED: u16 = NOCHANNEL | CONTROL_MESSAGE;

/// Whether the header's `extra` field is in use (and the ack mask is therefore 32-bit).
pub fn uses_extra(flags: u16) -> bool {
    flags & USES_EXTRA != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_extra_matches_the_documented_mask() {
        assert!(!uses_extra(RELIABLE | CONTROL | CONTROL_MESSAGE));
        assert!(uses_extra(ORDERED));
        assert!(uses_extra(ACK_RTT));
        assert!(uses_extra(BANDWIDTH));
        assert!(!uses_extra(URGENT | ACK_ONLY | FRAGMENT));
    }

    #[test]
    fn the_handshake_flag_sets_are_the_documented_ones() {
        assert_eq!(FLAGS_CONTROL, 0x0801);
        assert_eq!(FLAGS_ACCEPTED, 0x8001);
        assert_eq!(FLAGS_REJECTED, 0x1001);
    }
}
