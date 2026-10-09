//! The Steam query protocol on the query port: requests, answers and challenges.
//!
//! The official server hands this to the Steamworks game-server library; `docs/re/net-a2s.md`
//! (ADR 0004) has us speak the standard Source query protocol ourselves, with the values the
//! document lists. Steam's implementation (2020+) answers every query with a challenge first and
//! only then with the data, and a client of any age retries with the challenge, so this module
//! does the same.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crate::bytes::{Cursor, Writer};
use crate::error::NetError;
use crate::handshake::GAME_VERSION_STRING;
use crate::random::SplitMix64;

use super::rules::RulesBlock;
use super::tags::GameTags;

/// Every A2S request and answer starts with four `0xFF` bytes.
pub const QUERY_HEADER: [u8; 4] = [0xFF; 4];

/// `T`: A2S_INFO.
pub const REQUEST_INFO: u8 = b'T';
/// `U`: A2S_PLAYER.
pub const REQUEST_PLAYER: u8 = b'U';
/// `V`: A2S_RULES.
pub const REQUEST_RULES: u8 = b'V';
/// `W`: A2S_SERVERQUERY_GETCHALLENGE.
pub const REQUEST_CHALLENGE: u8 = b'W';

/// `A`: S2C_CHALLENGE.
pub const RESPONSE_CHALLENGE: u8 = 0x41;
/// `I`: S2A_INFO (Source reply).
pub const RESPONSE_INFO: u8 = 0x49;
/// `D`: S2A_PLAYER.
pub const RESPONSE_PLAYER: u8 = 0x44;
/// `E`: S2A_RULES.
pub const RESPONSE_RULES: u8 = 0x45;

/// The protocol version in an A2S_INFO reply.
pub const INFO_PROTOCOL: u8 = 17;
/// The trailing part of an A2S_INFO request.
pub const INFO_REQUEST_STRING: &[u8] = b"Source Engine Query\0";
/// Arma 3's Steam AppID, published in the answer's 64-bit game id field.
pub const APP_ID: u32 = 107_410;

/// The A2S_INFO `AppID` field is only 16 bits wide, so it carries the low half of [`APP_ID`]
/// (41874). That is forced by the wire format, and the full id travels in the EDF `0x01` game id
/// field, which is what a 2020-era server browser reads.
pub const APP_ID_SHORT: u16 = APP_ID as u16;
/// `d`: a dedicated server.
pub const SERVER_TYPE_DEDICATED: u8 = b'd';
/// `w`: Windows.
pub const ENVIRONMENT_WINDOWS: u8 = b'w';
/// The challenge value that means "I have none yet" (`-1` in the protocol's signed field).
pub const CHALLENGE_UNSET: u32 = u32::MAX;

/// Extra Data Flag bit: the game port follows.
pub const EDF_PORT: u8 = 0x80;
/// Extra Data Flag bit: the keywords (game tags) string follows.
pub const EDF_KEYWORDS: u8 = 0x20;
/// Extra Data Flag bit: the 64-bit game id follows.
pub const EDF_GAME_ID: u8 = 0x01;
/// Extra Data Flag bit: the server's SteamID follows. We have none without Steamworks.
pub const EDF_STEAM_ID: u8 = 0x10;

/// How long an issued challenge stays valid. Steam's own lifetime is not part of the engine RE:
/// this is our choice, and it only has to outlast a query tool's retry.
pub const CHALLENGE_LIFETIME: Duration = Duration::from_secs(30);

/// One A2S request this server answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Query {
    /// A2S_INFO, with the challenge when the client has been through the challenge dance.
    Info { challenge: Option<u32> },
    /// A2S_PLAYER; `CHALLENGE_UNSET` means the client has none yet.
    Player { challenge: u32 },
    /// A2S_RULES; `CHALLENGE_UNSET` means the client has none yet.
    Rules { challenge: u32 },
    /// A2S_SERVERQUERY_GETCHALLENGE, which only asks for a challenge.
    Challenge,
}

