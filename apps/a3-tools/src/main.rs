//! Command-line tools for Arma 3 data formats (PBO, config, PAA, ...).

use clap::{Parser, Subcommand};

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
