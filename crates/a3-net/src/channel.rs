//! Serials, acknowledgements and the reliable outbox of one connection (`NetChannelBasic`).
//!
//! From `docs/re/net-transport.md`, "Channel behaviour":
//!
//! * every outgoing datagram carries the next serial (resends get a fresh one) and the sender's
//!   acknowledgement state: `ack` is the highest serial received from the peer, and the mask names
//!   the 63 (or 31, when `extra` is in use) serials below it;
//! * the receiver keeps a 1024-entry ring and refuses a datagram whose serial is outside
//!   `[highest - 1023, highest + 0x100]` or already seen; while the channel has received nothing,
//!   the first serial in `2..1_000_000` is adopted as the start;
//! * reliable datagrams stay in the send window and are resent under a new serial until the peer's
//!   acknowledgement covers them.

use std::time::{Duration, Instant};

use crate::error::NetError;
use crate::transport::{Header, flags, packet};

/// Entries in the receiver's serial ring (`channel+0xa20`).
const RING: u32 = 1024;

/// How far ahead of the highest received serial a datagram may be before it is refused.
const AHEAD: u32 = 0x100;

/// The first serial a channel may adopt as its start, and the exclusive end of that range.
pub const START_SERIAL_RANGE: std::ops::Range<u32> = 2..1_000_000;

/// How long a reliable datagram waits for an acknowledgement before being resent.
///
/// Not from the documents: the engine's `ackTimeoutA`/`ackTimeoutB` come from `basic.cfg` class
/// `sockets` and their defaults sit in `.data`, so this value is ours. It only has to be well under
/// the client's 8-second handshake timeout.
pub const RESEND_AFTER: Duration = Duration::from_millis(250);

/// How many times one reliable datagram is retried before the channel gives up on it.
pub const MAX_RESENDS: u32 = 20;

/// One datagram the channel wants on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub header: Header,
    pub payload: Vec<u8>,
}

impl Outgoing {
    /// Serialise this datagram with the obfuscation of `keys`.
    pub fn pack(&self, keys: &crate::transport::Keys) -> Result<Vec<u8>, NetError> {
        packet::pack(&self.header, &self.payload, keys)
    }
}

/// One unacknowledged reliable datagram.
#[derive(Debug, Clone)]
struct Unacked {
    serial: u32,
    payload: Vec<u8>,
    flags: u16,
    extra: u32,
    sent_at: Instant,
    attempts: u32,
}

/// The send and receive state of one connection.
#[derive(Debug, Clone)]
pub struct Channel {
    next_serial: u32,
    highest: Option<u32>,
    lowest_kept: u32,
    seen: [u64; RING as usize / 64],
    outbox: Vec<Unacked>,
}

impl Channel {
    /// A channel whose first outgoing datagram carries `start_serial`.
    ///
    /// The peer adopts the first serial it sees in [`START_SERIAL_RANGE`] as the start of the
    /// channel, so a sender must choose its start inside that range.
    pub fn new(start_serial: u32) -> Self {
        Self {
            next_serial: start_serial,
            highest: None,
            lowest_kept: 0,
            seen: [0; RING as usize / 64],
            outbox: Vec::new(),
        }
    }

    /// The serial the next outgoing datagram will carry.
    pub fn peek_serial(&self) -> u32 {
        self.next_serial
    }

    /// The highest serial received from the peer, if anything has arrived.
    pub fn highest_received(&self) -> Option<u32> {
        self.highest
    }

    /// Number of reliable datagrams still waiting for an acknowledgement.
    pub fn pending(&self) -> usize {
        self.outbox.len()
    }

    /// Whether the reliable datagram with this serial is still unacknowledged.
    pub fn awaiting_ack(&self, serial: u32) -> bool {
        self.outbox.iter().any(|entry| entry.serial == serial)
    }

    /// Allocate the serial of the next outgoing datagram.
    pub fn allocate_serial(&mut self) -> u32 {
        let serial = self.next_serial;
        self.next_serial = self.next_serial.wrapping_add(1);
        serial
    }

