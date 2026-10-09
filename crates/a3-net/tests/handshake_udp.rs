//! The connect handshake over real loopback UDP.
//!
//! `src/server.rs` tests the state machine byte by byte with no socket; this proves the layer
//! above it: that [`BoundServer`] binds, addresses its answers and drives a handshake with a
//! client built only from the documented messages. It is the evidence that an official client's
//! first two datagrams are answered the way `docs/re/net-handshake.md` describes.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use a3_net::handshake::{
    Challenge, Connect, ConnectResult, GAME_ACTUAL_VERSION, GAME_BUILD, Hello, ResultCode,
    steam_blob_encode,
};
use a3_net::server::{BoundServer, ServerConfig};
use a3_net::transport::{Header, Keys, MAGIC, flags, packet};

const STEAM_ID: u64 = 76561197960287930;

/// The client half of the handshake, as the original sends it.
struct Client {
    socket: UdpSocket,
    server: SocketAddr,
    keys: Keys,
    serial: u32,
}

impl Client {
    fn connect_to(server: SocketAddr) -> Self {
        let socket = UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .expect("client socket");
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("read timeout");
        Self {
            socket,
            server,
            keys: Keys::default(),
            serial: 1000,
        }
    }

    /// Send one datagram with the given flags and payload.
    fn send(&mut self, flags: u16, payload: Vec<u8>) -> Header {
        let header = Header {
            flags,
            serial: self.serial,
            ack: 0,
            ack_mask: 0,
            extra: 0,
        };
        self.serial += 1;
        let datagram = packet::pack(&header, &payload, &self.keys).expect("pack");
        self.socket
            .send_to(&datagram, self.server)
            .expect("send to the server");
        header
    }

    /// Acknowledge the server's datagram with this serial, as the client's channel does on every
    /// datagram it sends. `ack` alone names it; the mask is only needed for older serials.
    fn acknowledge(&mut self, serial: u32) {
        let header = Header {
            flags: 0,
            serial: self.serial,
            ack: serial,
            ack_mask: 0,
            extra: 0,
        };
        self.serial += 1;
        let datagram = packet::pack(&header, &[], &self.keys).expect("pack");
        self.socket
            .send_to(&datagram, self.server)
            .expect("send the acknowledgement");
    }

    fn hello(&mut self) -> Header {
        let payload = Hello {
            magic: MAGIC,
            actual_version: u32::from(GAME_ACTUAL_VERSION),
        }
        .encode();
        self.send(flags::FLAGS_CONTROL, payload)
    }

    fn connect(
        &mut self,
        challenge: Option<u32>,
        name: &str,
        password: &str,
        build: u32,
    ) -> Header {
        let payload = Connect {
            with_challenge: challenge.is_some(),
            magic: MAGIC,
            name: name.into(),
            password: password.into(),
            check_string: "check-string".into(),
            steam_blob: steam_blob_encode(STEAM_ID),
            actual_version: u32::from(GAME_ACTUAL_VERSION),
            required_version: u32::from(GAME_ACTUAL_VERSION),
            build,
            flag_a: 1,
            flag_b: 0,
            player_id: 0x1000,
            be_state: 0,
            challenge,
        }
        .encode();
        self.send(flags::FLAGS_CONTROL, payload)
    }

    /// Receive one datagram, or `None` when nothing arrives within the read timeout.
    fn try_recv(&self) -> Option<(Header, Vec<u8>)> {
        let mut buffer = [0u8; 0x800];
        let (len, _) = self.socket.recv_from(&mut buffer).ok()?;
        Some(packet::unpack(&buffer[..len], &self.keys).expect("a datagram from the server"))
    }

    fn recv(&self) -> (Header, Vec<u8>) {
        self.try_recv().expect("the server must answer")
    }
}

/// Start a server on an ephemeral port and return its address plus a stop flag.
fn start_server(
    config: ServerConfig,
) -> (SocketAddr, Arc<AtomicBool>, thread::JoinHandle<BoundServer>) {
    let mut server = BoundServer::bind(config).expect("bind the game port");
    let addr = server.game_addr().expect("local address");
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            server.poll(Duration::from_millis(5)).expect("poll");
        }
        server
    });
    (addr, stop, handle)
}

