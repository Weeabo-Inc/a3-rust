//! Steam A2S queries over loopback UDP against a running server (`docs/re/net-a2s.md`).
//!
//! The codec is unit tested in `src/a2s`; this proves the query port itself: that [`BoundServer`]
//! binds it, that a query tool gets the `S2C_CHALLENGE` dance it expects, and that the three
//! answers describe the players who joined over the game port.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use a3_net::a2s::{
    CHALLENGE_UNSET, RESPONSE_CHALLENGE, RESPONSE_INFO, RESPONSE_PLAYER, RESPONSE_RULES,
};
use a3_net::handshake::{
    Challenge, Connect, GAME_ACTUAL_VERSION, GAME_BUILD, Hello, steam_blob_encode,
};
use a3_net::server::{BoundServer, ServerConfig};
use a3_net::transport::{Header, Keys, MAGIC, flags, packet};

const HEADER: [u8; 4] = [0xFF; 4];
const SOURCE_ENGINE_QUERY: &[u8] = b"Source Engine Query\0";
/// Arma 3's Steam AppID, which is what the answer's 64-bit game id field carries.
const APP_ID: u64 = 107_410;
/// The low half of it, which is all the 16-bit `AppID` field can hold.
const APP_ID_SHORT: u16 = APP_ID as u16;

