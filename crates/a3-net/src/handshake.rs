//! The transport-level connect handshake (`docs/re/net-handshake.md`).
//!
//! ```text
//! client                                            server
//!   HELLO     0xBBBA1564   12 B  flags 0x0801  -->
//!   <-- CHALLENGE 0xA5A48965  8 B  flags 0x0801
//!   CONNECT   0xCCCA5E62  160 B  flags 0x0801  -->   checks, channel created
//!   <-- RESULT 0xAAA51A7E  12 B  flags 0x8001        accepted: reliable, on the new channel
//!   <-- RESULT 0xAAA51A7E  12 B  flags 0x1001        rejected: no channel
//! ```
//!
//! All control payloads are packed little-endian structs without padding, so each message here is
//! a fixed length except the session query. Decoding is separate from the server's checks: a
//! message that parses but fails a check (net magic, version, password) produces a RESULT with the
//! documented rejection code, not a parse error.

use crate::bytes::{Cursor, Writer};
use crate::crypto::{rc4_drop512, sha1};
use crate::error::NetError;

/// Client → server: "I am here, this is my version".
pub const CTRL_HELLO: u32 = 0xBBBA_1564;
/// Server → client: the challenge the client must echo in its CONNECT.
pub const CTRL_CHALLENGE: u32 = 0xA5A4_8965;
/// Client → server: CONNECT without a challenge (156 bytes).
pub const CTRL_CONNECT: u32 = 0xCCCA_1E12;
/// Client → server: CONNECT with the challenge appended (160 bytes).
pub const CTRL_CONNECT_CHALLENGE: u32 = 0xCCCA_5E62;
/// Server → client: accepted or rejected.
pub const CTRL_RESULT: u32 = 0xAAA5_1A7E;
/// Client → server: "describe your session" (17 bytes).
pub const CTRL_SESSION_QUERY: u32 = 0xEEE1_91AE;
/// Server → client: the session description block (variable length).
pub const CTRL_SESSION_INFO: u32 = 0xFFF1_E8AC;

/// Length of a HELLO payload.
pub const HELLO_LEN: usize = 12;
/// Length of a CHALLENGE payload.
pub const CHALLENGE_LEN: usize = 8;
/// Length of a CONNECT payload without a challenge.
pub const CONNECT_LEN: usize = 0x9C;
/// Length of a CONNECT payload with the challenge appended.
pub const CONNECT_CHALLENGE_LEN: usize = 0xA0;
/// Length of a RESULT payload.
pub const RESULT_LEN: usize = 12;
/// Length of the Steam ID blob inside a CONNECT.
pub const STEAM_BLOB_LEN: usize = 9;
/// Length of the session-query payload.
pub const SESSION_QUERY_LEN: usize = 17;

/// Width of the `char[40]` fields of a CONNECT (name, password, check string).
pub const CONNECT_TEXT_WIDTH: usize = 40;

/// The `actualVersion` this build reports (u16 zero-extended into the message).
pub const GAME_ACTUAL_VERSION: u16 = 222;
/// The build number this build reports.
pub const GAME_BUILD: u32 = 154103;
/// The version string the game passes to Steam and reports to A2S (`2.22.<build>`).
pub const GAME_VERSION_STRING: &str = "2.22.154103";
/// The lowest player ID a CONNECT may propose.
pub const MIN_PLAYER_ID: i32 = 0x14;

const STEAM_BLOB_MARKER: u8 = b'[';
const STEAM_BLOB_KEY_MATERIAL: &[u8] = b"8CFB1217-A5BC-465F-AB31-5DCB4AE7F58A";

/// HELLO: magic and the client's version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hello {
    /// The client's copy of the net magic; the server rejects a mismatch silently.
    pub magic: u32,
    /// The client's `actualVersion` (222 for this build).
    pub actual_version: u32,
}

impl Hello {
    /// Parse a 12-byte HELLO.
    pub fn decode(payload: &[u8]) -> Result<Self, NetError> {
        let len = payload.len();
        if len != HELLO_LEN {
            return Err(NetError::ControlLength {
                len,
                expected: HELLO_LEN,
            });
        }
        let mut cursor = Cursor::new(payload);
        let _id = cursor.u32();
        Ok(Self {
            magic: cursor.u32().expect("length checked"),
            actual_version: cursor.u32().expect("length checked"),
        })
    }