fn test_config() -> ServerConfig {
    ServerConfig {
        bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        hostname: "a3-rust test server".into(),
        ..ServerConfig::default()
    }
}

#[test]
fn a_client_completes_the_handshake_over_loopback_udp() {
    let (addr, stop, handle) = start_server(test_config());
    let mut client = Client::connect_to(addr);

    // HELLO is answered with a challenge on a connection-less control datagram.
    client.hello();
    let (header, payload) = client.recv();
    assert_eq!(header.flags, flags::FLAGS_CONTROL);
    assert_eq!(header.flags, 0x0801, "the flags from the sequence diagram");
    let challenge = Challenge::decode(&payload).expect("CHALLENGE").challenge;

    // CONNECT with the challenge is answered with an accepted RESULT on a reliable channel.
    client.connect(Some(challenge), "J. Doe", "", GAME_BUILD);
    let (header, payload) = client.recv();
    assert_eq!(header.flags, flags::FLAGS_ACCEPTED, "accepted is 0x8001");
    let result = ConnectResult::decode(&payload).expect("RESULT");
    assert_eq!(result.code, ResultCode::Accepted);
    assert!(result.player_id >= 0x14, "ids start at 0x14");

    // Acknowledge the RESULT the way the real client's channel does; the server must then stop
    // resending it, which `try_recv` observes after its read timeout (longer than the server's
    // 250 ms resend delay).
    client.acknowledge(header.serial);
    assert!(
        client.try_recv().is_none(),
        "the acknowledged RESULT must not be resent"
    );

    stop.store(true, Ordering::Relaxed);
    let server = handle.join().expect("server thread");
    let players = server.server().players();
    assert_eq!(players.len(), 1);
    assert_eq!(players[0].name, "J. Doe");
    assert_eq!(players[0].id, result.player_id);
    assert_eq!(players[0].steam_id, Some(STEAM_ID));
    assert!(
        !players[0].channel.awaiting_ack(header.serial),
        "the client's acknowledgement cleared the resend"
    );
}

#[test]
fn a_wrong_password_is_rejected_over_loopback_udp() {
    let (addr, stop, handle) = start_server(ServerConfig {
        password: "hunter2".into(),
        ..test_config()
    });
    let mut client = Client::connect_to(addr);

    client.hello();
    let (_, payload) = client.recv();
    let challenge = Challenge::decode(&payload).expect("CHALLENGE").challenge;

    client.connect(Some(challenge), "J. Doe", "wrong", GAME_BUILD);
    let (header, payload) = client.recv();
    assert_eq!(header.flags, flags::FLAGS_REJECTED, "rejected is 0x1001");
    let result = ConnectResult::decode(&payload).expect("RESULT");
    assert_eq!(result.code, ResultCode::BadPassword);
    assert_eq!(result.player_id, 0);

    stop.store(true, Ordering::Relaxed);
    let server = handle.join().expect("server thread");
    assert!(server.server().players().is_empty());
}

#[test]
fn two_clients_get_distinct_player_ids_over_loopback_udp() {
    let (addr, stop, handle) = start_server(test_config());
    let mut first = Client::connect_to(addr);
    let mut second = Client::connect_to(addr);
    let mut ids = Vec::new();
    for client in [&mut first, &mut second] {
        client.hello();
        let (_, payload) = client.recv();
        let challenge = Challenge::decode(&payload).expect("CHALLENGE").challenge;
        client.connect(Some(challenge), "player", "", GAME_BUILD);
        let (_, payload) = client.recv();
        let result = ConnectResult::decode(&payload).expect("RESULT");
        assert_eq!(result.code, ResultCode::Accepted);
        ids.push(result.player_id);
    }
    assert_ne!(ids[0], ids[1], "two connections get two player ids");

    stop.store(true, Ordering::Relaxed);
    let server = handle.join().expect("server thread");
    assert_eq!(server.server().players().len(), 2);
}
