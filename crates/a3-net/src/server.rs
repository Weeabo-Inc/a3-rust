//! The dedicated server's session state and its UDP loop (`docs/re/net-handshake.md`).
//!
//! [`Server`] owns no sockets: it takes a received datagram and returns the datagrams to send
//! back, so the whole handshake is testable without a network. [`BoundServer`] is the thin socket
//! wrapper the binary runs.
//!
//! What the server checks, in the order the document lists them: the message length and the net
//! magic, `player_id >= 0x14`, the version window, the password, a duplicate address, a free
//! player ID in the proposed 12-wide window and room for another player. The anti-cheat check is
//! skipped on purpose: ADR 0004 has our server accept every BattlEye state, and the check string
//! is only compared when the server enables it, which we do not.
//!
//! Deliberately not here yet, all of it documented and each tracked in the issue the PRs
//! reference: the session-query reply layout (`net-handshake.md` calls it a follow-up), the
//! keepalive question, the `DContext` message layer and the game messages behind it.

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::channel::Channel;
use crate::error::NetError;
use crate::handshake::{
    Challenge, Connect, ConnectResult, Control, MIN_PLAYER_ID, ResultCode, Versions,
};
use crate::random::SplitMix64;
use crate::transport::{Header, Keys, MAGIC, flags, packet};

/// How long a pending challenge entry lives (`0x7b9790(list, 8000000)`).
pub const PENDING_LIFETIME: Duration = Duration::from_secs(8);

/// How long after the first challenge-less CONNECT a repeat is processed like a challenged one.
pub const CHALLENGE_LESS_REPEAT: Duration = Duration::from_millis(1);

/// Width of the player-ID window: the server takes the first free ID in `[proposed, proposed+12)`.
pub const PLAYER_ID_WINDOW: i32 = 12;

/// The receive buffer. The engine reads into 0x7ff bytes with a hard limit of 0x800.
pub const RECV_BUFFER: usize = 0x800;

/// How long one [`BoundServer::poll`] waits for a datagram before it services resends and expiry.
pub const POLL_TICK: Duration = Duration::from_millis(100);

/// Everything the server needs to start.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Where the game socket binds; 2302 in the original.
    pub bind: SocketAddr,
    /// The obfuscation magic; both sides of this build use [`MAGIC`].
    pub magic: u32,
    /// Server hostname (server.cfg `hostname`).
    pub hostname: String,
    /// Join password; empty means the server has none.
    pub password: String,
    pub max_players: u8,
    /// The version/build window the server accepts.
    pub versions: Versions,
    /// World (terrain) name.
    pub world: String,
    /// Mission name, or `None` while no mission is loaded.
    pub mission: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([0, 0, 0, 0], 2302)),
            magic: MAGIC,
            hostname: "a3-rust dedicated server".into(),
            password: String::new(),
            max_players: 64,
            versions: Versions::target(),
            world: "altis".into(),
            mission: None,
        }
    }
}

/// A player the server has accepted.
#[derive(Debug, Clone)]
pub struct Player {
    /// The player ID (dpnid) the server assigned.
    pub id: u32,
    pub addr: SocketAddr,
    pub name: String,
    /// The SteamID64 from the CONNECT blob, when it decoded.
    pub steam_id: Option<u64>,
    pub build: u32,
    pub check_string: String,
    /// `flag A` from the CONNECT, kept in the player record by the original.
    pub flag_a: u8,
    /// `flag B` from the CONNECT; bit 0 marks a "local/host" player in the original.
    pub flag_b: u8,
    /// The client's BattlEye state, accepted but never checked.
    pub be_state: u8,
    /// The channel of this connection: serials, acknowledgements and the reliable outbox.
    pub channel: Channel,
}

/// A datagram the server wants sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub to: SocketAddr,
    pub datagram: Vec<u8>,
}

/// A challenge issued to one address, until it expires.
#[derive(Debug, Clone, Copy)]
struct PendingChallenge {
    challenge: u32,
    created: Instant,
}

/// The session state of one dedicated server.
#[derive(Debug)]
pub struct Server {
    config: ServerConfig,
    keys: Keys,
    rng: SplitMix64,
    /// Serial counter for connection-less control datagrams.
    ///
    /// Which serials the original uses before a channel exists is not documented
    /// (`net-transport.md` only says `F_CONTROL` datagrams bypass the channel's serial
    /// bookkeeping), so we count from 1 and never ack them; the gap is in the referenced issue.
    control_serial: u32,
    pending: HashMap<SocketAddr, PendingChallenge>,
    /// The first challenge-less CONNECT of each address, which is only recorded.
    seen_connect: HashMap<SocketAddr, Instant>,
    players: Vec<Player>,
}