    /// Serialise this HELLO.
    pub fn encode(&self) -> Vec<u8> {
        Writer::new()
            .u32(CTRL_HELLO)
            .u32(self.magic)
            .u32(self.actual_version)
            .finish()
    }
}

/// CHALLENGE: the value the client must echo in its (challenged) CONNECT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Challenge {
    pub challenge: u32,
}

impl Challenge {
    /// Parse an 8-byte CHALLENGE.
    pub fn decode(payload: &[u8]) -> Result<Self, NetError> {
        let len = payload.len();
        if len != CHALLENGE_LEN {
            return Err(NetError::ControlLength {
                len,
                expected: CHALLENGE_LEN,
            });
        }
        let mut cursor = Cursor::new(payload);
        let _id = cursor.u32();
        Ok(Self {
            challenge: cursor.u32().expect("length checked"),
        })
    }

    /// Serialise this CHALLENGE.
    pub fn encode(&self) -> Vec<u8> {
        Writer::new()
            .u32(CTRL_CHALLENGE)
            .u32(self.challenge)
            .finish()
    }
}

/// CONNECT: everything the client tells the server about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connect {
    /// Whether the message carried the challenge (id `0xCCCA5E62` instead of `0xCCCA1E12`).
    pub with_challenge: bool,
    /// The client's net magic, which must equal the server's.
    pub magic: u32,
    /// Player (profile) name, `char[40]`.
    pub name: String,
    /// Server password, `char[40]`; empty when the server has none.
    pub password: String,
    /// The "check string" copied from the client's session descriptor. Its meaning is still
    /// unknown (`net-handshake.md` calls it medium confidence); the server compares it only when
    /// its own flag `+0x329 & 1` is set, which our server leaves clear.
    pub check_string: String,
    /// The raw 9-byte Steam blob, kept so a caller can decode it itself.
    pub steam_blob: [u8; STEAM_BLOB_LEN],
    /// Client's `actualVersion` (222).
    pub actual_version: u32,
    /// Client's `requiredVersion` (222).
    pub required_version: u32,
    /// Client's build (154103).
    pub build: u32,
    /// `flag A`: stored in the player record, input to the BattlEye/platform check.
    pub flag_a: u8,
    /// `flag B`: bit 0 marks the player as "local/host" in the original.
    pub flag_b: u8,
    /// The player ID the client proposes; the server takes the first free ID in `[id, id+12)`.
    pub player_id: i32,
    /// The client's anti-cheat (BattlEye) state. Our server accepts every value.
    pub be_state: u8,
    /// The echoed challenge, only present in the challenged form.
    pub challenge: Option<u32>,
}

impl Connect {
    /// The SteamID64 in the blob, when the blob decodes.
    pub fn steam_id(&self) -> Option<u64> {
        steam_blob_decode(&self.steam_blob)
    }

    /// Parse a 156- or 160-byte CONNECT.
    pub fn decode(payload: &[u8]) -> Result<Self, NetError> {
        let len = payload.len();
        let mut cursor = Cursor::new(payload);
        let id = cursor
            .u32()
            .ok_or(NetError::ControlLength { len, expected: 4 })?;
        let with_challenge = match id {
            CTRL_CONNECT if len == CONNECT_LEN => false,
            CTRL_CONNECT_CHALLENGE if len == CONNECT_CHALLENGE_LEN => true,
            _ => return Err(NetError::ConnectId { id, len }),
        };
        Ok(Self {
            with_challenge,
            magic: cursor.u32().expect("length checked"),
            name: cursor
                .fixed_string(CONNECT_TEXT_WIDTH)
                .expect("length checked"),
            password: cursor
                .fixed_string(CONNECT_TEXT_WIDTH)
                .expect("length checked"),
            check_string: cursor
                .fixed_string(CONNECT_TEXT_WIDTH)
                .expect("length checked"),
            steam_blob: cursor
                .bytes(STEAM_BLOB_LEN)
                .expect("length checked")
                .try_into()
                .expect("fixed width"),
            actual_version: cursor.u32().expect("length checked"),
            required_version: cursor.u32().expect("length checked"),
            build: cursor.u32().expect("length checked"),
            flag_a: cursor.u8().expect("length checked"),
            flag_b: cursor.u8().expect("length checked"),
            player_id: cursor.i32().expect("length checked"),
            be_state: cursor.u8().expect("length checked"),
            challenge: if with_challenge {
                Some(cursor.u32().expect("length checked"))
            } else {
                None
            },
        })
    }

