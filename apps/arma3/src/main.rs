//! The a3-rust game client.
//!
//! For now: a window with a free-fly camera over a procedural test scene or a World's terrain
//! (`--world altis`), an FPS overlay, a headless smoke-test mode and an offscreen screenshot
//! mode for checking rendering changes.

mod engine;
mod offline;
mod scene;
mod windowed;
mod world;

use std::path::PathBuf;

use a3_platform::WindowConfig;
use clap::Parser;

use crate::engine::EngineContext;

#[derive(Debug, Parser)]
#[command(name = "arma3", version, about = "a3-rust game client")]
struct Cli {
    /// Arma 3 installation directory.
    #[arg(long, env = "A3_ROOT")]
    game_dir: Option<PathBuf>,
    /// Window or render width in pixels.
    #[arg(long, default_value_t = 1280)]
    width: u32,
    /// Window or render height in pixels.
    #[arg(long, default_value_t = 720)]
    height: u32,
    /// Run in a window instead of borderless fullscreen.
    #[arg(long)]
    windowed: bool,
    /// Present without waiting for vertical sync.
    #[arg(long)]
    no_vsync: bool,
    /// Run the main loop without window or GPU for `--frames` frames, then exit.
    #[arg(long, conflicts_with = "screenshot")]
    headless: bool,
    /// Render `--frames` frames offscreen and save the last one to this PNG, then exit.
    #[arg(long, value_name = "PNG")]
    screenshot: Option<PathBuf>,
    /// Frame count for `--headless` and `--screenshot`.
    #[arg(long, default_value_t = 60)]
    frames: u64,
    /// Fly over this World's terrain (a CfgWorlds class, e.g. `altis`); needs the game dir.
    #[arg(long, value_name = "NAME")]
    world: Option<String>,
    /// Camera placement over the world: `east,north[,altitude,heading,pitch]` (metres above
    /// the terrain, degrees).
    #[arg(long, value_name = "SPEC", requires = "world")]
    camera: Option<world::CameraSpec>,
    /// Distance fog density per metre (default 8e-5, Altis' `hazeBaseBeta0`).
    #[arg(long, value_name = "DENSITY")]
    fog: Option<f32>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,wgpu_core=warn,wgpu_hal=warn,naga=warn".into()),
        )
        .init();
    let cli = Cli::parse();
    let engine = EngineContext {
        game_dir: cli.game_dir.clone(),
        world: cli.world.clone(),
        camera: cli.camera,
        fog: cli.fog,
    };
    log::info!(
        "a3-rust {} (targets Arma 3 {}), game dir: {:?}",
        env!("CARGO_PKG_VERSION"),
        a3_core::GAME_VERSION,
        engine.game_dir
    );

    if cli.headless {
        let report = offline::run_headless(cli.frames);
        println!(
            "headless: {} frames, {:.3} s simulated, {} fixed steps",
            report.frames, report.sim_time, report.fixed_steps
        );
        return Ok(());
    }
    if let Some(path) = &cli.screenshot {
        return offline::screenshot(&engine, path, cli.width, cli.height, cli.frames);
    }

    let config = WindowConfig {
        title: "a3-rust".to_owned(),
        width: cli.width,
        height: cli.height,
        windowed: cli.windowed,
        ..WindowConfig::default()
    };
    a3_platform::run(config, windowed::GameApp::new(engine, !cli.no_vsync))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn game_dir_flag_is_parsed() {
        let cli = Cli::try_parse_from(["arma3", "--game-dir", "X:/Arma 3", "--windowed"]).unwrap();
        assert_eq!(cli.game_dir, Some(PathBuf::from("X:/Arma 3")));
        assert!(cli.windowed);
    }

    #[test]
    fn world_and_camera_flags_are_parsed() {
        let cli = Cli::try_parse_from(["arma3", "--world", "altis", "--camera", "3600,13000,200"])
            .unwrap();
        assert_eq!(cli.world.as_deref(), Some("altis"));
        assert_eq!(cli.camera.map(|c| c.altitude), Some(200.0));
        assert!(Cli::try_parse_from(["arma3", "--camera", "1,2"]).is_err());
    }
}