impl Server {
    /// A server whose challenges come from the system clock.
    pub fn new(config: ServerConfig) -> Self {
        Self::with_rng(config, SplitMix64::from_clock(0xA3))
    }

    /// A server with an explicit generator, which is what tests use.
    pub fn with_rng(config: ServerConfig, rng: SplitMix64) -> Self {
        let keys = Keys::derive(config.magic);
        Self {
            config,
            keys,
            rng,
            control_serial: 1,
            pending: HashMap::new(),
            seen_connect: HashMap::new(),
            players: Vec::new(),
        }
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// The players currently connected.
    pub fn players(&self) -> &[Player] {
        &self.players
    }

    /// Handle one datagram received on the game port.
    ///
    /// A datagram that fails any transport check is dropped, which is what the original's receive
    /// path does; the reason is logged at debug level.
    pub fn on_game_datagram(
        &mut self,
        datagram: &[u8],
        from: SocketAddr,
        now: Instant,
    ) -> Vec<Reply> {
        let (header, payload) = match packet::unpack(datagram, &self.keys) {
            Ok(parsed) => parsed,
            Err(error) => {
                log::debug!("dropping a datagram from {from}: {error}");
                return Vec::new();
            }
        };
        // `F_CONTROL` and no-channel datagrams go to the default handler, everything else to the
        // connection's channel (`net-transport.md`, "Receive path").
        if header.flags & (flags::CONTROL | flags::NOCHANNEL) != 0 {
            self.on_control(&header, &payload, from, now)
        } else {
            self.on_channel(&header, &payload, from)
        }
    }

    /// Retry every reliable datagram whose acknowledgement is overdue.
    pub fn resend_due(&mut self, now: Instant) -> Vec<Reply> {
        let keys = &self.keys;
        let mut replies = Vec::new();
        for player in &mut self.players {
            for outgoing in player.channel.resend_due(now) {
                match outgoing.pack(keys) {
                    Ok(datagram) => replies.push(Reply {
                        to: player.addr,
                        datagram,
                    }),
                    Err(error) => log::warn!("cannot resend to {}: {error}", player.addr),
                }
            }
        }
        replies
    }

    /// Drop expired challenges and stale challenge-less CONNECT records.
    pub fn expire(&mut self, now: Instant) {
        self.pending
            .retain(|_, entry| now.saturating_duration_since(entry.created) < PENDING_LIFETIME);
        self.seen_connect
            .retain(|_, first| now.saturating_duration_since(*first) < PENDING_LIFETIME);
    }

    /// A connection-less control message, which is everything the handshake uses.
    fn on_control(
        &mut self,
        header: &Header,
        payload: &[u8],
        from: SocketAddr,
        now: Instant,
    ) -> Vec<Reply> {
        match Control::decode(payload) {
            Ok(Control::Hello(hello)) => {
                if hello.magic != self.config.magic {
                    // The original rejects a mismatched magic silently.
                    log::debug!(
                        "HELLO from {from} has magic {:#010x}, not ours; ignored",
                        hello.magic
                    );
                    return Vec::new();
                }
                log::info!(
                    "HELLO from {from}: client version {}, serial {}",
                    hello.actual_version,
                    header.serial
                );
                let challenge = self.challenge_for(from, now);
                self.control(
                    from,
                    flags::FLAGS_CONTROL,
                    &Challenge { challenge }.encode(),
                )
                .into_iter()
                .collect()
            }
            Ok(Control::Connect(connect)) => self.on_connect(connect, from, now),
            Ok(Control::SessionQuery(query)) => {
                // The reply is the session-info block at server+0x198; `net-handshake.md` calls its
                // full layout a follow-up, so we log the request instead of inventing one.
                log::info!(
                    "session info request from {from} (steam id {:?}): reply layout is not documented yet",
                    query.steam_id()
                );
                Vec::new()
            }
            Err(error) => {
                log::debug!("control message from {from}: {error}");
                Vec::new()
            }
        }
    }

    /// A datagram on a connection's channel: acknowledgements first, then the serial check.
    fn on_channel(&mut self, header: &Header, payload: &[u8], from: SocketAddr) -> Vec<Reply> {
        let Some(player) = self.players.iter_mut().find(|player| player.addr == from) else {
            log::debug!("datagram from {from} without a session; dropped");
            return Vec::new();
        };
        player.channel.acknowledge(header);
        if let Err(error) = player.channel.accept(header) {
            // "*** NetChannel (%u) refused a packet: serialID=%u" in the original.
            log::debug!("refused a packet from {from}: {error}");
            return Vec::new();
        }
        // The game message layer (`docs/re/net-messages.md`) is the next slice: for now the
        // payload is logged so a real client's traffic is visible while the handshake is proven.
        log::info!(
            "{} bytes of channel payload from {from} (player {}), game messages not implemented yet",
            payload.len(),
            player.id
        );
        Vec::new()
    }

    /// The checks and the answer of one CONNECT.
    fn on_connect(&mut self, connect: Connect, from: SocketAddr, now: Instant) -> Vec<Reply> {
        if !self.challenge_ok(&connect, from, now) {
            return Vec::new();
        }
        if connect.magic != self.config.magic {
            log::debug!(
                "CONNECT from {from} has magic {:#010x}, not ours; ignored",
                connect.magic
            );
            return Vec::new();
        }
        if connect.player_id < MIN_PLAYER_ID {
            log::debug!(
                "CONNECT from {from} proposes player id {}; below the minimum, dropped",
                connect.player_id
            );
            return Vec::new();
        }
        if let Some(code) = self.config.versions.check(&connect) {
            log::info!(
                "rejecting {} from {from}: version/build mismatch (client {} build {})",
                connect.name,
                connect.actual_version,
                connect.build
            );
            return self.reject(from, code);
        }
        if !self.config.password.is_empty() && connect.password != self.config.password {
            log::info!("rejecting {} from {from}: wrong password", connect.name);
            return self.reject(from, ResultCode::BadPassword);
        }

        // An address that connects again replaces its old session; the original destroys the old
        // channel first and reuses the ID when the client proposes the same one.
        if let Some(index) = self.players.iter().position(|player| player.addr == from) {
            if self.players[index].id == connect.player_id as u32 {
                let id = self.players[index].id;
                log::info!("{from} reconnected with player id {id}; answering again");
                return self.accept_reply(from, id, now);
            }
            log::info!("{from} reconnected with a new id; dropping the old session");
            self.players.remove(index);
        }

        if self.players.len() >= usize::from(self.config.max_players) {
            log::info!(
                "rejecting {} from {from}: server full ({} players)",
                connect.name,
                self.players.len()
            );
            return self.reject(from, ResultCode::Full);
        }
        let Some(id) = self.free_player_id(connect.player_id) else {
            log::info!(
                "rejecting {} from {from}: no free player id in [{}, {})",
                connect.name,
                connect.player_id,
                connect.player_id + PLAYER_ID_WINDOW
            );
            return self.reject(from, ResultCode::Full);
        };

        let mut channel = Channel::new(self.rng.next_serial());
        let reply = accept_reply_with(&self.keys, &mut channel, from, id, now);
        let steam_id = connect.steam_id();
        log::info!(
            "player {} ({}) from {from} joined: build {}, steam id {steam_id:?}, flags {}/{}, BE {}",
            id,
            connect.name,
            connect.build,
            connect.flag_a,
            connect.flag_b,
            connect.be_state
        );
        self.players.push(Player {
            id,
            addr: from,
            name: connect.name,
            steam_id,
            build: connect.build,
            check_string: connect.check_string,
            flag_a: connect.flag_a,
            flag_b: connect.flag_b,
            be_state: connect.be_state,
            channel,
        });
        reply.into_iter().collect()
    }

    /// Whether this CONNECT passes the challenge rules.
    ///
    /// A challenged CONNECT must echo the entry recorded for its address. The challenge-less form
    /// is recorded on first sight and processed only when it repeats at least a millisecond later,
    /// which is what the original's pending list does (`net-handshake.md`, "Sequence").
    fn challenge_ok(&mut self, connect: &Connect, from: SocketAddr, now: Instant) -> bool {
        if connect.with_challenge {
            let Some(entry) = self.pending.get(&from) else {
                log::debug!("CONNECT from {from} echoes a challenge we never issued; dropped");
                return false;
            };
            let fresh = now.saturating_duration_since(entry.created) < PENDING_LIFETIME;
            if !fresh || Some(entry.challenge) != connect.challenge {
                log::debug!("CONNECT from {from} echoes a wrong or stale challenge; dropped");
                return false;
            }
            return true;
        }
        match self.seen_connect.insert(from, now) {
            None => {
                log::debug!(
                    "first challenge-less CONNECT from {from} recorded; a repeat is processed"
                );
                false
            }
            Some(first) if now.saturating_duration_since(first) < CHALLENGE_LESS_REPEAT => {
                log::debug!("challenge-less CONNECT from {from} repeated too soon; dropped");
                false
            }
            Some(_) => true,
        }
    }

    /// The first free player ID in the client's window.
    fn free_player_id(&self, proposed: i32) -> Option<u32> {
        let start = proposed.max(MIN_PLAYER_ID) as u32;
        (start..start + PLAYER_ID_WINDOW as u32)
            .find(|id| !self.players.iter().any(|player| player.id == *id))
    }

    /// The challenge recorded for an address, or a new one.
    fn challenge_for(&mut self, from: SocketAddr, now: Instant) -> u32 {
        if let Some(entry) = self.pending.get(&from) {
            if now.saturating_duration_since(entry.created) < PENDING_LIFETIME {
                return entry.challenge;
            }
        }
        // The original draws `(rand() << 16) | rand()` from MSVC's rand; any u32 works, because
        // the client echoes whatever it is given.
        let challenge = self.rng.next_u32();
        self.pending.insert(
            from,
            PendingChallenge {
                challenge,
                created: now,
            },
        );
        challenge
    }

    /// A rejection RESULT, sent without a channel.
    fn reject(&mut self, to: SocketAddr, code: ResultCode) -> Vec<Reply> {
        let result = ConnectResult { code, player_id: 0 };
        self.control(to, flags::FLAGS_REJECTED, &result.encode())
            .into_iter()
            .collect()
    }

    /// The accepted RESULT for an existing player, on the channel it already has.
    fn accept_reply(&mut self, to: SocketAddr, id: u32, now: Instant) -> Vec<Reply> {
        let keys = &self.keys;
        let Some(player) = self.players.iter_mut().find(|player| player.addr == to) else {
            return Vec::new();
        };
        accept_reply_with(keys, &mut player.channel, to, id, now)
            .into_iter()
            .collect()
    }

    /// Pack one connection-less control message.
    fn control(&mut self, to: SocketAddr, flags: u16, payload: &[u8]) -> Option<Reply> {
        let header = Header {
            flags,
            serial: self.control_serial,
            ack: 0,
            ack_mask: 0,
            extra: 0,
        };
        self.control_serial = self.control_serial.wrapping_add(1);
        match packet::pack(&header, payload, &self.keys) {
            Ok(datagram) => Some(Reply { to, datagram }),
            Err(error) => {
                log::error!("cannot send a control message to {to}: {error}");
                None
            }
        }
    }
}

/// A [`Server`] with its UDP socket.
#[derive(Debug)]
pub struct BoundServer {
    server: Server,
    game: UdpSocket,
    buffer: Vec<u8>,
}

/// Build the accepted RESULT on `channel` and remember it for a resend.
fn accept_reply_with(
    keys: &Keys,
    channel: &mut Channel,
    to: SocketAddr,
    id: u32,
    now: Instant,
) -> Option<Reply> {
    let payload = ConnectResult {
        code: ResultCode::Accepted,
        player_id: id,
    }
    .encode();
    let outgoing = channel.send(flags::FLAGS_ACCEPTED, payload, 0, now);
    match outgoing.pack(keys) {
        Ok(datagram) => Some(Reply { to, datagram }),
        Err(error) => {
            log::error!("cannot send the connect result to {to}: {error}");
            None
        }
    }
}

impl BoundServer {
    /// Bind the game socket of `config`.
    pub fn bind(config: ServerConfig) -> Result<Self, NetError> {
        let game = UdpSocket::bind(config.bind)?;
        Ok(Self {
            server: Server::new(config),
            game,
            buffer: vec![0u8; RECV_BUFFER],
        })
    }

