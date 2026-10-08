//! `a3-tools sqf ...`: run scripts headless on the SQF VM, and compile-check the install.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::{Args, Subcommand};

use a3_gamedata::{GameData, LoadOptions, VfsResolver, decode_text, script_vm};
use a3_preproc::{FsResolver, IncludeError, IncludeResolver, Preprocessor, ResolvedInclude};

#[derive(Args)]
pub struct SqfArgs {
    /// Game install folder.
    #[arg(long, env = "A3_ROOT")]
    game_dir: PathBuf,
    /// Mod folder to load after the game, in order; repeatable.
    #[arg(long = "mod", value_name = "DIR")]
    mods: Vec<PathBuf>,
    /// Also load every optional DLC and `@mod` folder found in the game folder.
    #[arg(long)]
    all_mods: bool,
    #[command(subcommand)]
    command: SqfCommand,
}

#[derive(Subcommand)]
pub enum SqfCommand {
    /// Run a script on the VM (unscheduled, then scheduled scripts it spawned) and print its
    /// result, `diag_log` output and script errors.
    Exec {
        /// A script file on disk, or a VFS path such as `\a3\functions_f\misc\fn_log.sqf`.
        script: String,
        /// Initialise the function library first (`initFunctions.sqf`, as at game start).
        #[arg(long)]
        init_functions: bool,
        /// Maximum scheduler frames to run after the script.
        #[arg(long, default_value_t = 1000)]
        frames: usize,
    },
    /// Preprocess and compile every `.sqf` in the install and print statistics.
    CompileAll {
        /// Use only a3-sqf's builtin command table (default: plus the engine's full table).
        #[arg(long)]
        builtin_table: bool,
        /// Examples printed per error category.
        #[arg(long, default_value_t = 2)]
        examples: usize,
    },
}

/// What `sqf exec` produced.
#[derive(Debug, Default)]
pub struct ExecOutput {
    /// Lines describing the function library initialisation, if requested.
    pub boot: Vec<String>,
    /// `str` of the script's result.
    pub result: Option<String>,
    /// `diag_log` lines.
    pub log: Vec<String>,
    /// Script errors.
    pub errors: Vec<String>,
}

pub fn run(args: SqfArgs) -> anyhow::Result<()> {
    match args.command {
        SqfCommand::Exec {
            ref script,
            init_functions,
            frames,
        } => {
            let data = load(&args)?;
            let out = exec(&data, script, init_functions, frames)?;
            for line in &out.boot {
                eprintln!("{line}");
            }
            for line in &out.log {
                println!("{line}");
            }
            for error in &out.errors {
                eprintln!("{error}");
            }
            if let Some(result) = &out.result {
                println!("=> {result}");
            }
            if !out.errors.is_empty() {
                bail!("{} script error(s)", out.errors.len());
            }
            Ok(())
        }
        SqfCommand::CompileAll {
            builtin_table,
            examples,
        } => {
            let vfs = a3_vfs::Vfs::new();
            let mut mods = Vec::new();
            if args.all_mods {
                mods.extend(a3_vfs::optional_mod_dirs(&args.game_dir));
            }
            mods.extend(args.mods.iter().cloned());
            let report = vfs.mount_game(&args.game_dir, &mods);
            eprintln!("mounted {} PBOs, {} files", report.pbos, vfs.len());
            let table = if builtin_table {
                a3_sqf::CommandTable::builtin()
            } else {
                a3_gamedata::engine_command_table()
            };
            let stats = a3_gamedata::compile_all(&vfs, &table);
            let total = stats.ok + stats.failed();
            println!(
                "{} / {total} .sqf files compiled ({:.2}%) in {:.2?}",
                stats.ok,
                100.0 * stats.ok as f64 / total.max(1) as f64,
                stats.elapsed
            );
            for (category, list) in stats.top() {
                println!("{:6}  {category}", list.len());
                for example in list.iter().take(examples) {
                    println!("          {example}");
                }
            }
            Ok(())
        }
    }
}

fn load(args: &SqfArgs) -> anyhow::Result<GameData> {
    let options = LoadOptions::new(&args.game_dir)
        .with_mods(&args.mods)
        .with_optional_mods(args.all_mods);
    Ok(GameData::load(&options)?)
}

/// Includes of a script from disk: next to the script first, then the game's VFS.
struct DiskThenVfs<'a> {
    disk: FsResolver,
    vfs: VfsResolver<'a>,
}

impl IncludeResolver for DiskThenVfs<'_> {
    fn resolve(&self, current_file: &str, include: &str) -> Result<ResolvedInclude, IncludeError> {
        self.disk
            .resolve(current_file, include)
            .or_else(|_| self.vfs.resolve(current_file, include))
    }

    fn exists(&self, current_file: &str, include: &str) -> bool {
        self.disk.exists(current_file, include) || self.vfs.exists(current_file, include)
    }
}

/// Runs `script` (a disk file or a VFS path) on a VM over `data`.
pub fn exec(
    data: &GameData,
    script: &str,
    init_functions: bool,
    frames: usize,
) -> anyhow::Result<ExecOutput> {
    let mut out = ExecOutput::default();
    let mut vm = script_vm(data);
    if init_functions {
        let report = a3_gamedata::init_functions(&mut vm);
        out.boot.push(format!(
            "{}: {} of {} functions compiled in {:.2?}",
            report.init_script, report.compiled, report.declared, report.elapsed
        ));
        out.errors.extend(report.errors);
        vm.host.errors.clear();
    }
    let disk = Path::new(script);
    let (name, text) = if disk.is_file() {
        let bytes = std::fs::read(disk).with_context(|| format!("reading {script}"))?;
        let file_name = disk
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let resolver = DiskThenVfs {
            disk: FsResolver::new(disk.parent().unwrap_or(Path::new("."))),
            vfs: VfsResolver::new(&data.vfs),
        };
        let name = format!("\\{file_name}");
        let output = Preprocessor::new(&resolver)
            .preprocess_str(&name, &decode_text(&bytes))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        (name, output.with_line_directives())
    } else {
        let text = a3_sqf::Host::preprocess_file(&mut vm.host, script, true)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        (script.to_owned(), text)
    };
    match vm.compile_file(&name, &text) {
        Ok(code) => {
            if let Ok(value) = vm.call(&code, None) {
                out.result =
                    Some(value.to_sqf_string_with(&|h| a3_sqf::Host::format_handle(&vm.host, h)));
            }
            vm.run_until_idle(frames, |_| {});
        }
        Err(e) => out
            .errors
            .push(e.render(&a3_sqf::SourceFile::new(name.as_str(), text.as_str()))),
    }
    out.log.append(&mut vm.host.log);
    out.errors.append(&mut vm.host.errors);
    Ok(out)
}