impl Query {
    /// The challenge this request carries, if any.
    pub fn challenge(&self) -> Option<u32> {
        match *self {
            Self::Info { challenge } => challenge,
            Self::Player { challenge } | Self::Rules { challenge } => {
                (challenge != CHALLENGE_UNSET).then_some(challenge)
            }
            Self::Challenge => None,
        }
    }
}

/// Parse one A2S request datagram.
pub fn parse_query(datagram: &[u8]) -> Result<Query, NetError> {
    let len = datagram.len();
    if len < 5 {
        return Err(NetError::A2sShort { len });
    }
    let mut cursor = Cursor::new(datagram);
    let header: [u8; 4] = cursor
        .bytes(4)
        .expect("length checked")
        .try_into()
        .expect("fixed width");
    let kind = cursor.u8().expect("length checked");
    if header != QUERY_HEADER {
        return Err(NetError::A2sUnknownQuery { header, kind });
    }
    match kind {
        REQUEST_INFO => {
            // The request string is part of the query; a client that omits it is not one we know.
            let text = cursor.string().expect("length checked");
            if !text.starts_with("Source Engine Query") {
                return Err(NetError::A2sUnknownQuery { header, kind });
            }
            let challenge = if cursor.remaining() >= 4 {
                cursor.u32()
            } else {
                None
            };
            Ok(Query::Info { challenge })
        }
        REQUEST_PLAYER => Ok(Query::Player {
            challenge: cursor.u32().unwrap_or(CHALLENGE_UNSET),
        }),
        REQUEST_RULES => Ok(Query::Rules {
            challenge: cursor.u32().unwrap_or(CHALLENGE_UNSET),
        }),
        REQUEST_CHALLENGE => Ok(Query::Challenge),
        _ => Err(NetError::A2sUnknownQuery { header, kind }),
    }
}

/// The `S2C_CHALLENGE` answer to any query that needs one.
pub fn challenge_response(challenge: u32) -> Vec<u8> {
    Writer::new()
        .bytes(&QUERY_HEADER)
        .u8(RESPONSE_CHALLENGE)
        .u32(challenge)
        .finish()
}

/// The fields of an A2S_INFO answer (`docs/re/net-a2s.md`, "A2S_INFO fields").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// Server hostname (server.cfg `hostname`).
    pub name: String,
    /// The world (terrain) name.
    pub map: String,
    /// `"Arma3"`, as the engine tells Steam.
    pub folder: String,
    /// The mission name, `"Waiting"` while no mission is loaded.
    pub game: String,
    /// 107410 for Arma 3.
    pub app_id: u16,
    /// Players connected, reported by the server itself (no Steam user list here).
    pub players: u8,
    /// `SetMaxPlayerCount(maxPlayers)`.
    pub max_players: u8,
    /// Bots on the server: always 0 for us.
    pub bots: u8,
    pub server_type: u8,
    pub environment: u8,
    /// `SetPasswordProtected`.
    pub password: bool,
    /// BattlEye secured: always false on our server (ADR 0004).
    pub vac: bool,
    /// The version string given at init, `2.22.<build>`.
    pub version: String,
    /// The game port (game port, not query port), reported through [`EDF_PORT`].
    pub port: u16,
    /// The game tags, reported through [`EDF_KEYWORDS`].
    pub keywords: String,
    /// The Steam AppID as a 64-bit game id, reported through [`EDF_GAME_ID`].
    pub game_id: u64,
}

impl Info {
    /// An A2S_INFO answer for a server with no mission loaded.
    pub fn waiting(name: String, map: String, port: u16, max_players: u8, password: bool) -> Self {
        Self {
            name,
            map,
            folder: "Arma3".into(),
            game: "Waiting".into(),
            app_id: APP_ID_SHORT,
            players: 0,
            max_players,
            bots: 0,
            server_type: SERVER_TYPE_DEDICATED,
            environment: ENVIRONMENT_WINDOWS,
            password,
            vac: false,
            version: GAME_VERSION_STRING.into(),
            port,
            keywords: String::new(),
            game_id: u64::from(APP_ID),
        }
    }

