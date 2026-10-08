//! `a3-tools stringtable ...`: read stringtables and look up localized text.

use std::fmt::Write as _;
use std::path::PathBuf;

use a3_stringtable::{Localizer, Stringtable};
use anyhow::Context;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum StringtableCommand {
    /// Print every key of one stringtable file (XML or binarized) with its text in a language.
    Dump {
        /// The stringtable file.
        file: PathBuf,
        /// Language to resolve (falls back to Original, English, then the first entry).
        #[arg(long, default_value = "English")]
        language: String,
    },
    /// Look up keys in every stringtable of the game, as the `localize` command does.
    Localize {
        /// Keys such as `STR_DISP_OK` (a leading `$` is accepted).
        #[arg(required = true)]
        keys: Vec<String>,
        /// Language to resolve.
        #[arg(long, default_value = "English")]
        language: String,
        /// Game install folder.
        #[arg(long, env = "A3_ROOT")]
        game_dir: PathBuf,
    },
}

pub fn run(cmd: StringtableCommand) -> anyhow::Result<()> {
    match cmd {
        StringtableCommand::Dump { file, language } => {
            let data =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            print!("{}", dump(&Stringtable::read(&data)?, &language));
        }
        StringtableCommand::Localize {
            keys,
            language,
            game_dir,
        } => {
            let vfs = a3_vfs::Vfs::new();
            vfs.mount_game(&game_dir, &a3_vfs::optional_mod_dirs(&game_dir));
            let (localizer, report) = Localizer::load_vfs(&vfs, &language);
            eprintln!(
                "{} stringtables, {} keys ({language})",
                report.tables.len(),
                localizer.len()
            );
            for key in keys {
                println!("{key}\t{}", localizer.localize(&key));
            }
        }
    }
    Ok(())
}

fn dump(table: &Stringtable, language: &str) -> String {
    let mut out = String::new();
    for entry in &table.entries {
        let text = entry.resolve(language).unwrap_or_default();
        let _ = writeln!(out, "{}\t{}", entry.key, text.replace('\n', "\\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_prints_one_line_per_key_with_escaped_newlines() {
        let table = Stringtable::from_xml(
            "<Project><Key ID=\"STR_a\"><English>one\ntwo</English><German>eins</German></Key></Project>",
        )
        .unwrap();
        assert_eq!(dump(&table, "English"), "STR_a\tone\\ntwo\n");
        assert_eq!(dump(&table, "German"), "STR_a\teins\n");
    }
}
