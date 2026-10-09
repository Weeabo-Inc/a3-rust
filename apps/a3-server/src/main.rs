//! The a3-rust dedicated server.
//!
//! The first slice of Phase 6 (`docs/ROADMAP.md`, ADR 0004): it binds the game port, speaks the
//! transport in `crates/a3-net`, completes the connect handshake with a joining client and logs
//! what it receives. The game message layer behind the handshake is the next slice, and there is
//! no Steam query socket yet, so this binary does not answer A2S.
//!
//! Configuration comes from the command line, with `server.cfg` in `--game-dir`/`A3_ROOT` as the
//! fallback for the four values the original takes from there: `hostname`, `password`,
//! `maxPlayers` and `requiredBuild`.

use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use a3_config::{ConfigClass, EntryKind, Value};
use a3_net::handshake::{GAME_ACTUAL_VERSION, GAME_BUILD, Versions};
use a3_net::server::{BoundServer, ServerConfig};
use a3_net::transport::MAGIC;
use anyhow::Context;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "a3-server",
    about = "The a3-rust dedicated server for Arma 3 2.22.0.154103 clients",
    version
)]
struct Cli {
    /// Address the game socket binds.
    #[arg(long, default_value = "0.0.0.0")]
    bind: IpAddr,

    /// The game port (2302 in the original).
    #[arg(long, default_value_t = 2302)]
    port: u16,

    /// Server hostname, as the server browser shows it.
    #[arg(long)]
    hostname: Option<String>,

    /// Join password; without one anybody may join.
    #[arg(long)]
    password: Option<String>,

    /// Maximum number of players.
    #[arg(long)]
    max_players: Option<u8>,

    /// Oldest client build accepted (server.cfg `requiredBuild`).
    #[arg(long)]
    required_build: Option<u32>,

    /// World (terrain) name.
    #[arg(long)]
    world: Option<String>,

    /// Mission name, if one is loaded.
    #[arg(long)]
    mission: Option<String>,

    /// Game installation directory; `server.cfg` is read from it when it exists.
    #[arg(long, env = "A3_ROOT")]
    game_dir: Option<std::path::PathBuf>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cli = Cli::parse();
    let config = build_config(&cli)?;
    let mut server = BoundServer::bind(config.clone()).context("binding the game port")?;
    let addr = server.game_addr().context("reading the bound address")?;
    tracing::info!(
        "a3-rust dedicated server listening on {addr} (UDP), hostname {:?}, {} players, world {}, build {}",
        config.hostname,
        config.max_players,
        config.world,
        config.versions.required_build
    );
    if cli.game_dir.is_none() {
        tracing::info!(
            "no --game-dir/A3_ROOT given: using command-line defaults, no server.cfg read"
        );
    }
    tracing::info!("no Steam query port in this slice: A2S_INFO/PLAYER/RULES are not answered yet");
    server.run().context("serving")
}

/// The values the original reads from `server.cfg`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ServerCfg {
    hostname: Option<String>,
    password: Option<String>,
    max_players: Option<u8>,
    required_build: Option<u32>,
}