    /// Serialise this answer.
    ///
    /// The optional fields follow the Extra Data Flag bits in descending order of their bit value,
    /// which is the order every query tool reads them in: port (0x80), keywords (0x20), game id
    /// (0x01). The SteamID bit (0x10) stays clear: without Steamworks we have no SteamID to report,
    /// and a made-up one would be worse than none.
    pub fn encode(&self) -> Result<Vec<u8>, NetError> {
        let edf = EDF_PORT | EDF_KEYWORDS | EDF_GAME_ID;
        let mut writer = Writer::new()
            .bytes(&QUERY_HEADER)
            .u8(RESPONSE_INFO)
            .u8(INFO_PROTOCOL)
            .string(&self.name)
            .string(&self.map)
            .string(&self.folder)
            .string(&self.game)
            .u16(self.app_id)
            .u8(self.players)
            .u8(self.max_players)
            .u8(self.bots)
            .u8(self.server_type)
            .u8(self.environment)
            .u8(u8::from(self.password))
            .u8(u8::from(self.vac))
            .string(&self.version)
            .u8(edf);
        if edf & EDF_PORT != 0 {
            writer = writer.u16(self.port);
        }
        if edf & EDF_KEYWORDS != 0 {
            writer = writer.string(&self.keywords);
        }
        if edf & EDF_GAME_ID != 0 {
            writer = writer.u64(self.game_id);
        }
        Ok(writer.finish())
    }
}

/// One row of an A2S_PLAYER answer.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerEntry {
    /// The player's index, counting from 1.
    pub index: u8,
    /// The player's profile name.
    pub name: String,
    /// The score the server reports. Which score the official server sends is still unknown
    /// (`net-a2s.md`, "A2S_PLAYER"), so we report 0.
    pub score: i32,
    /// Seconds since the player joined.
    pub duration: f32,
}

/// Serialise an A2S_PLAYER answer.
pub fn player_response(players: &[PlayerEntry]) -> Result<Vec<u8>, NetError> {
    if players.len() > 255 {
        return Err(NetError::A2sFieldTooLong {
            field: "player count",
            len: players.len(),
            max: 255,
        });
    }
    let mut writer = Writer::new()
        .bytes(&QUERY_HEADER)
        .u8(RESPONSE_PLAYER)
        .u8(players.len() as u8);
    for player in players {
        writer = writer
            .u8(player.index)
            .string(&player.name)
            .i32(player.score)
            .f32(player.duration);
    }
    Ok(writer.finish())
}

/// One A2S_RULES entry. Arma's own key is two raw bytes, so keys are byte strings, not text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

impl Rule {
    /// A textual rule, the ordinary kind.
    pub fn text(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into().into_bytes(),
            value: value.into().into_bytes(),
        }
    }
}

/// Serialise an A2S_RULES answer.
///
/// Both parts of a rule are NUL-terminated strings, so neither may contain a NUL byte; Arma's
/// binary block is escaped precisely to guarantee that.
pub fn rules_response(rules: &[Rule]) -> Result<Vec<u8>, NetError> {
    if rules.len() > 255 {
        return Err(NetError::A2sFieldTooLong {
            field: "rule count",
            len: rules.len(),
            max: 255,
        });
    }
    for rule in rules {
        for (field, bytes) in [("rule key", &rule.key), ("rule value", &rule.value)] {
            if bytes.contains(&0) {
                return Err(NetError::A2sFieldTooLong {
                    field,
                    len: bytes.len(),
                    max: bytes.iter().position(|b| *b == 0).unwrap_or(0),
                });
            }
        }
    }
    let mut writer = Writer::new()
        .bytes(&QUERY_HEADER)
        .u8(RESPONSE_RULES)
        .u16(rules.len() as u16);
    for rule in rules {
        writer = writer.raw_string(&rule.key).raw_string(&rule.value);
    }
    Ok(writer.finish())
}

/// The challenges issued to querying addresses.
#[derive(Debug)]
pub struct ChallengeTable {
    entries: HashMap<SocketAddr, (u32, Instant)>,
    rng: SplitMix64,
}

