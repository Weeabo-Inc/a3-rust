//! The a3-rust game client.
//!
//! For now: a window with a free-fly camera over a procedural test scene or a World's terrain
//! (`--world altis`), an FPS overlay, a headless smoke-test mode and an offscreen screenshot
//! mode for checking rendering changes.

mod audio;
mod combat;
mod engine;
mod environment;
mod gear;
mod hud;
mod keys;
mod man;
mod models;
mod offline;
mod player;
mod roads;
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
    /// Keyboard preset from `CfgDefaultKeysPresets` (default: the config's `default = 1` preset).
    /// Needs the game folder.
    #[arg(long, value_name = "CLASS")]
    keys_preset: Option<String>,
    /// Player profile (`.Arma3Profile`) whose `key<Action>[]` entries override the preset.
    #[arg(long, value_name = "FILE")]
    profile: Option<PathBuf>,
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
    /// the terrain, degrees). With `--play` this is the player's spawn instead, altitude
    /// ignored.
    #[arg(long, value_name = "SPEC", requires = "world")]
    camera: Option<world::CameraSpec>,
    /// Play as a Man on the terrain: WASD walks it, the mouse looks, Numpad Enter switches
    /// first and third person, C/Z/X crouch, go prone and stand (SHIFT sprints, CTRL walks).
    #[arg(long, requires = "world")]
    play: bool,
    /// Camera `--play` starts in: `first` (`fp`) or `third` (`tp`).
    #[arg(long, value_name = "MODE", requires = "play", default_value = "first")]
    camera_mode: player::CameraMode,
    /// `--screenshot` with `--play`: hold the fire action (the left mouse button) for the
    /// captured frames, so the shot shows the rifle's tracers in flight.
    #[arg(long, requires = "play")]
    fire: bool,
    /// Objects quality (`VeryLow`, `Low`, `High`, `VeryHigh`, `Ultra`, `Extreme`): LOD
    /// coefficients and how far small objects stay visible.
    #[arg(long, default_value = "High")]
    objects_quality: a3_render_models::ObjectsQuality,
    /// Object view distance in metres.
    #[arg(long, default_value_t = 1600.0)]
    view_distance: f32,
    /// Model viewer: show one P3D from the game data (VFS path) with an orbit camera.
    #[arg(long, value_name = "VFS_PATH", conflicts_with = "world")]
    model: Option<String>,
    /// Model viewer camera heading in degrees (0 looks north; the default keeps the sun behind).
    #[arg(long, default_value_t = 325.0, allow_hyphen_values = true)]
    view_yaw: f32,
    /// Model viewer camera pitch in degrees (negative looks down).
    #[arg(long, default_value_t = -20.0, allow_hyphen_values = true)]
    view_pitch: f32,
    /// Model viewer camera distance in model radii.
    #[arg(long, default_value_t = 1.9)]
    view_zoom: f32,
    /// Model viewer: always draw this Resolution LOD (0 = most detailed).
    #[arg(long, value_name = "N")]
    lod: Option<usize>,
    /// Model viewer: pose the model as a Man in this Move (a `CfgMovesMaleSdr` class, e.g.
    /// `AmovPpneMstpSrasWrflDnon`), like `switchMove`. Loads the game config.
    #[arg(long = "move", value_name = "CLASS", requires = "model")]
    move_: Option<String>,
    /// Model viewer: the phase of `--move`'s cycle to pose, 0..1.
    #[arg(long, default_value_t = 0.0, requires = "move_")]
    move_phase: f32,
    /// Model viewer: dress the posed Man in the gear of this `CfgVehicles` class (e.g.
    /// `B_Soldier_F`): head, vest, helmet, NVG and rifle.
    #[arg(long, value_name = "CLASS", requires = "move_")]
    loadout: Option<String>,
    /// `--screenshot`: after loading, time this many frames and print the average.
    #[arg(long, default_value_t = 0)]
    bench_frames: u64,
    /// Date over the world, `year-month-day` (default: the world's `startDate`).
    #[arg(long, value_name = "DATE", value_parser = environment::parse_date, requires = "world")]
    date: Option<(i32, u32, u32)>,
    /// Local time over the world, `hh:mm` (default: the world's `startTime`).
    #[arg(long, value_name = "TIME", value_parser = environment::parse_time, requires = "world")]
    time: Option<f64>,
    /// Overcast 0..1, as `setOvercast` (default: the world's `startWeather`).
    #[arg(long, value_name = "VALUE", requires = "world")]
    overcast: Option<f32>,
    /// Waves 0..1, as `setWaves` (default: from the overcast).
    #[arg(long, value_name = "VALUE", requires = "world")]
    waves: Option<f32>,
    /// Fog as `setFog`: `value[,decay[,base]]` (default: the world's `startFog*`).
    #[arg(long, value_name = "FOG", requires = "world")]
    fog: Option<environment::FogSpec>,
    /// Fog view distance in metres: the linear fog ends there.
    #[arg(long, value_name = "METRES", requires = "world")]
    fog_distance: Option<f32>,
    /// Camera field of view as RV's `fovTop` (tangent of half the vertical angle). A
    /// `camSetFov f` camera renders at `fovTop = f * 0.75` (docs/fidelity/render-oracle.md).
    /// Default 0.75.
    #[arg(long, value_name = "TOP", requires = "world")]
    fov: Option<f32>,
    /// Leave out the debug text overlay, for frames to compare with the original game.
    #[arg(long)]
    no_overlay: bool,
    /// `--play`: run this SQF on the in-game UI's script VM when play starts (`hint`,
    /// `hintSilent`, `systemChat` show on the HUD).
    #[arg(long, value_name = "SQF", requires = "play")]
    exec: Option<String>,
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
        keys: keys::KeySources {
            game_dir: cli.game_dir.clone(),
            preset: cli.keys_preset.clone(),
            profile: cli.profile.clone(),
        },
        world: cli.world.clone(),
        camera: cli.camera,
        fov: cli.fov,
        play: cli.play,
        camera_mode: cli.camera_mode,
        fire: cli.fire,
        objects: models::ObjectOptions {
            quality: cli.objects_quality,
            view_distance: cli.view_distance,
        },
        model: cli.model.as_ref().map(|path| models::ModelSpec {
            path: path.clone(),
            yaw: cli.view_yaw,
            pitch: cli.view_pitch,
            zoom: cli.view_zoom,
            lod: cli.lod,
            pose: cli.move_.clone().map(|name| (name, cli.move_phase)),
            loadout: cli.loadout.clone(),
        }),
        environment: environment::EnvironmentSpec {
            date: cli.date,
            time: cli.time,
            overcast: cli.overcast,
            waves: cli.waves,
            fog: cli.fog,
            fog_distance: cli.fog_distance,
        },
        hide_overlay: cli.no_overlay,
        exec: cli.exec.clone(),
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
        return offline::screenshot(
            &engine,
            path,
            (cli.width, cli.height),
            cli.frames,
            cli.bench_frames,
        );
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
    fn play_and_camera_mode_flags_are_parsed() {
        let cli = Cli::try_parse_from([
            "arma3",
            "--world",
            "altis",
            "--play",
            "--camera-mode",
            "third",
            "--camera",
            "3600,13000",
        ])
        .unwrap();
        assert!(cli.play);
        assert_eq!(cli.camera_mode, crate::player::CameraMode::ThirdPerson);
        assert!(!cli.no_overlay);
        assert!(
            Cli::try_parse_from(["arma3", "--no-overlay"])
                .unwrap()
                .no_overlay
        );
        assert!(Cli::try_parse_from(["arma3", "--world", "altis"]).is_ok());
        assert_eq!(
            Cli::try_parse_from(["arma3", "--world", "altis", "--play"])
                .unwrap()
                .camera_mode,
            crate::player::CameraMode::FirstPerson,
            "the game starts in first person"
        );
        assert!(
            Cli::try_parse_from(["arma3", "--play"]).is_err(),
            "--play needs a world"
        );
        assert!(
            Cli::try_parse_from(["arma3", "--camera-mode", "third"]).is_err(),
            "--camera-mode needs --play"
        );
        assert!(Cli::try_parse_from(["arma3", "--world", "altis", "--camera-mode", "2d"]).is_err());
    }

    #[test]
    fn world_and_camera_flags_are_parsed() {
        let cli = Cli::try_parse_from(["arma3", "--world", "altis", "--camera", "3600,13000,200"])
            .unwrap();
        assert_eq!(cli.world.as_deref(), Some("altis"));
        assert_eq!(cli.camera.map(|c| c.altitude), Some(200.0));
        assert!(Cli::try_parse_from(["arma3", "--camera", "1,2"]).is_err());
    }

    #[test]
    fn oracle_flags_are_parsed() {
        let cli = Cli::try_parse_from([
            "arma3",
            "--world",
            "altis",
            "--fov",
            "0.5",
            "--screenshot",
            "out.png",
            "--no-overlay",
        ])
        .unwrap();
        assert_eq!(cli.fov, Some(0.5));
        assert!(cli.no_overlay);
        assert!(
            Cli::try_parse_from(["arma3", "--fov", "0.5"]).is_err(),
            "--fov needs a world"
        );
    }

    #[test]
    fn environment_flags_are_parsed() {
        let cli = Cli::try_parse_from([
            "arma3",
            "--world",
            "altis",
            "--date",
            "2035-06-24",
            "--time",
            "05:30",
            "--overcast",
            "0.4",
            "--fog",
            "0.2,0.05,0",
            "--fog-distance",
            "3000",
        ])
        .unwrap();
        assert_eq!(cli.date, Some((2035, 6, 24)));
        assert_eq!(cli.time, Some(5.5));
        assert_eq!(cli.overcast, Some(0.4));
        assert_eq!(cli.fog_distance, Some(3000.0));
        assert_eq!(
            cli.fog,
            Some(environment::FogSpec {
                value: 0.2,
                decay: Some(0.05),
                base: Some(0.0),
            })
        );
    }
}