fn localhost(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

/// A server on ephemeral game and query ports, served by a background thread.
struct TestServer {
    game: SocketAddr,
    query: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<BoundServer>>,
}

impl TestServer {
    fn start(config: ServerConfig) -> Self {
        let mut server = BoundServer::bind(config).expect("bind");
        let game = server.game_addr().expect("game address");
        let query = server.query_addr().expect("query address");
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                server.poll(Duration::from_millis(5)).expect("poll");
            }
            server
        });
        Self {
            game,
            query,
            stop,
            handle: Some(handle),
        }
    }

    fn config() -> ServerConfig {
        ServerConfig {
            bind: localhost(0),
            query_bind: Some(localhost(0)),
            hostname: "a3-rust a2s test".into(),
            world: "stratis".into(),
            ..ServerConfig::default()
        }
    }

    /// Join one player over the game port, so the query answers have something to report.
    fn join(&self, name: &str) {
        let socket = UdpSocket::bind(localhost(0)).expect("client socket");
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        let keys = Keys::default();
        let mut serial = 1000;
        let mut send = |payload: Vec<u8>| {
            let header = Header {
                flags: flags::FLAGS_CONTROL,
                serial,
                ack: 0,
                ack_mask: 0,
                extra: 0,
            };
            serial += 1;
            let datagram = packet::pack(&header, &payload, &keys).expect("pack");
            socket.send_to(&datagram, self.game).expect("send");
        };
        let receive = || {
            let mut buffer = [0u8; 0x800];
            let (len, _) = socket.recv_from(&mut buffer).expect("the server answers");
            packet::unpack(&buffer[..len], &keys).expect("a transport datagram")
        };

        send(
            Hello {
                magic: MAGIC,
                actual_version: u32::from(GAME_ACTUAL_VERSION),
            }
            .encode(),
        );
        let (_, payload) = receive();
        let challenge = Challenge::decode(&payload).expect("CHALLENGE").challenge;
        send(
            Connect {
                with_challenge: true,
                magic: MAGIC,
                name: name.into(),
                password: String::new(),
                check_string: String::new(),
                steam_blob: steam_blob_encode(76561197960287930),
                actual_version: u32::from(GAME_ACTUAL_VERSION),
                required_version: u32::from(GAME_ACTUAL_VERSION),
                build: GAME_BUILD,
                flag_a: 1,
                flag_b: 0,
                player_id: 0x1000,
                be_state: 0,
                challenge: Some(challenge),
            }
            .encode(),
        );
        let (header, payload) = receive();
        assert_eq!(
            a3_net::handshake::ConnectResult::decode(&payload)
                .expect("RESULT")
                .player_id,
            a3_net::handshake::ResultCode::Accepted as u32 + 0x1000,
            "the player is in"
        );
        // Acknowledge the RESULT so the server stops resending it.
        let ack = Header {
            flags: 0,
            serial,
            ack: header.serial,
            ack_mask: 0,
            extra: 0,
        };
        socket
            .send_to(&packet::pack(&ack, &[], &keys).expect("pack"), self.game)
            .expect("ack");
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// A query client: one datagram, one answer, following the challenge dance.
struct QueryClient {
    socket: UdpSocket,
    server: SocketAddr,
}

impl QueryClient {
    fn new(server: SocketAddr) -> Self {
        let socket = UdpSocket::bind(localhost(0)).expect("query socket");
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        Self { socket, server }
    }

    fn exchange(&self, request: &[u8]) -> Vec<u8> {
        self.socket.send_to(request, self.server).expect("send");
        let mut buffer = [0u8; 0x1000];
        let (len, _) = self.socket.recv_from(&mut buffer).expect("answer");
        buffer[..len].to_vec()
    }

    /// Ask with no challenge, then ask again with the one the server handed out.
    fn ask(&self, build: impl Fn(Option<u32>) -> Vec<u8>) -> Vec<u8> {
        let challenge = self.exchange(&build(None));
        assert_eq!(&challenge[..4], &HEADER);
        assert_eq!(
            challenge[4], RESPONSE_CHALLENGE,
            "a query without a challenge is answered with one"
        );
        let challenge =
            u32::from_le_bytes([challenge[5], challenge[6], challenge[7], challenge[8]]);
        let answer = self.exchange(&build(Some(challenge)));
        assert_eq!(&answer[..4], &HEADER);
        answer
    }
}

fn info_request(challenge: Option<u32>) -> Vec<u8> {
    let mut out = HEADER.to_vec();
    out.push(b'T');
    out.extend_from_slice(SOURCE_ENGINE_QUERY);
    if let Some(challenge) = challenge {
        out.extend_from_slice(&challenge.to_le_bytes());
    }
    out
}

fn player_request(challenge: Option<u32>) -> Vec<u8> {
    let mut out = HEADER.to_vec();
    out.push(b'U');
    out.extend_from_slice(&challenge.unwrap_or(CHALLENGE_UNSET).to_le_bytes());
    out
}

fn rules_request(challenge: Option<u32>) -> Vec<u8> {
    let mut out = HEADER.to_vec();
    out.push(b'V');
    out.extend_from_slice(&challenge.unwrap_or(CHALLENGE_UNSET).to_le_bytes());
    out
}

/// Read a NUL-terminated string out of an answer.
fn take_cstring(data: &[u8], pos: &mut usize) -> String {
    let end = data[*pos..]
        .iter()
        .position(|b| *b == 0)
        .expect("terminator")
        + *pos;
    let text = String::from_utf8_lossy(&data[*pos..end]).into_owned();
    *pos = end + 1;
    text
}

#[test]
fn a_query_tool_gets_the_challenge_dance_and_the_three_answers() {
    let server = TestServer::start(TestServer::config());
    server.join("J. Doe");
    let client = QueryClient::new(server.query);

    // A2S_INFO: the documented fields, with the game port and the full AppID in the EDF fields.
    let info = client.ask(info_request);
    assert_eq!(info[4], RESPONSE_INFO);
    let mut pos = 6; // header, kind, protocol
    assert_eq!(info[5], 17, "the Source protocol version");
    assert_eq!(take_cstring(&info, &mut pos), "a3-rust a2s test");
    assert_eq!(take_cstring(&info, &mut pos), "stratis");
    assert_eq!(take_cstring(&info, &mut pos), "Arma3");
    assert_eq!(
        take_cstring(&info, &mut pos),
        "Waiting",
        "no mission loaded"
    );
    let app_id = u16::from_le_bytes([info[pos], info[pos + 1]]);
    pos += 2;
    assert_eq!(app_id, APP_ID_SHORT);
    assert_eq!(info[pos], 1, "one player joined");
    pos += 1;
    assert_eq!(info[pos], 64, "max players");
    pos += 1;
    assert_eq!(info[pos], 0, "no bots");
    pos += 1;
    assert_eq!(info[pos], b'd', "dedicated");
    pos += 1;
    assert_eq!(info[pos], b'w', "windows");
    pos += 1;
    assert_eq!(info[pos], 0, "not password protected");
    pos += 1;
    assert_eq!(info[pos], 0, "no BattlEye");
    pos += 1;
    assert_eq!(
        take_cstring(&info, &mut pos),
        "2.22.154103",
        "the A2S version"
    );
    let edf = info[pos];
    pos += 1;
    assert_eq!(edf & 0x80, 0x80, "the port follows");
    assert_eq!(edf & 0x20, 0x20, "the keywords follow");
    assert_eq!(edf & 0x01, 0x01, "the game id follows");
    let port = u16::from_le_bytes([info[pos], info[pos + 1]]);
    pos += 2;
    assert_eq!(
        port,
        server.game.port(),
        "the game port, not the query port"
    );
    let keywords = take_cstring(&info, &mut pos);
    assert!(
        keywords.contains("r222"),
        "the tags carry the version: {keywords}"
    );
    let game_id = u64::from_le_bytes(info[pos..pos + 8].try_into().expect("8 bytes"));
    pos += 8;
    assert_eq!(game_id, APP_ID);
    assert_eq!(pos, info.len(), "nothing is left over");

    // A2S_PLAYER lists the player who joined.
    let players = client.ask(player_request);
    assert_eq!(players[4], RESPONSE_PLAYER);
    assert_eq!(players[5], 1, "one player");
    let mut pos = 6;
    assert_eq!(players[pos], 1, "index 1");
    pos += 1;
    assert_eq!(take_cstring(&players, &mut pos), "J. Doe");
    let score = i32::from_le_bytes(players[pos..pos + 4].try_into().expect("score"));
    pos += 4;
    let duration = f32::from_le_bytes(players[pos..pos + 4].try_into().expect("duration"));
    pos += 4;
    assert_eq!(
        score, 0,
        "the score source is a documented follow-up; we report 0"
    );
    assert!(duration >= 0.0, "a duration since the join");
    assert_eq!(pos, players.len());

    // A2S_RULES carries the binary block as escaped 127-byte chunks under two-byte keys.
    let rules = client.ask(rules_request);
    assert_eq!(rules[4], RESPONSE_RULES);
    assert_eq!(u16::from_le_bytes([rules[5], rules[6]]), 1, "one chunk");
    let mut pos = 7;
    let key = take_cstring(&rules, &mut pos);
    assert_eq!(key.as_bytes(), &[1, 1], "chunk 1 of 1");
    let value = take_cstring(&rules, &mut pos);
    assert_eq!(pos, rules.len());
    // Unescape and check the documented empty block: version 3, no mods, no signatures.
    let mut block = Vec::new();
    let mut index = 0;
    while index < value.len() {
        let byte = value.as_bytes()[index];
        index += 1;
        if byte != 0x01 {
            block.push(byte);
            continue;
        }
        let code = value.as_bytes()[index];
        index += 1;
        block.push(match code {
            0x01 => 0x01,
            0x02 => 0x00,
            0x03 => 0xFF,
            other => panic!("unknown escape {other:#04x}"),
        });
    }
    assert_eq!(block, vec![3, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn a_stray_datagram_on_the_query_port_is_ignored() {
    let server = TestServer::start(TestServer::config());
    let client = QueryClient::new(server.query);
    client
        .socket
        .send_to(b"not a query", server.query)
        .expect("send");
    client
        .socket
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("read timeout");
    let mut buffer = [0u8; 0x100];
    assert!(
        client.socket.recv_from(&mut buffer).is_err(),
        "nothing may be answered"
    );
}