    /// Serialise this CONNECT, which is what the test client sends.
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new()
            .u32(if self.with_challenge {
                CTRL_CONNECT_CHALLENGE
            } else {
                CTRL_CONNECT
            })
            .u32(self.magic)
            .fixed_string(&self.name, CONNECT_TEXT_WIDTH)
            .fixed_string(&self.password, CONNECT_TEXT_WIDTH)
            .fixed_string(&self.check_string, CONNECT_TEXT_WIDTH)
            .bytes(&self.steam_blob)
            .u32(self.actual_version)
            .u32(self.required_version)
            .u32(self.build)
            .u8(self.flag_a)
            .u8(self.flag_b)
            .i32(self.player_id)
            .u8(self.be_state);
        if let Some(challenge) = self.challenge {
            writer = writer.u32(challenge);
        }
        writer.finish()
    }
}

/// Why a CONNECT was rejected (`net-handshake.md`, "RESULT").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultCode {
    /// 0: the player is in, `player_id` is theirs.
    Accepted,
    /// 1: wrong server password.
    BadPassword,
    /// 2: version or build mismatch.
    VersionMismatch,
    /// 3: the server could not create the channel.
    ServerError,
    /// 4: server full, or no free player ID in the requested window.
    Full,
    /// 5: the anti-cheat/platform check rejected the client. Our server never sends this.
    AntiCheat,
    /// 6: the check string did not match.
    CheckString,
}

impl ResultCode {
    /// The wire value of this code.
    pub fn as_u32(self) -> u32 {
        match self {
            Self::Accepted => 0,
            Self::BadPassword => 1,
            Self::VersionMismatch => 2,
            Self::ServerError => 3,
            Self::Full => 4,
            Self::AntiCheat => 5,
            Self::CheckString => 6,
        }
    }

    /// The code a wire value names, if it is one we know.
    pub fn from_u32(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Accepted,
            1 => Self::BadPassword,
            2 => Self::VersionMismatch,
            3 => Self::ServerError,
            4 => Self::Full,
            5 => Self::AntiCheat,
            6 => Self::CheckString,
            _ => return None,
        })
    }
}

/// RESULT: the server's answer to a CONNECT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectResult {
    pub code: ResultCode,
    /// The assigned player ID (dpnid) when accepted, else 0.
    pub player_id: u32,
}

impl ConnectResult {
    /// Parse a 12-byte RESULT.
    pub fn decode(payload: &[u8]) -> Result<Self, NetError> {
        let len = payload.len();
        if len != RESULT_LEN {
            return Err(NetError::ControlLength {
                len,
                expected: RESULT_LEN,
            });
        }
        let mut cursor = Cursor::new(payload);
        let _id = cursor.u32();
        let raw = cursor.u32().expect("length checked");
        let code = ResultCode::from_u32(raw).ok_or(NetError::UnknownResultCode(raw))?;
        Ok(Self {
            code,
            player_id: cursor.u32().expect("length checked"),
        })
    }

    /// Serialise this RESULT.
    pub fn encode(&self) -> Vec<u8> {
        Writer::new()
            .u32(CTRL_RESULT)
            .u32(self.code.as_u32())
            .u32(self.player_id)
            .finish()
    }
}

/// The session-info query: magic plus the requester's Steam blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionQuery {
    pub magic: u32,
    pub steam_blob: [u8; STEAM_BLOB_LEN],
}

impl SessionQuery {
    /// Parse a 17-byte session query.
    pub fn decode(payload: &[u8]) -> Result<Self, NetError> {
        let len = payload.len();
        if len != SESSION_QUERY_LEN {
            return Err(NetError::ControlLength {
                len,
                expected: SESSION_QUERY_LEN,
            });
        }
        let mut cursor = Cursor::new(payload);
        let _id = cursor.u32();
        Ok(Self {
            magic: cursor.u32().expect("length checked"),
            steam_blob: cursor
                .bytes(STEAM_BLOB_LEN)
                .expect("length checked")
                .try_into()
                .expect("fixed width"),
        })
    }