impl ChallengeTable {
    pub fn new(rng: SplitMix64) -> Self {
        Self {
            entries: HashMap::new(),
            rng,
        }
    }

    /// The challenge for `from`, reusing a live one so a tool that queries info, rules and players
    /// in a row needs the dance only once.
    pub fn issue(&mut self, from: SocketAddr, now: Instant) -> u32 {
        if let Some((challenge, issued)) = self.entries.get(&from) {
            if now.saturating_duration_since(*issued) < CHALLENGE_LIFETIME {
                return *challenge;
            }
        }
        let challenge = self.rng.next_u32();
        self.entries.insert(from, (challenge, now));
        challenge
    }

    /// Whether `challenge` is the live challenge for `from`.
    pub fn accepts(&mut self, from: SocketAddr, challenge: u32, now: Instant) -> bool {
        match self.entries.get(&from) {
            Some((issued, at)) => {
                now.saturating_duration_since(*at) < CHALLENGE_LIFETIME && *issued == challenge
            }
            None => false,
        }
    }

    /// Drop expired entries; the server calls this while it waits for datagrams.
    pub fn expire(&mut self, now: Instant) {
        self.entries
            .retain(|_, (_, issued)| now.saturating_duration_since(*issued) < CHALLENGE_LIFETIME);
    }
}

/// Everything the three answers are built from.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerState {
    pub info: Info,
    pub rules: RulesBlock,
    pub players: Vec<PlayerEntry>,
    /// The game tags; the `keywords` field of [`Info`] is filled from them at answer time.
    pub tags: GameTags,
}

/// Answers A2S requests with the server's current state.
#[derive(Debug)]
pub struct Responder {
    state: ServerState,
    challenges: ChallengeTable,
}

impl Responder {
    pub fn new(state: ServerState, rng: SplitMix64) -> Self {
        Self {
            state,
            challenges: ChallengeTable::new(rng),
        }
    }

    /// The state the answers are built from, which the server updates as players come and go.
    pub fn state_mut(&mut self) -> &mut ServerState {
        &mut self.state
    }

    /// Drop expired challenges.
    pub fn expire(&mut self, now: Instant) {
        self.challenges.expire(now);
    }