    /// Fill a header's `ack`/`ack_mask` from what this side has received.
    pub fn fill_ack(&self, header: &mut Header) {
        let Some(highest) = self.highest else {
            header.ack = 0;
            header.ack_mask = 0;
            return;
        };
        header.ack = highest;
        let bits: i64 = if header.uses_extra() { 32 } else { 64 };
        let mut mask = 0u64;
        for i in 0..bits {
            let serial = i64::from(highest) - (bits - 1) + i;
            if serial >= 0 && self.is_seen(serial as u32) {
                mask |= 1 << i;
            }
        }
        header.ack_mask = mask;
    }

    /// Build the next outgoing datagram: allocate its serial, fill the acknowledgement fields and,
    /// when the flags ask for reliability, remember the payload for a resend.
    pub fn send(&mut self, flags: u16, payload: Vec<u8>, extra: u32, now: Instant) -> Outgoing {
        let mut header = Header::new(flags, self.allocate_serial());
        header.extra = extra;
        self.fill_ack(&mut header);
        if flags & flags::RELIABLE != 0 {
            self.outbox.push(Unacked {
                serial: header.serial,
                payload: payload.clone(),
                flags,
                extra,
                sent_at: now,
                attempts: 0,
            });
        }
        Outgoing { header, payload }
    }

    /// Accept a received datagram, updating the acknowledgement state.
    ///
    /// A refused datagram is dropped by the channel; the caller logs it.
    pub fn accept(&mut self, header: &Header) -> Result<(), NetError> {
        let serial = header.serial;
        match self.highest {
            None => {
                if !START_SERIAL_RANGE.contains(&serial) {
                    return Err(NetError::SerialRefused { serial });
                }
                self.highest = Some(serial);
                self.lowest_kept = serial;
            }
            Some(highest) => {
                // The window slides with the highest received serial, but never below the serial
                // the channel started on.
                self.lowest_kept = self.lowest_kept.max(highest.saturating_sub(RING - 1));
                if serial < self.lowest_kept {
                    return Err(NetError::SerialRefused { serial });
                }
                if serial > highest && serial - highest > AHEAD {
                    return Err(NetError::SerialRefused { serial });
                }
                if self.is_seen(serial) {
                    return Err(NetError::SerialRefused { serial });
                }
                if serial > highest {
                    self.highest = Some(serial);
                }
            }
        }
        self.set_seen(serial);
        Ok(())
    }

    /// Mark our reliable datagrams acknowledged by the peer's `ack`/`ack_mask`.
    pub fn acknowledge(&mut self, header: &Header) {
        let ack = header.ack;
        let bits: u64 = if header.uses_extra() { 32 } else { 64 };
        self.outbox.retain(|entry| {
            if entry.serial == ack {
                return false;
            }
            let offset = i64::from(entry.serial) - (i64::from(ack) - (bits as i64 - 1));
            let covered =
                offset >= 0 && offset < bits as i64 && (header.ack_mask >> offset) & 1 == 1;
            !covered
        });
    }

    /// Datagrams whose acknowledgement is overdue, each with a fresh serial.
    ///
    /// A reliable datagram that has been retried [`MAX_RESENDS`] times is dropped: the peer is
    /// gone or unreachable, and the session layer above decides what that means. The resend keeps
    /// the entry's `extra`, so a resent `ORDERED` datagram still names the serial it was ordered
    /// after; that is only correct for messages sent before it, which is all this layer sends.
    pub fn resend_due(&mut self, now: Instant) -> Vec<Outgoing> {
        let mut due = Vec::new();
        let mut give_up = Vec::new();
        for entry in &mut self.outbox {
            if entry.attempts >= MAX_RESENDS {
                give_up.push(entry.serial);
            } else if now.saturating_duration_since(entry.sent_at) >= RESEND_AFTER {
                entry.attempts += 1;
                entry.sent_at = now;
                due.push(entry.serial);
            }
        }
        if !give_up.is_empty() {
            log::warn!(
                "channel: dropping reliable datagrams {give_up:?} after {MAX_RESENDS} resends"
            );
            self.outbox.retain(|entry| !give_up.contains(&entry.serial));
        }
        let mut resends = Vec::with_capacity(due.len());
        for serial in due {
            let Some(index) = self.outbox.iter().position(|entry| entry.serial == serial) else {
                continue;
            };
            let new_serial = self.allocate_serial();
            self.outbox[index].serial = new_serial;
            let mut header = Header::new(self.outbox[index].flags, new_serial);
            header.extra = self.outbox[index].extra;
            self.fill_ack(&mut header);
            let payload = self.outbox[index].payload.clone();
            resends.push(Outgoing { header, payload });
        }
        resends
    }

