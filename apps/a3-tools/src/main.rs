//! Command-line tools for Arma 3 data formats (PBO, config, PAA, ...).

mod config_cmd;
mod font_cmd;
mod input;
mod mission_cmd;
mod p3d_cmd;
mod p3d_export;
mod paa_cmd;
mod pbo_cmd;
mod rtm_cmd;
mod sign_cmd;
mod sound_cmd;
mod sqf_cmd;
mod stringtable_cmd;
mod vfs_cmd;
mod wrp_cmd;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "a3-tools", version, about = "Tools for Arma 3 data formats")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the tool version and the targeted game version.
    Version,
    /// Inspect, unpack and build PBO archives.
    #[command(subcommand)]
    Pbo(PboCommand),
    /// Browse the virtual file system of a game install.
    Vfs(VfsArgs),
    /// Rapify and derap config files (config.cpp <-> config.bin).
    #[command(subcommand)]
    Config(config_cmd::ConfigCommand),
    /// Inspect P3D models (MLOD and ODOL).
    P3d(p3d_cmd::P3dArgs),
    /// Inspect binarized terrains (WRP): summary, heightmap, objects, overview map.
    Wrp(wrp_cmd::WrpArgs),
    /// Inspect and convert PAA/PAC textures and texHeaders.bin.
    Paa(paa_cmd::PaaArgs),
    /// Inspect and convert sound files (WSS, Ogg Vorbis, WAV).
    #[command(subcommand)]
    Sound(sound_cmd::SoundCommand),
    /// Inspect bikeys/bisigns, verify and sign PBOs.
    #[command(subcommand)]
    Sign(sign_cmd::SignCommand),
    /// Inspect RTM animations.
    #[command(subcommand)]
    Rtm(rtm_cmd::RtmCommand),
    /// Inspect FXY bitmap fonts.
    #[command(subcommand)]
    Font(font_cmd::FontCommand),
    /// Read stringtables and look up localized text.
    #[command(subcommand)]
    Stringtable(stringtable_cmd::StringtableCommand),
    /// Run SQF scripts headless and compile-check the install.
    Sqf(sqf_cmd::SqfArgs),
    /// Load a mission (mission.sqm, description.ext) and print what it places; `--run` spawns it
    /// into a World and runs its scripts.
    Mission(mission_cmd::MissionArgs),
}

#[derive(Subcommand)]
enum PboCommand {
    /// Print the properties and entries of a PBO.
    List {
        /// The PBO file.
        file: PathBuf,
        /// Also check the SHA-1 trailer (reads the whole archive).
        #[arg(long)]
        verify: bool,
    },
    /// Extract every entry of a PBO into a folder (prefix written to `$PBOPREFIX$`).
    Unpack {
        /// The PBO file.
        file: PathBuf,
        /// Output folder.
        outdir: PathBuf,
    },
    /// Build a PBO from every file in a folder.
    Pack {
        /// Source folder.
        dir: PathBuf,
        /// Output PBO file.
        file: PathBuf,
        /// The `prefix` property (default: content of `<dir>/$PBOPREFIX$`, if present).
        #[arg(long)]
        prefix: Option<String>,
        /// Extra header property as `key=value`; repeatable.
        #[arg(long = "property", value_name = "KEY=VALUE")]
        properties: Vec<String>,
    },
}

#[derive(Args)]
struct VfsArgs {
    /// Game install folder.
    #[arg(long, env = "A3_ROOT")]
    game_dir: PathBuf,
    /// Mod folder to mount after the game, in order; repeatable.
    #[arg(long = "mod", value_name = "DIR")]
    mods: Vec<PathBuf>,
    /// Also mount every optional DLC and `@mod` folder found in the game folder.
    #[arg(long)]
    all_mods: bool,
    #[command(subcommand)]
    command: VfsCommand,
}

#[derive(Subcommand)]
enum VfsCommand {
    /// List a VFS directory (default: the root).
    Ls {
        #[arg(default_value = "")]
        dir: String,
    },
    /// Write a file's content to standard output.
    Cat { path: String },
    /// Print the size and source archive of a file.
    Stat { path: String },
    /// Print every path matching a pattern (`*`, `?` within a component; `**` across).
    Find { pattern: String },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Version => {
            println!(
                "a3-tools {} (targets Arma 3 {})",
                env!("CARGO_PKG_VERSION"),
                a3_core::GAME_VERSION
            );
        }
        Command::Pbo(PboCommand::List { file, verify }) => pbo_cmd::list(&file, verify)?,
        Command::Pbo(PboCommand::Unpack { file, outdir }) => {
            for name in pbo_cmd::unpack(&file, &outdir)? {
                eprintln!("skipped unsafe entry name {name:?}");
            }
        }
        Command::Pbo(PboCommand::Pack {
            dir,
            file,
            prefix,
            properties,
        }) => pbo_cmd::pack(&dir, &file, prefix.as_deref(), &properties)?,
        Command::Paa(args) => paa_cmd::run(args)?,
        Command::Vfs(args) => {
            let vfs = vfs_cmd::mount(&args.game_dir, &args.mods, args.all_mods)?;
            match args.command {
                VfsCommand::Ls { dir } => vfs_cmd::ls(&vfs, &dir)?,
                VfsCommand::Cat { path } => vfs_cmd::cat(&vfs, &path)?,
                VfsCommand::Stat { path } => vfs_cmd::stat(&vfs, &path)?,
                VfsCommand::Find { pattern } => vfs_cmd::find(&vfs, &pattern)?,
            }
        }
        Command::Config(cmd) => config_cmd::run(cmd)?,
        Command::P3d(args) => p3d_cmd::run(args)?,
        Command::Wrp(args) => wrp_cmd::run(args)?,
        Command::Sound(cmd) => sound_cmd::run(cmd)?,
        Command::Sign(cmd) => sign_cmd::run(cmd)?,
        Command::Rtm(cmd) => rtm_cmd::run(cmd)?,
        Command::Font(cmd) => font_cmd::run(cmd)?,
        Command::Stringtable(cmd) => stringtable_cmd::run(cmd)?,
        Command::Sqf(args) => sqf_cmd::run(args)?,
        Command::Mission(args) => mission_cmd::run(args)?,
    }
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
}
