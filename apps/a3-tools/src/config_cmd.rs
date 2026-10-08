//! `a3-tools config ...`: rapify and derap config files.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Convert a rapified config (config.bin, rapified mission.sqm/rvmat) to config.cpp text.
    Derap {
        /// Rapified input file.
        input: PathBuf,
        /// Output text file; stdout when omitted.
        output: Option<PathBuf>,
    },
    /// Convert config.cpp text to the rapified binary form. The input must already be
    /// preprocessed (no #include/#define).
    Rap {
        /// Preprocessed config text.
        input: PathBuf,
        /// Rapified output file.
        output: PathBuf,
    },
}

pub fn run(cmd: ConfigCommand) -> anyhow::Result<()> {
    match cmd {
        ConfigCommand::Derap { input, output } => {
            let text = derap_file(&input)?;
            match output {
                Some(out) => {
                    std::fs::write(&out, text).with_context(|| format!("writing {}", out.display()))
                }
                None => {
                    print!("{text}");
                    Ok(())
                }
            }
        }
        ConfigCommand::Rap { input, output } => rap_file(&input, &output),
    }
}

fn derap_file(input: &Path) -> anyhow::Result<String> {
    let bytes = std::fs::read(input).with_context(|| format!("reading {}", input.display()))?;
    if !a3_config::is_rap(&bytes) {
        bail!("{} is not a rapified config", input.display());
    }
    let config =
        a3_config::read_rap(&bytes).with_context(|| format!("derapping {}", input.display()))?;
    Ok(a3_config::write_text(&config))
}

fn rap_file(input: &Path, output: &Path) -> anyhow::Result<()> {
    let text =
        std::fs::read_to_string(input).with_context(|| format!("reading {}", input.display()))?;
    let config =
        a3_config::parse_text(&text).map_err(|e| anyhow::anyhow!("{}:{e}", input.display()))?;
    std::fs::write(output, a3_config::write_rap(&config))
        .with_context(|| format!("writing {}", output.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("a3-tools-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rap_then_derap_round_trips_a_file() {
        let dir = temp_dir("config-roundtrip");
        let cpp = dir.join("config.cpp");
        let bin = dir.join("config.bin");
        let out = dir.join("derapped.cpp");
        std::fs::write(
            &cpp,
            "class CfgPatches { class X { units[] = {\"a\"}; }; };",
        )
        .unwrap();

        run(ConfigCommand::Rap {
            input: cpp.clone(),
            output: bin.clone(),
        })
        .unwrap();
        assert!(a3_config::is_rap(&std::fs::read(&bin).unwrap()));
        run(ConfigCommand::Derap {
            input: bin,
            output: Some(out.clone()),
        })
        .unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert_eq!(
            text,
            "class CfgPatches\n{\n    class X\n    {\n        units[] = {\"a\"};\n    };\n};\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rap_reports_syntax_errors_with_file_and_position() {
        let dir = temp_dir("config-error");
        let cpp = dir.join("bad.cpp");
        std::fs::write(&cpp, "class A {\n  x = 1;\n").unwrap();
        let err = run(ConfigCommand::Rap {
            input: cpp,
            output: dir.join("bad.bin"),
        })
        .unwrap_err();
        assert!(err.to_string().contains("bad.cpp:3:1:"), "{err}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
