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
    /// Load the game's merged config and print one class or entry as config text.
    Dump {
        /// Path below the root, e.g. `CfgVehicles/B_Soldier_F` (`/`, `\` or `>>` separated);
        /// the whole config when omitted.
        path: Option<String>,
        /// Flatten inheritance: include inherited entries and apply `+=`.
        #[arg(long)]
        resolved: bool,
        /// Game install folder.
        #[arg(long, env = "A3_ROOT")]
        game_dir: PathBuf,
        /// Mod folder to load after the game, in order; repeatable.
        #[arg(long = "mod", value_name = "DIR")]
        mods: Vec<PathBuf>,
        /// Also load every optional DLC and `@mod` folder found in the game folder.
        #[arg(long)]
        all_mods: bool,
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
        ConfigCommand::Dump {
            path,
            resolved,
            game_dir,
            mods,
            all_mods,
        } => {
            let options = a3_gamedata::LoadOptions::new(game_dir)
                .with_mods(mods)
                .with_optional_mods(all_mods);
            let data = a3_gamedata::GameData::load(&options)?;
            let r = &data.report;
            eprintln!(
                "loaded {} PBOs, {} addon configs in {:.2?} ({} config errors, {} missing \
                 requirements)",
                r.mount.pbos,
                data.addons.len(),
                r.timings.total,
                r.config_errors.len(),
                r.missing_requirements.len()
            );
            print!(
                "{}",
                dump(&data.config, path.as_deref().unwrap_or(""), resolved)?
            );
            Ok(())
        }
    }
}

/// The entry at `path` (components separated by `/`, `\` or `>>`) as config text, preceded by
/// comments giving its full path and base chain.
pub fn dump(config: &a3_config::ConfigTree, path: &str, resolved: bool) -> anyhow::Result<String> {
    let mut entry = config.root();
    let normalized = path.replace(">>", "/").replace('\\', "/");
    for part in normalized.split('/').map(|p| p.trim().trim_matches('"')) {
        if part.is_empty() {
            continue;
        }
        let next = entry.get(part);
        if next.is_null() {
            bail!("no entry {part:?} in {}", entry.path_string());
        }
        entry = next;
    }
    let mode = if resolved {
        a3_config::ExportMode::Resolved
    } else {
        a3_config::ExportMode::Merged
    };
    let mut out = format!("// {}\n", entry.path_string());
    let bases: Vec<&str> = entry.bases().iter().map(|b| b.name()).collect();
    if !bases.is_empty() {
        out.push_str(&format!("// inherits: {}\n", bases.join(" -> ")));
    }
    let exported = entry.export(mode).expect("entry is not null");
    out.push_str(&a3_config::write_text(&exported));
    Ok(out)
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

    fn tree(src: &str) -> a3_config::ConfigTree {
        a3_config::ConfigTree::from_config(&a3_config::parse_text(src).unwrap())
    }

    #[test]
    fn dump_prints_the_merged_class_or_its_resolved_form() {
        let t = tree("class Cfg { class A { x = 1; }; class B: A { y = 2; }; };");
        assert_eq!(
            dump(&t, "Cfg/B", false).unwrap(),
            "// bin\\config.bin/Cfg/B\n// inherits: A\nclass B: A\n{\n    y = 2;\n};\n"
        );
        assert_eq!(
            dump(&t, r#"cfg >> "b""#, true).unwrap(),
            "// bin\\config.bin/Cfg/B\n// inherits: A\nclass B\n{\n    y = 2;\n    x = 1;\n};\n"
        );
        // An inherited entry prints at the class that declares it, as the engine's configs do
        // (oracle probe `cfg.configfile_str_path`).
        assert_eq!(
            dump(&t, r"Cfg\B\x", true).unwrap(),
            "// bin\\config.bin/Cfg/A/x\nx = 1;\n"
        );
        let err = dump(&t, "Cfg/Nope/x", false).unwrap_err();
        assert!(err.to_string().contains("\"Nope\""), "{err}");
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