    /// The address the game socket bound, which is how a test learns its port.
    pub fn game_addr(&self) -> Result<SocketAddr, NetError> {
        Ok(self.game.local_addr()?)
    }

    pub fn server(&self) -> &Server {
        &self.server
    }

    /// Receive whatever is waiting, answer it, then service resends and expiry.
    ///
    /// One call waits at most `tick` for the first datagram, so a caller with a shutdown flag can
    /// use it as a loop body.
    pub fn poll(&mut self, tick: Duration) -> Result<usize, NetError> {
        self.game.set_read_timeout(Some(tick))?;
        let mut handled = 0;
        loop {
            let (len, from) = match self.game.recv_from(&mut self.buffer) {
                Ok(received) => received,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(error) => return Err(NetError::Io(error)),
            };
            handled += 1;
            let now = Instant::now();
            let datagram = self.buffer[..len].to_vec();
            let replies = self.server.on_game_datagram(&datagram, from, now);
            self.send(replies)?;
            if handled >= 32 {
                break;
            }
        }
        let now = Instant::now();
        self.server.expire(now);
        let replies = self.server.resend_due(now);
        self.send(replies)?;
        Ok(handled)
    }

    /// Serve until the process is stopped.
    pub fn run(&mut self) -> Result<(), NetError> {
        loop {
            self.poll(POLL_TICK)?;
        }
    }