    /// Serialise this query.
    pub fn encode(&self) -> Vec<u8> {
        Writer::new()
            .u32(CTRL_SESSION_QUERY)
            .u32(self.magic)
            .bytes(&self.steam_blob)
            .finish()
    }
    /// The SteamID64 in the blob, when it decodes.
    pub fn steam_id(&self) -> Option<u64> {
        steam_blob_decode(&self.steam_blob)
    }
}

/// A control payload this side knows how to parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    Hello(Hello),
    Connect(Connect),
    SessionQuery(SessionQuery),
}

impl Control {
    /// Parse a control payload by its leading message id.
    pub fn decode(payload: &[u8]) -> Result<Self, NetError> {
        let len = payload.len();
        let mut cursor = Cursor::new(payload);
        let id = cursor
            .u32()
            .ok_or(NetError::ControlLength { len, expected: 4 })?;
        match id {
            CTRL_HELLO => Hello::decode(payload).map(Self::Hello),
            CTRL_CONNECT | CTRL_CONNECT_CHALLENGE => Connect::decode(payload).map(Self::Connect),
            CTRL_SESSION_QUERY => SessionQuery::decode(payload).map(Self::SessionQuery),
            other => Err(NetError::UnknownControl(other)),
        }
    }
}

/// The obfuscation of the 9-byte Steam blob inside a CONNECT: `'['` + SteamID64 LE, XORed with
/// RC4-drop512 keyed by `SHA1("8CFB1217-A5BC-465F-AB31-5DCB4AE7F58A")`.
fn steam_blob_keystream() -> [u8; STEAM_BLOB_LEN] {
    let mut keystream = [0u8; STEAM_BLOB_LEN];
    rc4_drop512(&sha1(STEAM_BLOB_KEY_MATERIAL), &mut keystream);
    keystream
}

/// Build the Steam blob a client sends for `steam_id`.
pub fn steam_blob_encode(steam_id: u64) -> [u8; STEAM_BLOB_LEN] {
    let mut plain = [0u8; STEAM_BLOB_LEN];
    plain[0] = STEAM_BLOB_MARKER;
    plain[1..].copy_from_slice(&steam_id.to_le_bytes());
    let keystream = steam_blob_keystream();
    let mut out = [0u8; STEAM_BLOB_LEN];
    for (slot, (byte, key)) in out.iter_mut().zip(plain.iter().zip(keystream.iter())) {
        *slot = byte ^ key;
    }
    out
}

/// Decode a Steam blob back to its SteamID64, or `None` when the marker byte is wrong.
pub fn steam_blob_decode(blob: &[u8]) -> Option<u64> {
    let keystream = steam_blob_keystream();
    let mut plain = [0u8; STEAM_BLOB_LEN];
    for (slot, (byte, key)) in plain.iter_mut().zip(blob.iter().zip(keystream.iter())) {
        *slot = byte ^ key;
    }
    if plain[0] != STEAM_BLOB_MARKER {
        return None;
    }
    let mut id = [0u8; 8];
    id.copy_from_slice(&plain[1..]);
    Some(u64::from_le_bytes(id))
}

/// The version window a server accepts (`net-handshake.md`, "Server checks" step 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Versions {
    /// `server+0x29c`: the version this server runs.
    pub actual: u16,
    /// `server+0x29e`: the oldest client version accepted.
    pub required: u16,
    /// The oldest client build accepted (`server.cfg` `requiredBuild`).
    pub required_build: u32,
}

impl Versions {
    /// The versions of the game this crate targets.
    pub const fn target() -> Self {
        Self {
            actual: GAME_ACTUAL_VERSION,
            required: GAME_ACTUAL_VERSION,
            required_build: GAME_BUILD,
        }
    }

    /// `None` when the client fits, else the rejection code to send back.
    pub fn check(&self, connect: &Connect) -> Option<ResultCode> {
        if connect.build < self.required_build
            || connect.actual_version < u32::from(self.required)
            || connect.required_version > u32::from(self.actual)
        {
            Some(ResultCode::VersionMismatch)
        } else {
            None
        }
    }
}