/// Read `server.cfg` from a game directory, if it is there.
fn read_server_cfg(game_dir: &Path) -> anyhow::Result<ServerCfg> {
    let path = game_dir.join("server.cfg");
    if !path.is_file() {
        tracing::info!("{} does not exist; using the defaults", path.display());
        return Ok(ServerCfg::default());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let config =
        a3_config::parse_text(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(ServerCfg {
        hostname: string_entry(&config.root, "hostname"),
        password: string_entry(&config.root, "password"),
        max_players: int_entry(&config.root, "maxPlayers").and_then(|v| u8::try_from(v).ok()),
        required_build: int_entry(&config.root, "requiredBuild")
            .and_then(|v| u32::try_from(v).ok()),
    })
}

/// A root-level string entry of a config, compared case-insensitively as the engine does.
fn string_entry(class: &ConfigClass, name: &str) -> Option<String> {
    match &class.get(name)?.kind {
        EntryKind::Value(Value::String(text)) => Some(text.clone()),
        _ => None,
    }
}

/// A root-level integer entry of a config.
fn int_entry(class: &ConfigClass, name: &str) -> Option<i32> {
    match &class.get(name)?.kind {
        EntryKind::Value(Value::Int(value)) => Some(*value),
        _ => None,
    }
}

/// The command line, with `server.cfg` filling the values it does not give.
fn build_config(cli: &Cli) -> anyhow::Result<ServerConfig> {
    let file = match &cli.game_dir {
        Some(dir) => read_server_cfg(dir)?,
        None => ServerCfg::default(),
    };
    let required_build = cli
        .required_build
        .or(file.required_build)
        .unwrap_or(GAME_BUILD);
    Ok(ServerConfig {
        bind: SocketAddr::new(cli.bind, cli.port),
        magic: MAGIC,
        hostname: cli
            .hostname
            .clone()
            .or(file.hostname)
            .unwrap_or_else(|| "a3-rust dedicated server".into()),
        password: cli.password.clone().or(file.password).unwrap_or_default(),
        max_players: cli.max_players.or(file.max_players).unwrap_or(64),
        versions: Versions {
            actual: GAME_ACTUAL_VERSION,
            required: GAME_ACTUAL_VERSION,
            required_build,
        },
        world: cli.world.clone().unwrap_or_else(|| "altis".into()),
        mission: cli.mission.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn server_cfg_fills_the_four_values_the_original_reads_from_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            dir.path().join("server.cfg"),
            r#"
            // a minimal server.cfg, the shape the game ships
            hostname = "My Arma Server";
            password = "hunter2";
            maxPlayers = 32;
            requiredBuild = 154100;
            motd[] = {"hello"};
            "#,
        )
        .expect("write server.cfg");
        let cfg = read_server_cfg(dir.path()).expect("read server.cfg");
        assert_eq!(cfg.hostname.as_deref(), Some("My Arma Server"));
        assert_eq!(cfg.password.as_deref(), Some("hunter2"));
        assert_eq!(cfg.max_players, Some(32));
        assert_eq!(cfg.required_build, Some(154100));
    }

    #[test]
    fn a_missing_server_cfg_is_not_an_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(
            read_server_cfg(dir.path()).expect("missing file"),
            ServerCfg::default()
        );
    }

    #[test]
    fn the_command_line_overrides_server_cfg_and_defaults_fill_the_rest() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            dir.path().join("server.cfg"),
            "hostname = \"From the file\";\nmaxPlayers = 32;\npassword = \"fromthefile\";\n",
        )
        .expect("write server.cfg");
        let cli = Cli::try_parse_from([
            "a3-server",
            "--port",
            "2402",
            "--hostname",
            "From the command line",
            "--game-dir",
            dir.path().to_str().expect("utf-8 path"),
        ])
        .expect("parse");
        let config = build_config(&cli).expect("build");
        assert_eq!(config.bind.port(), 2402);
        assert_eq!(config.hostname, "From the command line", "the flag wins");
        assert_eq!(
            config.password, "fromthefile",
            "the file fills what the flag omits"
        );
        assert_eq!(config.max_players, 32);
        assert_eq!(config.versions.required_build, GAME_BUILD, "the default");
        assert_eq!(config.magic, MAGIC);
        assert_eq!(config.world, "altis");
    }

    #[test]
    fn the_defaults_are_the_documented_ones() {
        let cli = Cli::try_parse_from(["a3-server"]).expect("parse");
        let config = build_config(&cli).expect("build");
        assert_eq!(
            config.bind,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 2302)
        );
        assert_eq!(config.max_players, 64);
        assert!(config.password.is_empty());
        assert_eq!(config.versions.required_build, GAME_BUILD);
        assert_eq!(config.versions.actual, GAME_ACTUAL_VERSION);
    }

    #[test]
    fn the_server_binds_an_ephemeral_port_for_a_test() {
        let config = ServerConfig {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            ..ServerConfig::default()
        };
        let server = BoundServer::bind(config).expect("bind");
        assert_ne!(server.game_addr().expect("address").port(), 0);
    }
}