    fn send(&self, replies: Vec<Reply>) -> Result<(), NetError> {
        for reply in replies {
            self.game.send_to(&reply.datagram, reply.to)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handshake::{Hello, steam_blob_encode};

    const CLIENT: SocketAddr =
        SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 40000);

    /// A test client: it builds the client side of the handshake with the public codec.
    struct TestClient {
        keys: Keys,
        serial: u32,
        steam_id: u64,
    }

    impl TestClient {
        fn new() -> Self {
            Self {
                keys: Keys::default(),
                serial: 1000,
                steam_id: 76561197960287930,
            }
        }

        fn datagram(&mut self, flags: u16, payload: Vec<u8>) -> Vec<u8> {
            let header = Header {
                flags,
                serial: self.serial,
                ack: 0,
                ack_mask: 0,
                extra: 0,
            };
            self.serial += 1;
            packet::pack(&header, &payload, &self.keys).expect("pack")
        }

        fn hello(&mut self, magic: u32) -> Vec<u8> {
            let payload = Hello {
                magic,
                actual_version: u32::from(crate::handshake::GAME_ACTUAL_VERSION),
            }
            .encode();
            self.datagram(flags::FLAGS_CONTROL, payload)
        }

        fn connect(&mut self, magic: u32, challenge: Option<u32>, name: &str) -> Vec<u8> {
            let connect = Connect {
                with_challenge: challenge.is_some(),
                magic,
                name: name.into(),
                password: String::new(),
                check_string: "check-string".into(),
                steam_blob: steam_blob_encode(self.steam_id),
                actual_version: u32::from(crate::handshake::GAME_ACTUAL_VERSION),
                required_version: u32::from(crate::handshake::GAME_ACTUAL_VERSION),
                build: crate::handshake::GAME_BUILD,
                flag_a: 1,
                flag_b: 0,
                player_id: 0x1000,
                be_state: 0,
                challenge,
            };
            self.datagram(flags::FLAGS_CONTROL, connect.encode())
        }
    }

    fn test_server() -> Server {
        Server::with_rng(ServerConfig::default(), SplitMix64::new(0xA3))
    }

    fn parse(reply: &Reply, keys: &Keys) -> (Header, Vec<u8>) {
        packet::unpack(&reply.datagram, keys).expect("a reply must be a valid datagram")
    }

    /// Drive HELLO and return the challenge the server answered with.
    fn greet(server: &mut Server, client: &mut TestClient, from: SocketAddr, now: Instant) -> u32 {
        let replies = server.on_game_datagram(&client.hello(MAGIC), from, now);
        assert_eq!(replies.len(), 1, "HELLO is answered");
        assert_eq!(replies[0].to, from);
        let (header, payload) = parse(&replies[0], &client.keys);
        assert_eq!(header.flags, flags::FLAGS_CONTROL);
        let challenge = Challenge::decode(&payload).expect("challenge");
        challenge.challenge
    }

    #[test]
    fn hello_is_answered_with_a_documented_challenge() {
        let now = Instant::now();
        let mut server = test_server();
        let mut client = TestClient::new();
        let challenge = greet(&mut server, &mut client, CLIENT, now);
        // The same address keeps its challenge while the entry lives.
        assert_eq!(greet(&mut server, &mut client, CLIENT, now), challenge);
        // After the 8-second lifetime it is replaced.
        assert_ne!(
            greet(
                &mut server,
                &mut client,
                CLIENT,
                now + PENDING_LIFETIME + Duration::from_millis(1)
            ),
            challenge
        );
    }

    #[test]
    fn hello_with_a_foreign_magic_is_ignored() {
        let mut server = test_server();
        let mut client = TestClient::new();
        assert!(
            server
                .on_game_datagram(&client.hello(0x1234_5678), CLIENT, Instant::now())
                .is_empty()
        );
    }

    #[test]
    fn a_challenged_connect_is_accepted_with_a_player_id_and_a_reliable_result() {
        let now = Instant::now();
        let mut server = test_server();
        let mut client = TestClient::new();
        let challenge = greet(&mut server, &mut client, CLIENT, now);
        let replies = server.on_game_datagram(
            &client.connect(MAGIC, Some(challenge), "J. Doe"),
            CLIENT,
            now,
        );
        assert_eq!(replies.len(), 1);
        let (header, payload) = parse(&replies[0], &client.keys);
        assert_eq!(header.flags, flags::FLAGS_ACCEPTED, "accepted is reliable");
        let result = ConnectResult::decode(&payload).expect("result");
        assert_eq!(result.code, ResultCode::Accepted);
        assert_eq!(result.player_id, 0x1000, "the first free id in the window");

        assert_eq!(server.players().len(), 1);
        let player = &server.players()[0];
        assert_eq!(player.name, "J. Doe");
        assert_eq!(player.steam_id, Some(client.steam_id));
        assert_eq!(player.addr, CLIENT);
        assert!(
            player.channel.awaiting_ack(header.serial),
            "RESULT is resent until acked"
        );
    }

    #[test]
    fn a_challenged_connect_must_echo_the_challenge() {
        let now = Instant::now();
        let mut server = test_server();
        let mut client = TestClient::new();
        // No HELLO at all: the challenge is unknown.
        assert!(
            server
                .on_game_datagram(&client.connect(MAGIC, Some(1234), "J. Doe"), CLIENT, now)
                .is_empty()
        );
        let challenge = greet(&mut server, &mut client, CLIENT, now);
        assert!(
            server
                .on_game_datagram(
                    &client.connect(MAGIC, Some(challenge ^ 0xFF), "J. Doe"),
                    CLIENT,
                    now
                )
                .is_empty()
        );
        // The right challenge still works.
        assert_eq!(
            server
                .on_game_datagram(
                    &client.connect(MAGIC, Some(challenge), "J. Doe"),
                    CLIENT,
                    now
                )
                .len(),
            1
        );
    }

    #[test]
    fn a_challenge_less_connect_is_recorded_and_then_accepted_on_a_repeat() {
        let now = Instant::now();
        let mut server = test_server();
        let mut client = TestClient::new();
        assert!(
            server
                .on_game_datagram(&client.connect(MAGIC, None, "J. Doe"), CLIENT, now)
                .is_empty(),
            "the first one is only recorded"
        );
        // A repeat in the same instant is still too soon.
        assert!(
            server
                .on_game_datagram(&client.connect(MAGIC, None, "J. Doe"), CLIENT, now)
                .is_empty()
        );
        let later = now + CHALLENGE_LESS_REPEAT;
        let replies =
            server.on_game_datagram(&client.connect(MAGIC, None, "J. Doe"), CLIENT, later);
        assert_eq!(replies.len(), 1);
        let (_, payload) = parse(&replies[0], &client.keys);
        assert_eq!(
            ConnectResult::decode(&payload).expect("result").code,
            ResultCode::Accepted
        );
    }

    #[test]
    fn a_connect_with_a_foreign_magic_or_a_low_player_id_is_dropped() {
        let now = Instant::now();
        let mut server = test_server();
        let mut client = TestClient::new();
        let challenge = greet(&mut server, &mut client, CLIENT, now);
        assert!(
            server
                .on_game_datagram(
                    &client.connect(0x1234_5678, Some(challenge), "J. Doe"),
                    CLIENT,
                    now
                )
                .is_empty()
        );
        // A player id below 0x14 never reaches the version check.
        let mut connect = Connect {
            with_challenge: true,
            magic: MAGIC,
            name: "J. Doe".into(),
            password: String::new(),
            check_string: String::new(),
            steam_blob: steam_blob_encode(1),
            actual_version: 222,
            required_version: 222,
            build: crate::handshake::GAME_BUILD,
            flag_a: 0,
            flag_b: 0,
            player_id: 0x13,
            be_state: 0,
            challenge: Some(challenge),
        };
        assert!(
            server
                .on_game_datagram(
                    &client.datagram(flags::FLAGS_CONTROL, connect.encode()),
                    CLIENT,
                    now
                )
                .is_empty()
        );
        connect.player_id = 0x14;
        assert_eq!(
            server
                .on_game_datagram(
                    &client.datagram(flags::FLAGS_CONTROL, connect.encode()),
                    CLIENT,
                    now
                )
                .len(),
            1
        );
        assert!(server.players()[0].id >= 0x14);
    }

    #[test]
    fn the_documented_rejection_codes_are_sent_without_a_channel() {
        let now = Instant::now();

        // Wrong password: result 1.
        let config = ServerConfig {
            password: "hunter2".into(),
            ..ServerConfig::default()
        };
        let mut server = Server::with_rng(config, SplitMix64::new(1));
        let mut client = TestClient::new();
        let challenge = greet(&mut server, &mut client, CLIENT, now);
        let mut connect = Connect {
            with_challenge: true,
            magic: MAGIC,
            name: "J. Doe".into(),
            password: "wrong".into(),
            check_string: String::new(),
            steam_blob: steam_blob_encode(1),
            actual_version: 222,
            required_version: 222,
            build: crate::handshake::GAME_BUILD,
            flag_a: 0,
            flag_b: 0,
            player_id: 0x100,
            be_state: 0,
            challenge: Some(challenge),
        };
        let replies = server.on_game_datagram(
            &client.datagram(flags::FLAGS_CONTROL, connect.encode()),
            CLIENT,
            now,
        );
        let (header, payload) = parse(&replies[0], &client.keys);
        assert_eq!(
            header.flags,
            flags::FLAGS_REJECTED,
            "rejected has no channel"
        );
        assert_eq!(
            ConnectResult::decode(&payload).expect("result").code,
            ResultCode::BadPassword
        );

        // An old build: result 2.
        let mut server = test_server();
        let mut client = TestClient::new();
        let challenge = greet(&mut server, &mut client, CLIENT, now);
        connect.password = String::new();
        connect.build = crate::handshake::GAME_BUILD - 1;
        connect.challenge = Some(challenge);
        let replies = server.on_game_datagram(
            &client.datagram(flags::FLAGS_CONTROL, connect.encode()),
            CLIENT,
            now,
        );
        let (_, payload) = parse(&replies[0], &client.keys);
        assert_eq!(
            ConnectResult::decode(&payload).expect("result").code,
            ResultCode::VersionMismatch
        );

        // A full server: result 4.
        let config = ServerConfig {
            max_players: 1,
            ..ServerConfig::default()
        };
        let mut server = Server::with_rng(config, SplitMix64::new(2));
        let mut client = TestClient::new();
        let challenge = greet(&mut server, &mut client, CLIENT, now);
        connect.build = crate::handshake::GAME_BUILD;
        connect.challenge = Some(challenge);
        connect.player_id = 0x100;
        assert_eq!(
            server
                .on_game_datagram(
                    &client.datagram(flags::FLAGS_CONTROL, connect.encode()),
                    CLIENT,
                    now
                )
                .len(),
            1
        );
        let other = SocketAddr::new(CLIENT.ip(), CLIENT.port() + 1);
        let mut second = TestClient::new();
        let challenge = greet(&mut server, &mut second, other, now);
        connect.challenge = Some(challenge);
        let replies = server.on_game_datagram(
            &second.datagram(flags::FLAGS_CONTROL, connect.encode()),
            other,
            now,
        );
        let (_, payload) = parse(&replies[0], &second.keys);
        assert_eq!(
            ConnectResult::decode(&payload).expect("result").code,
            ResultCode::Full
        );
    }

    #[test]
    fn the_id_window_is_twelve_wide_and_a_reconnect_reuses_the_session() {
        let now = Instant::now();
        let mut server = test_server();
        let mut client = TestClient::new();
        let challenge = greet(&mut server, &mut client, CLIENT, now);
        assert_eq!(
            server
                .on_game_datagram(
                    &client.connect(MAGIC, Some(challenge), "J. Doe"),
                    CLIENT,
                    now
                )
                .len(),
            1
        );
        assert_eq!(server.players()[0].id, 0x1000);

        // A second player whose proposal collides takes the next free id in its window.
        let other = SocketAddr::new(CLIENT.ip(), CLIENT.port() + 1);
        let mut second = TestClient::new();
        let challenge = greet(&mut server, &mut second, other, now);
        assert_eq!(
            server
                .on_game_datagram(
                    &second.connect(MAGIC, Some(challenge), "K. Roe"),
                    other,
                    now
                )
                .len(),
            1
        );
        assert_eq!(server.players()[1].id, 0x1001);

        // The same address and the same proposed id is answered again with the same id, and no
        // second player appears.
        let challenge = greet(&mut server, &mut client, CLIENT, now);
        let replies = server.on_game_datagram(
            &client.connect(MAGIC, Some(challenge), "J. Doe"),
            CLIENT,
            now,
        );
        let (_, payload) = parse(&replies[0], &client.keys);
        assert_eq!(
            ConnectResult::decode(&payload).expect("result").player_id,
            0x1000
        );
        assert_eq!(server.players().len(), 2);
    }

    #[test]
    fn the_accepted_result_is_resent_on_the_channel_until_the_client_acknowledges() {
        let now = Instant::now();
        let mut server = test_server();
        let mut client = TestClient::new();
        let challenge = greet(&mut server, &mut client, CLIENT, now);
        let replies = server.on_game_datagram(
            &client.connect(MAGIC, Some(challenge), "J. Doe"),
            CLIENT,
            now,
        );
        let (header, _) = parse(&replies[0], &client.keys);
        assert_eq!(server.resend_due(now).len(), 0, "not due yet");

        let resends = server.resend_due(now + crate::channel::RESEND_AFTER);
        assert_eq!(resends.len(), 1, "the RESULT is reliable");
        let (resent, payload) = parse(&resends[0], &client.keys);
        assert_ne!(
            resent.serial, header.serial,
            "a resend carries a fresh serial"
        );
        assert_eq!(
            ConnectResult::decode(&payload).expect("result").code,
            ResultCode::Accepted
        );

        // The client acknowledges the resend: nothing more is due.
        let ack = Header {
            flags: 0,
            serial: 100,
            ack: resent.serial,
            ack_mask: 0,
            extra: 0,
        };
        let ack_datagram = packet::pack(&ack, &[], &client.keys).expect("pack");
        assert!(
            server
                .on_game_datagram(&ack_datagram, CLIENT, now + crate::channel::RESEND_AFTER)
                .is_empty()
        );
        assert_eq!(
            server
                .resend_due(now + crate::channel::RESEND_AFTER * 8)
                .len(),
            0
        );
    }

    #[test]
    fn a_channel_datagram_from_an_unknown_address_is_dropped() {
        let now = Instant::now();
        let mut server = test_server();
        let mut client = TestClient::new();
        let datagram = client.datagram(flags::RELIABLE, b"game message".to_vec());
        assert!(server.on_game_datagram(&datagram, CLIENT, now).is_empty());
    }

    #[test]
    fn a_session_query_is_logged_but_not_answered() {
        let now = Instant::now();
        let mut server = test_server();
        let mut client = TestClient::new();
        let query = crate::handshake::SessionQuery {
            magic: MAGIC,
            steam_blob: steam_blob_encode(client.steam_id),
        };
        let datagram = client.datagram(flags::FLAGS_CONTROL, query.encode());
        assert!(
            server.on_game_datagram(&datagram, CLIENT, now).is_empty(),
            "the session-info reply layout is still a follow-up"
        );
    }

    #[test]
    fn garbage_and_broken_datagrams_are_dropped() {
        let now = Instant::now();
        let mut server = test_server();
        let mut client = TestClient::new();
        assert!(server.on_game_datagram(&[], CLIENT, now).is_empty());
        assert!(
            server
                .on_game_datagram(b"not a transport datagram", CLIENT, now)
                .is_empty()
        );
        // A valid datagram whose payload is not a known control message.
        let datagram = client.datagram(flags::FLAGS_CONTROL, vec![0xAA; 12]);
        assert!(server.on_game_datagram(&datagram, CLIENT, now).is_empty());
        // A corrupted datagram: the CRC check drops it.
        let mut broken = client.hello(MAGIC);
        let last = broken.len() - 1;
        broken[last] ^= 0x20;
        assert!(server.on_game_datagram(&broken, CLIENT, now).is_empty());
    }
}