impl Default for Versions {
    fn default() -> Self {
        Self::target()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The payload of the first datagram a real client sends, from `net-transport.md`'s
    /// "Validation" section: a HELLO with the net magic and `actualVersion` 222.
    const HELLO_FIXTURE: &str = "6415babb25252525de000000";

    fn decode_hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("test hex"))
            .collect()
    }

    /// A challenged CONNECT built by the reference codec:
    /// `a3net.connect_request("J. Doe", "hunter2", "check-string", 76561197960287930, 0x1234,
    /// challenge=0xDEADBEEF, flags95=1, flags96=2)`.
    const CONNECT_160: &str = "625ecacc252525254a2e20446f650000000000000000000000000000000000000000000000000000000000000000000068756e74657232000000000000000000000000000000000000000000000000000000000000000000636865636b2d737472696e67000000000000000000000000000000000000000000000000000000005da664525ef484aee2de000000de000000f759020001023412000000efbeadde";
    /// The same CONNECT without the challenge (156 bytes).
    const CONNECT_156: &str = "121ecacc252525254a2e20446f650000000000000000000000000000000000000000000000000000000000000000000068756e74657232000000000000000000000000000000000000000000000000000000000000000000636865636b2d737472696e67000000000000000000000000000000000000000000000000000000005da664525ef484aee2de000000de000000f759020001023412000000";

    const STEAM_ID: u64 = 76561197960287930;
    const STEAM_BLOB: &str = "5da664525ef484aee2";

    fn connect_160() -> Connect {
        Connect {
            with_challenge: true,
            magic: crate::transport::MAGIC,
            name: "J. Doe".into(),
            password: "hunter2".into(),
            check_string: "check-string".into(),
            steam_blob: steam_blob_encode(STEAM_ID),
            actual_version: 222,
            required_version: 222,
            build: 154103,
            flag_a: 1,
            flag_b: 2,
            player_id: 0x1234,
            be_state: 0,
            challenge: Some(0xDEAD_BEEF),
        }
    }

    #[test]
    fn parses_the_documented_hello() {
        let hello = Hello::decode(&decode_hex(HELLO_FIXTURE)).expect("hello");
        assert_eq!(hello.magic, crate::transport::MAGIC);
        assert_eq!(hello.actual_version, 222);
        assert_eq!(hello.encode(), decode_hex(HELLO_FIXTURE));
    }

    #[test]
    fn challenge_round_trips() {
        let challenge = Challenge {
            challenge: 0x1234_5678,
        };
        assert_eq!(
            Challenge::decode(&challenge.encode()).expect("challenge"),
            challenge
        );
        assert_eq!(challenge.encode(), decode_hex("6589a4a578563412"));
    }

    #[test]
    fn parses_the_reference_connect_with_a_challenge() {
        let payload = decode_hex(CONNECT_160);
        assert_eq!(payload.len(), CONNECT_CHALLENGE_LEN);
        let connect = Connect::decode(&payload).expect("connect");
        assert_eq!(connect, connect_160());
        assert_eq!(connect.steam_id(), Some(STEAM_ID));
        assert_eq!(connect.challenge, Some(0xDEAD_BEEF));
        assert_eq!(connect.name, "J. Doe");
        assert_eq!(connect.password, "hunter2");
        assert_eq!(connect.build, GAME_BUILD);
        assert_eq!(connect.player_id, 0x1234);
        // The reference bytes are reproduced exactly, which pins every offset.
        assert_eq!(connect.encode(), payload);
    }

    #[test]
    fn parses_the_reference_connect_without_a_challenge() {
        let payload = decode_hex(CONNECT_156);
        assert_eq!(payload.len(), CONNECT_LEN);
        let connect = Connect::decode(&payload).expect("connect");
        assert!(!connect.with_challenge);
        assert_eq!(connect.challenge, None);
        assert_eq!(connect.encode(), payload);
        let mut expected = connect_160();
        expected.with_challenge = false;
        expected.challenge = None;
        assert_eq!(connect, expected);
    }

    #[test]
    fn rejects_a_connect_whose_id_and_length_disagree() {
        let mut wrong = decode_hex(CONNECT_160);
        wrong[0] = 0x12; // the challenge-less id with the challenged length
        assert!(matches!(
            Connect::decode(&wrong),
            Err(NetError::ConnectId { .. })
        ));
        assert!(matches!(
            Connect::decode(&decode_hex(CONNECT_160)[..0x9c]),
            Err(NetError::ConnectId { .. })
        ));
    }

    #[test]
    fn steam_blob_round_trips_and_rejects_a_wrong_marker() {
        assert_eq!(steam_blob_encode(STEAM_ID).to_vec(), decode_hex(STEAM_BLOB));
        assert_eq!(steam_blob_decode(&decode_hex(STEAM_BLOB)), Some(STEAM_ID));
        let mut bad = decode_hex(STEAM_BLOB);
        bad[0] ^= 0xFF;
        assert_eq!(steam_blob_decode(&bad), None);
    }

    #[test]
    fn result_round_trips_with_every_documented_code() {
        for (code, value) in [
            (ResultCode::Accepted, 0),
            (ResultCode::BadPassword, 1),
            (ResultCode::VersionMismatch, 2),
            (ResultCode::ServerError, 3),
            (ResultCode::Full, 4),
            (ResultCode::AntiCheat, 5),
            (ResultCode::CheckString, 6),
        ] {
            assert_eq!(code.as_u32(), value);
            assert_eq!(ResultCode::from_u32(value), Some(code));
            let result = ConnectResult { code, player_id: 7 };
            assert_eq!(
                ConnectResult::decode(&result.encode()).expect("result"),
                result
            );
        }
        let accepted = ConnectResult {
            code: ResultCode::Accepted,
            player_id: 3,
        };
        assert_eq!(accepted.encode(), decode_hex("7e1aa5aa0000000003000000"));
        assert!(ResultCode::from_u32(7).is_none());
    }

    #[test]
    fn session_query_round_trips() {
        let query = SessionQuery {
            magic: crate::transport::MAGIC,
            steam_blob: steam_blob_encode(STEAM_ID),
        };
        let encoded = query.encode();
        assert_eq!(encoded.len(), SESSION_QUERY_LEN);
        assert_eq!(SessionQuery::decode(&encoded).expect("query"), query);
        assert_eq!(query.steam_id(), Some(STEAM_ID));
    }

    #[test]
    fn control_dispatch_reads_the_leading_id() {
        assert!(matches!(
            Control::decode(&decode_hex(HELLO_FIXTURE)),
            Ok(Control::Hello(_))
        ));
        assert!(matches!(
            Control::decode(&decode_hex(CONNECT_156)),
            Ok(Control::Connect(_))
        ));
        assert!(matches!(
            Control::decode(&[]),
            Err(NetError::ControlLength { .. })
        ));
        assert!(matches!(
            Control::decode(&0xDEAD_BEEFu32.to_le_bytes()),
            Err(NetError::UnknownControl(0xDEAD_BEEF))
        ));
        assert!(matches!(
            Control::decode(&decode_hex(HELLO_FIXTURE)[..11]),
            Err(NetError::ControlLength { .. })
        ));
    }

    #[test]
    fn the_version_window_is_the_documented_one() {
        let server = Versions::target();
        let client = connect_160();
        assert_eq!(server.check(&client), None);

        let mut old_build = client.clone();
        old_build.build = GAME_BUILD - 1;
        assert_eq!(server.check(&old_build), Some(ResultCode::VersionMismatch));

        let mut old_version = client.clone();
        old_version.actual_version = 221;
        assert_eq!(
            server.check(&old_version),
            Some(ResultCode::VersionMismatch)
        );

        // A client that needs a newer server than we run is rejected too.
        let mut wants_newer = client.clone();
        wants_newer.required_version = 223;
        assert_eq!(
            server.check(&wants_newer),
            Some(ResultCode::VersionMismatch)
        );

        // A server that still accepts an older build lets that client in.
        let lenient = Versions {
            actual: 223,
            required: 222,
            required_build: GAME_BUILD - 10,
        };
        let mut older = client;
        older.build = GAME_BUILD - 5;
        assert_eq!(lenient.check(&older), None);
    }
}