    /// Answer one datagram, or return `Ok(None)` when it is not a query we handle.
    ///
    /// A query without a valid challenge is answered with a new challenge, as Steam's own
    /// implementation does; the client repeats the request with it.
    pub fn answer(
        &mut self,
        datagram: &[u8],
        from: SocketAddr,
        now: Instant,
    ) -> Result<Option<Vec<u8>>, NetError> {
        let query = match parse_query(datagram) {
            Ok(query) => query,
            Err(error) => {
                log::debug!("a2s: ignoring a datagram from {from}: {error}");
                return Ok(None);
            }
        };
        if let Query::Challenge = query {
            let challenge = self.challenges.issue(from, now);
            return Ok(Some(challenge_response(challenge)));
        }
        let valid = query
            .challenge()
            .is_some_and(|challenge| self.challenges.accepts(from, challenge, now));
        if !valid {
            let challenge = self.challenges.issue(from, now);
            return Ok(Some(challenge_response(challenge)));
        }
        Ok(Some(match query {
            Query::Info { .. } => {
                let info = Info {
                    players: self.state.players.len().min(255) as u8,
                    keywords: self.state.tags.encode(),
                    ..self.state.info.clone()
                };
                info.encode()?
            }
            Query::Player { .. } => player_response(&self.state.players)?,
            Query::Rules { .. } => {
                let rules: Vec<Rule> = self
                    .state
                    .rules
                    .chunks()?
                    .into_iter()
                    .map(|(key, value)| Rule { key, value })
                    .collect();
                rules_response(&rules)?
            }
            Query::Challenge => return Ok(None),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    fn info_request(challenge: Option<u32>) -> Vec<u8> {
        let mut out = QUERY_HEADER.to_vec();
        out.push(REQUEST_INFO);
        out.extend_from_slice(INFO_REQUEST_STRING);
        if let Some(challenge) = challenge {
            out.extend_from_slice(&challenge.to_le_bytes());
        }
        out
    }

    fn player_request(challenge: u32) -> Vec<u8> {
        let mut out = QUERY_HEADER.to_vec();
        out.push(REQUEST_PLAYER);
        out.extend_from_slice(&challenge.to_le_bytes());
        out
    }

    fn rules_request(challenge: u32) -> Vec<u8> {
        let mut out = QUERY_HEADER.to_vec();
        out.push(REQUEST_RULES);
        out.extend_from_slice(&challenge.to_le_bytes());
        out
    }

    fn responder() -> Responder {
        let state = ServerState {
            info: Info::waiting(
                "a3-rust test server".into(),
                "altis".into(),
                2302,
                64,
                false,
            ),
            rules: RulesBlock::default(),
            players: Vec::new(),
            tags: GameTags::default(),
        };
        Responder::new(state, SplitMix64::new(0xA3))
    }

    fn challenge_of(answer: &[u8]) -> u32 {
        assert_eq!(&answer[..4], &QUERY_HEADER);
        assert_eq!(answer[4], RESPONSE_CHALLENGE);
        u32::from_le_bytes([answer[5], answer[6], answer[7], answer[8]])
    }

    #[test]
    fn parses_every_request_form() {
        assert_eq!(
            parse_query(&info_request(None)).expect("info"),
            Query::Info { challenge: None }
        );
        assert_eq!(
            parse_query(&info_request(Some(7))).expect("info"),
            Query::Info { challenge: Some(7) }
        );
        assert_eq!(
            parse_query(&player_request(CHALLENGE_UNSET)).expect("player"),
            Query::Player {
                challenge: CHALLENGE_UNSET
            }
        );
        assert_eq!(
            parse_query(&rules_request(9)).expect("rules"),
            Query::Rules { challenge: 9 }
        );
        let mut getchallenge = QUERY_HEADER.to_vec();
        getchallenge.push(REQUEST_CHALLENGE);
        assert_eq!(
            parse_query(&getchallenge).expect("challenge"),
            Query::Challenge
        );

        assert!(matches!(
            parse_query(&[0xFF, 0xFF]),
            Err(NetError::A2sShort { .. })
        ));
        assert!(matches!(
            parse_query(&[0, 0, 0, 0, REQUEST_INFO, 0]),
            Err(NetError::A2sUnknownQuery { .. })
        ));
        assert!(matches!(
            parse_query(&[0xFF, 0xFF, 0xFF, 0xFF, b'X']),
            Err(NetError::A2sUnknownQuery { .. })
        ));
        // An info request without the query string is not one we answer.
        let mut no_string = QUERY_HEADER.to_vec();
        no_string.push(REQUEST_INFO);
        no_string.extend_from_slice(b"nonsense\0");
        assert!(matches!(
            parse_query(&no_string),
            Err(NetError::A2sUnknownQuery { .. })
        ));
    }

    #[test]
    fn every_query_needs_a_challenge_first_and_then_works() {
        let now = Instant::now();
        let mut responder = responder();
        for request in [
            info_request(None),
            player_request(CHALLENGE_UNSET),
            rules_request(CHALLENGE_UNSET),
        ] {
            let answer = responder
                .answer(&request, addr(1000), now)
                .expect("answer")
                .expect("a reply");
            let challenge = challenge_of(&answer);
            // The same address keeps the same challenge, so one dance serves every query.
            let second = responder
                .answer(&player_request(CHALLENGE_UNSET), addr(1000), now)
                .expect("answer")
                .expect("a reply");
            assert_eq!(challenge_of(&second), challenge);
            let with_challenge: Vec<u8> = match request[4] {
                REQUEST_INFO => info_request(Some(challenge)),
                REQUEST_PLAYER => player_request(challenge),
                _ => rules_request(challenge),
            };
            let answer = responder
                .answer(&with_challenge, addr(1000), now)
                .expect("answer")
                .expect("a reply");
            assert_eq!(&answer[..4], &QUERY_HEADER);
            assert_ne!(answer[4], RESPONSE_CHALLENGE);
        }
    }

    #[test]
    fn a_wrong_or_expired_challenge_is_replaced() {
        let now = Instant::now();
        let mut responder = responder();
        let answer = responder
            .answer(&info_request(None), addr(1), now)
            .expect("answer")
            .expect("reply");
        let challenge = challenge_of(&answer);

        // A wrong challenge gets a fresh one instead of data.
        let answer = responder
            .answer(&info_request(Some(challenge ^ 1)), addr(1), now)
            .expect("answer")
            .expect("reply");
        assert_eq!(answer[4], RESPONSE_CHALLENGE);

        // The right challenge still works after that.
        let answer = responder
            .answer(&info_request(Some(challenge)), addr(1), now)
            .expect("answer")
            .expect("reply");
        assert_eq!(answer[4], RESPONSE_INFO);

        // After the lifetime, the challenge is stale.
        let later = now + CHALLENGE_LIFETIME + Duration::from_secs(1);
        let answer = responder
            .answer(&info_request(Some(challenge)), addr(1), later)
            .expect("answer")
            .expect("reply");
        assert_eq!(answer[4], RESPONSE_CHALLENGE);
    }

    #[test]
    fn the_challenge_request_form_is_answered_directly() {
        let now = Instant::now();
        let mut responder = responder();
        let mut request = QUERY_HEADER.to_vec();
        request.push(REQUEST_CHALLENGE);
        let answer = responder
            .answer(&request, addr(2), now)
            .expect("answer")
            .expect("reply");
        assert_eq!(answer.len(), 9);
        challenge_of(&answer);
    }

    #[test]
    fn the_info_answer_has_the_documented_layout() {
        let now = Instant::now();
        let mut responder = responder();
        let answer = responder
            .answer(&info_request(None), addr(3), now)
            .expect("answer")
            .expect("reply");
        let challenge = challenge_of(&answer);
        let answer = responder
            .answer(&info_request(Some(challenge)), addr(3), now)
            .expect("answer")
            .expect("reply");
        let mut cursor = Cursor::new(&answer);
        assert_eq!(cursor.bytes(4), Some(&QUERY_HEADER[..]));
        assert_eq!(cursor.u8(), Some(RESPONSE_INFO));
        assert_eq!(cursor.u8(), Some(INFO_PROTOCOL));
        assert_eq!(cursor.string().as_deref(), Some("a3-rust test server"));
        assert_eq!(cursor.string().as_deref(), Some("altis"));
        assert_eq!(cursor.string().as_deref(), Some("Arma3"));
        assert_eq!(cursor.string().as_deref(), Some("Waiting"));
        assert_eq!(cursor.u16(), Some(APP_ID_SHORT));
        assert_eq!(cursor.u8(), Some(0), "players");
        assert_eq!(cursor.u8(), Some(64), "max players");
        assert_eq!(cursor.u8(), Some(0), "bots");
        assert_eq!(cursor.u8(), Some(SERVER_TYPE_DEDICATED));
        assert_eq!(cursor.u8(), Some(ENVIRONMENT_WINDOWS));
        assert_eq!(cursor.u8(), Some(0), "not password protected");
        assert_eq!(cursor.u8(), Some(0), "no BattlEye");
        assert_eq!(cursor.string().as_deref(), Some(GAME_VERSION_STRING));
        let edf = cursor.u8().expect("edf");
        assert_eq!(edf, EDF_PORT | EDF_KEYWORDS | EDF_GAME_ID);
        assert_eq!(cursor.u16(), Some(2302), "the game port");
        assert_eq!(
            cursor.string().as_deref(),
            Some("bf,mf,r222,n154103,s0,dt,lf,vf,g0,i0,pw,e15,f0,")
        );
        assert_eq!(cursor.u64(), Some(u64::from(APP_ID)));
        assert_eq!(cursor.remaining(), 0, "nothing is left over");
    }

    #[test]
    fn the_player_answer_lists_the_players_the_server_reports() {
        let now = Instant::now();
        let mut responder = responder();
        responder.state_mut().players = vec![
            PlayerEntry {
                index: 1,
                name: "J. Doe".into(),
                score: 0,
                duration: 12.5,
            },
            PlayerEntry {
                index: 2,
                name: "K. Roe".into(),
                score: 0,
                duration: 3.0,
            },
        ];
        let challenge = challenge_of(
            &responder
                .answer(&rules_request(CHALLENGE_UNSET), addr(4), now)
                .expect("answer")
                .expect("reply"),
        );
        let answer = responder
            .answer(&player_request(challenge), addr(4), now)
            .expect("answer")
            .expect("reply");
        let mut cursor = Cursor::new(&answer);
        assert_eq!(cursor.bytes(4), Some(&QUERY_HEADER[..]));
        assert_eq!(cursor.u8(), Some(RESPONSE_PLAYER));
        assert_eq!(cursor.u8(), Some(2));
        assert_eq!(cursor.u8(), Some(1));
        assert_eq!(cursor.string().as_deref(), Some("J. Doe"));
        assert_eq!(cursor.i32(), Some(0));
        assert_eq!(cursor.f32(), Some(12.5));
        assert_eq!(cursor.u8(), Some(2));
        assert_eq!(cursor.string().as_deref(), Some("K. Roe"));
        assert_eq!(cursor.i32(), Some(0));
        assert_eq!(cursor.f32(), Some(3.0));
        assert_eq!(cursor.remaining(), 0);

        // The player count in A2S_INFO follows the list.
        let info_challenge = challenge_of(
            &responder
                .answer(&info_request(None), addr(4), now)
                .expect("answer")
                .expect("reply"),
        );
        let answer = responder
            .answer(&info_request(Some(info_challenge)), addr(4), now)
            .expect("answer")
            .expect("reply");
        let players_at = 4
            + 1
            + 1
            + "a3-rust test server".len()
            + 1
            + "altis".len()
            + 1
            + "Arma3".len()
            + 1
            + "Waiting".len()
            + 1
            + 2;
        assert_eq!(answer[players_at], 2, "the info answer counts two players");
    }

    #[test]
    fn the_rules_answer_carries_the_binary_block_as_chunks() {
        let now = Instant::now();
        let mut responder = responder();
        let challenge = challenge_of(
            &responder
                .answer(&rules_request(CHALLENGE_UNSET), addr(5), now)
                .expect("answer")
                .expect("reply"),
        );
        let answer = responder
            .answer(&rules_request(challenge), addr(5), now)
            .expect("answer")
            .expect("reply");
        let mut cursor = Cursor::new(&answer);
        assert_eq!(cursor.bytes(4), Some(&QUERY_HEADER[..]));
        assert_eq!(cursor.u8(), Some(RESPONSE_RULES));
        assert_eq!(cursor.u16(), Some(1), "one chunk");
        let key = cursor.string().expect("key");
        assert_eq!(key.as_bytes(), &[1u8, 1u8]);
        let value = cursor.string().expect("value");
        assert_eq!(cursor.remaining(), 0);
        // The published value unescapes to the documented block.
        assert_eq!(
            super::super::rules::unescape(value.as_bytes()),
            Some(vec![3, 0, 0, 0, 0, 0, 0, 0])
        );
    }

    #[test]
    fn a_rules_key_or_value_containing_a_nul_is_refused() {
        assert!(matches!(
            rules_response(&[Rule {
                key: vec![1, 0],
                value: vec![2]
            }]),
            Err(NetError::A2sFieldTooLong { .. })
        ));
        assert!(matches!(
            rules_response(&[Rule {
                key: vec![1],
                value: vec![2, 0]
            }]),
            Err(NetError::A2sFieldTooLong { .. })
        ));
        assert!(rules_response(&[Rule::text("hostname", "server")]).is_ok());
    }

    #[test]
    fn garbage_is_ignored_rather_than_answered() {
        let now = Instant::now();
        let mut responder = responder();
        assert_eq!(
            responder
                .answer(b"not a query at all", addr(6), now)
                .expect("answer"),
            None
        );
        assert_eq!(responder.answer(&[], addr(6), now).expect("answer"), None);
    }
}