    /// Whether this serial has already been received inside the ring window.
    fn is_seen(&self, serial: u32) -> bool {
        let index = (serial & (RING - 1)) as usize;
        self.seen[index / 64] & (1 << (index % 64)) != 0
    }

    /// Record a serial as received.
    fn set_seen(&mut self, serial: u32) {
        let index = (serial & (RING - 1)) as usize;
        self.seen[index / 64] |= 1 << (index % 64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(flags: u16, serial: u32) -> Header {
        Header::new(flags, serial)
    }

    #[test]
    fn adopts_the_first_serial_inside_the_documented_range() {
        let mut channel = Channel::new(10);
        // Outside 2..1000000: refused until a valid start arrives.
        assert!(channel.accept(&header(0, 1)).is_err());
        assert!(channel.accept(&header(0, 1_000_000)).is_err());
        assert!(channel.accept(&header(0, 2)).is_ok());
        assert_eq!(channel.highest_received(), Some(2));
        // Older than the start: refused. The same serial twice: refused.
        assert!(channel.accept(&header(0, 1)).is_err());
        assert!(channel.accept(&header(0, 2)).is_err());
        assert!(channel.accept(&header(0, 3)).is_ok());
    }

    #[test]
    fn refuses_serials_outside_the_window() {
        let mut channel = Channel::new(10);
        assert!(channel.accept(&header(0, 1000)).is_ok());
        // 0x100 ahead of the highest received is the last acceptable serial; one more is refused.
        assert!(channel.accept(&header(0, 1000 + AHEAD + 1)).is_err());
        assert!(channel.accept(&header(0, 1000 + AHEAD)).is_ok());
        // The window then slides with the new highest.
        assert!(channel.accept(&header(0, 1000 + 2 * AHEAD)).is_ok());

        // Below the 1024-entry ring: walk the highest far enough past the start that the ring,
        // not the start serial, is the floor.
        let mut old = Channel::new(10);
        assert!(old.accept(&header(0, 1000)).is_ok());
        let mut serial = 1000;
        while serial < 3000 {
            serial += 250;
            assert!(old.accept(&header(0, serial)).is_ok(), "advance to {serial}");
        }
        let floor = serial - (RING - 1);
        assert!(
            (1000..=serial).step_by(250).all(|s| s != floor),
            "the floor serial {floor} was never received"
        );
        assert!(old.accept(&header(0, floor)).is_ok());
        assert!(old.accept(&header(0, floor - 1)).is_err());
    }

    #[test]
    fn ack_fields_describe_what_arrived() {
        let mut channel = Channel::new(10);
        // Out-of-order arrival is normal: 98 starts the channel, 100 jumps ahead, 99 fills the gap.
        for serial in [98, 100, 99] {
            assert!(channel.accept(&header(0, serial)).is_ok(), "serial {serial}");
        }
        let mut out = header(flags::RELIABLE, 0);
        channel.fill_ack(&mut out);
        assert_eq!(out.ack, 100);
        // Bits 63, 62 and 61 name serials 100, 99 and 98.
        assert_eq!(out.ack_mask, 0b111 << 61);
    }

    #[test]
    fn a_serial_below_the_adopted_start_is_refused() {
        // The first serial seen fixes the start of the channel, so an earlier one has no window
        // left to fall into (`net-transport.md`, "Channel behaviour").
        let mut channel = Channel::new(10);
        assert!(channel.accept(&header(0, 100)).is_ok());
        assert!(channel.accept(&header(0, 99)).is_err());
    }

    #[test]
    fn no_received_serial_means_an_empty_acknowledgement() {
        let channel = Channel::new(10);
        let mut out = header(flags::RELIABLE, 0);
        channel.fill_ack(&mut out);
        assert_eq!(out.ack, 0);
        assert_eq!(out.ack_mask, 0);
    }

    #[test]
    fn the_ack_mask_narrows_to_32_bits_when_extra_is_in_use() {
        let mut channel = Channel::new(10);
        // 32 serials ending at 100, so the whole 32-bit mask is set.
        for serial in 69..=100 {
            assert!(channel.accept(&header(0, serial)).is_ok());
        }
        let mut out = header(flags::ORDERED, 0);
        channel.fill_ack(&mut out);
        assert_eq!(out.ack, 100);
        assert_eq!(out.ack_mask, 0xFFFF_FFFF);
    }

    #[test]
    fn a_reliable_datagram_is_resent_until_it_is_acknowledged() {
        let keys = crate::transport::Keys::default();
        let mut channel = Channel::new(50);
        let t0 = Instant::now();
        let outgoing = channel.send(flags::RELIABLE, b"RESULT".to_vec(), 0, t0);
        let serial = outgoing.header.serial;
        assert_eq!(serial, 50);
        assert_eq!(channel.pending(), 1);
        assert!(channel.awaiting_ack(serial));

        // Not yet due.
        assert!(channel.resend_due(t0 + RESEND_AFTER - Duration::from_millis(1)).is_empty());
        // Due: a fresh serial, the same payload.
        let resends = channel.resend_due(t0 + RESEND_AFTER);
        assert_eq!(resends.len(), 1);
        assert_eq!(resends[0].payload, b"RESULT");
        assert_ne!(resends[0].header.serial, serial);
        assert!(channel.awaiting_ack(resends[0].header.serial));
        assert_eq!(channel.pending(), 1, "the original is replaced by the resend");

        // The peer acknowledges the resend: nothing left to resend.
        let mut ack = header(flags::RELIABLE, 0);
        ack.ack = resends[0].header.serial;
        channel.acknowledge(&ack);
        assert_eq!(channel.pending(), 0);
        assert!(channel.resend_due(t0 + RESEND_AFTER * 4).is_empty());

        // The packed datagram of an outgoing is a valid transport datagram.
        let datagram = outgoing.pack(&keys).expect("pack");
        let (parsed, payload) = packet::unpack(&datagram, &keys).expect("unpack");
        assert_eq!(parsed.serial, serial);
        assert_eq!(payload, b"RESULT");
    }

    #[test]
    fn a_32_bit_mask_acknowledges_older_reliable_serials() {
        let mut channel = Channel::new(200);
        let t0 = Instant::now();
        let serials: Vec<u32> = (0..3)
            .map(|_| channel.send(flags::RELIABLE, b"x".to_vec(), 0, t0).header.serial)
            .collect();
        assert_eq!(serials, [200, 201, 202]);
        assert_eq!(channel.pending(), 3);

        let mut ack = header(flags::ORDERED, 0);
        ack.ack = 202;
        ack.ack_mask = 0b111 << 29; // serials 202, 201, 200
        channel.acknowledge(&ack);
        assert_eq!(channel.pending(), 0);
    }

    #[test]
    fn a_64_bit_mask_acknowledges_older_reliable_serials() {
        let mut channel = Channel::new(7);
        let t0 = Instant::now();
        let first = channel.send(flags::RELIABLE, b"a".to_vec(), 0, t0).header.serial;
        let second = channel.send(flags::RELIABLE, b"b".to_vec(), 0, t0).header.serial;
        let mut ack = header(flags::RELIABLE, 0);
        ack.ack = second;
        ack.ack_mask = 1 << 63 | 1 << 62; // `second` and `first`
        channel.acknowledge(&ack);
        assert_eq!(channel.pending(), 0, "both {first} and {second} are covered");
    }

    #[test]
    fn gives_up_on_a_reliable_datagram_after_the_retry_limit() {
        let mut channel = Channel::new(900_000);
        let t0 = Instant::now();
        channel.send(flags::RELIABLE, b"gone".to_vec(), 0, t0);
        for attempt in 1..=MAX_RESENDS {
            let now = t0 + RESEND_AFTER * attempt;
            assert_eq!(channel.resend_due(now).len(), 1, "attempt {attempt}");
        }
        assert_eq!(channel.resend_due(t0 + RESEND_AFTER * 100).len(), 0);
        assert_eq!(channel.pending(), 0);
    }
}
