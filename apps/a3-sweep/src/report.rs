//! What one scenario run produced ([`ScenarioResult`]) and what a whole sweep produced
//! ([`Sweep`]), as written to `.work/sweep/<timestamp>.json`; and the grouping of engine-style
//! script error reports into [`Signature`]s.

use std::collections::BTreeMap;

use a3_sqf::Form;
use serde::{Deserialize, Serialize};

use crate::inventory::Scenario;
use crate::stubs::StubIndex;

/// How a scenario's run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Loaded, started and simulated with no script error, missing command, unspawned unit or
    /// failed sanity check.
    Pass,
    /// Ran to the end, with problems.
    Errors,
    /// `mission.sqm`, `description.ext` or the terrain did not load.
    LoadFailed,
    /// The engine panicked (caught; the worker went on).
    Panic,
    /// The worker process died (stack overflow, abort, out of memory).
    Crash,
    /// The worker did not answer within the hard time limit and was killed.
    Timeout,
}

impl Status {
    pub const ALL: [Status; 6] = [
        Status::Pass,
        Status::Errors,
        Status::LoadFailed,
        Status::Panic,
        Status::Crash,
        Status::Timeout,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Errors => "errors",
            Status::LoadFailed => "load_failed",
            Status::Panic => "panic",
            Status::Crash => "crash",
            Status::Timeout => "timeout",
        }
    }
}

/// The phase a scenario run was in; a crash or timeout reports the last one reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    #[default]
    Queued,
    /// Loading the game (at worker start, or to switch between base and optional DLC).
    Game,
    Terrain,
    Load,
    Spawn,
    Start,
    Simulate,
    Inspect,
    Done,
}

/// One distinct script error of a scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorRecord {
    /// The grouping key: the error message, see [`Signature`].
    pub signature: String,
    /// The command the error names (`setPos` in "setPos: Type String, expected Array"; the
    /// command of an "Unimplemented command" error), when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The file of the first occurrence, as the report locates it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// How often it was reported.
    pub count: usize,
}

/// A command the VM has no implementation for, as one scenario met it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandUse {
    /// The command name, as the engine spells it.
    pub name: String,
    /// Uses: runtime "Unimplemented command" errors, or static occurrences in compiled code.
    pub count: usize,
}

/// A unit the World placed somewhere other than where the SQM put it, measured right after
/// spawning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Misplaced {
    /// The SQM unit `id`.
    pub id: i32,
    /// The `vehicle` config class.
    pub class: String,
    /// Where the SQM put it, world space.
    pub declared: [f64; 3],
    /// Where it spawned.
    pub spawned: [f64; 3],
}

/// A unit the mission places that was not created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unspawned {
    pub class: String,
    pub reason: String,
}

/// Whether the World looks plausible after the run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sanity {
    /// The mission places a player unit.
    pub player_expected: bool,
    /// That unit exists in the World at the end.
    pub player_present: bool,
    /// And is alive.
    pub player_alive: bool,
    /// Live Entities at the end.
    pub entities: usize,
    /// Entities alive at the end.
    pub alive: usize,
    /// Entities whose position is NaN or infinite.
    pub non_finite_positions: usize,
    /// Entities more than 5 m below the terrain surface.
    pub below_terrain: usize,
    /// Entities more than 2 km above the terrain surface. A statistic, not a failure: shipped
    /// missions place aircraft at altitude, so being high above the ground is the mission's
    /// business. What a misread `position[]` produces is measured by
    /// [`ScenarioResult::placement_mismatches`] instead.
    #[serde(default)]
    pub far_above_terrain: usize,
}

impl Sanity {
    /// The checks that failed, in words.
    pub fn failures(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.player_expected && !self.player_present {
            out.push("player missing".to_owned());
        } else if self.player_expected && !self.player_alive {
            out.push("player dead".to_owned());
        }
        if self.non_finite_positions > 0 {
            out.push(format!(
                "{} non-finite positions",
                self.non_finite_positions
            ));
        }
        if self.below_terrain > 0 {
            out.push(format!("{} below terrain", self.below_terrain));
        }
        out
    }
}

/// Wall-clock timings of one run, in milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Timings {
    pub terrain_ms: f64,
    pub load_ms: f64,
    pub spawn_ms: f64,
    pub start_ms: f64,
    pub simulate_ms: f64,
    pub total_ms: f64,
    /// Mean wall time of one simulated frame (World step + scheduled scripts).
    pub frame_ms_mean: f64,
    /// Slowest frame.
    pub frame_ms_max: f64,
}

/// Everything one scenario run produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub scenario: Scenario,
    pub status: Status,
    /// The last stage reached.
    pub stage: Stage,
    /// Why loading failed, the panic message, or what the supervisor saw of a crash or timeout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// Simulated mission time.
    pub sim_seconds: f64,
    pub frames: u64,
    /// The simulation stopped early because the scenario's wall-clock budget ran out.
    pub budget_exhausted: bool,
    pub timings: Timings,
    /// Units the mission places (present ones only).
    pub units_declared: usize,
    /// `version` of `mission.sqm` (12: 2D editor, 53: 3D editor); 0 when it did not load.
    #[serde(default)]
    pub sqm_version: i32,
    pub units_spawned: usize,
    pub unspawned: Vec<Unspawned>,
    /// Units the World placed somewhere other than where the SQM put them (more than a metre off
    /// in the plane, or in height when the SQM gave one), measured right after spawning. This is
    /// the check that catches a misread `position[]`: being high above the terrain is not a bug.
    #[serde(default)]
    pub misplaced: usize,
    /// Up to ten of those, in full.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub misplaced_examples: Vec<Misplaced>,
    /// Model files the spawned Entity types name that the VFS does not have.
    pub missing_models: Vec<String>,
    /// Start-up scripts that failed (`functions init`, `init of ...`, `init.sqf`).
    pub failed_scripts: Vec<String>,
    /// Why each of those failed: `"<script>: <error>"`, the error being the one that *ended* the
    /// script. A script reports only its first error (`docs/re/sqf-semantics.md`), which can be a
    /// harmless one the engine logs and carries on from, so the name alone does not say what
    /// stopped it.
    #[serde(default)]
    pub failed_script_errors: Vec<String>,
    /// Distinct script errors, most frequent first.
    pub errors: Vec<ErrorRecord>,
    /// Total script errors reported (sum of the records' counts).
    pub error_count: usize,
    /// Commands that stopped a script at runtime ("Unimplemented command").
    pub unimplemented_runtime: Vec<CommandUse>,
    /// Commands with no implementation that the mission's own scripts use (every `.sqf` of the
    /// mission folder, the init fields, the trigger expressions, the 3D editor's attribute
    /// expressions), counted statically.
    pub unimplemented_static: Vec<CommandUse>,
    /// Commands the mission's own scripts use whose record in the verification ledger says
    /// `stub`: they run, but their effect is a stand-in, so a pass that uses one does not rest on
    /// the engine. See [`crate::stubs`].
    #[serde(default)]
    pub stubbed_static: Vec<CommandUse>,
    /// Mission `.sqf` files that did not compile, with the message.
    pub compile_errors: Vec<(String, String)>,
    /// Scheduled scripts still running at the end.
    pub scripts_running: usize,
    pub sanity: Sanity,
}

impl ScenarioResult {
    /// A result for a scenario that produced nothing else (crash, timeout, load failure).
    pub fn failed(scenario: Scenario, status: Status, stage: Stage, failure: String) -> Self {
        Self {
            scenario,
            status,
            stage,
            failure: Some(failure),
            sim_seconds: 0.0,
            frames: 0,
            budget_exhausted: false,
            timings: Timings::default(),
            units_declared: 0,
            sqm_version: 0,
            units_spawned: 0,
            unspawned: Vec::new(),
            misplaced: 0,
            misplaced_examples: Vec::new(),
            missing_models: Vec::new(),
            failed_scripts: Vec::new(),
            failed_script_errors: Vec::new(),
            errors: Vec::new(),
            error_count: 0,
            unimplemented_runtime: Vec::new(),
            unimplemented_static: Vec::new(),
            stubbed_static: Vec::new(),
            compile_errors: Vec::new(),
            scripts_running: 0,
            sanity: Sanity::default(),
        }
    }

    /// [`Status::Pass`] or [`Status::Errors`] for a run that got to the end, from what it
    /// found.
    pub fn grade(&self) -> Status {
        let clean = self.error_count == 0
            && self.unimplemented_runtime.is_empty()
            && self.unspawned.is_empty()
            && self.misplaced == 0
            && self.missing_models.is_empty()
            && self.failed_scripts.is_empty()
            && self.compile_errors.is_empty()
            && self.sanity.failures().is_empty();
        if clean { Status::Pass } else { Status::Errors }
    }
}

/// A whole sweep, as written to `.work/sweep/<timestamp>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sweep {
    /// Local start time, `YYYY-MM-DD HH:MM:SS`.
    pub started: String,
    /// `git describe` of the engine, when known.
    #[serde(default)]
    pub engine: String,
    pub options: SweepOptions,
    /// The stub records the run's `pass with no stubs` count is based on; `None` when the
    /// verification ledger could not be read, in which case no pass can be told apart from a
    /// stubbed one.
    #[serde(default)]
    pub stubs: Option<StubIndex>,
    /// Wall-clock duration of the sweep in seconds.
    pub elapsed_s: f64,
    pub scenarios: Vec<ScenarioResult>,
}

/// The run parameters recorded with a sweep.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepOptions {
    pub seconds: f64,
    pub fps: f64,
    pub budget_s: f64,
    pub jobs: usize,
    pub terrain: bool,
    pub filters: Vec<String>,
    /// The build profile the sweep ran in (`release`, `debug`). `ms/frame` is only comparable
    /// within one profile; every other number is.
    #[serde(default)]
    pub profile: String,
}

/// An error report reduced to what groups it with its siblings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// The message, without position and source excerpt.
    pub message: String,
    /// The command it names, when it names one.
    pub command: Option<String>,
    /// The reported file, when the report has one.
    pub file: Option<String>,
}

/// Longest message kept in a signature.
const MAX_MESSAGE: usize = 160;

impl Signature {
    /// Parses an engine-style report:
    ///
    /// ```text
    /// Error in expression <...>
    ///   Error position: <...>
    ///   Error Undefined variable in expression: _x
    /// File a3\foo.sqf..., line 3
    /// ```
    ///
    /// A report without that shape (a host message, a compile error) is its own first line.
    pub fn parse(report: &str) -> Signature {
        let mut message = None;
        let mut file = None;
        for line in report.lines() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("Error ")
                && !rest.starts_with("in expression")
                && !rest.starts_with("position:")
                && message.is_none()
            {
                message = Some(rest.trim().to_owned());
            } else if let Some(rest) = line.strip_prefix("File ") {
                let path = rest.split("...").next().unwrap_or(rest).trim();
                let path = path.split(", line").next().unwrap_or(path).trim();
                if !path.is_empty() {
                    file = Some(path.to_owned());
                }
            }
        }
        let message =
            message.unwrap_or_else(|| report.lines().next().unwrap_or("").trim().to_owned());
        let message = truncate(&message, MAX_MESSAGE);
        let command = command_of(&message);
        Signature {
            message,
            command,
            file,
        }
    }
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}...", &text[..cut]),
        None => text.to_owned(),
    }
}

/// The command a message names: "Unimplemented command: X" names X; "cmd: Type ..." names cmd.
fn command_of(message: &str) -> Option<String> {
    if let Some(rest) = message.strip_prefix("Unimplemented command: ") {
        return Some(rest.trim().to_owned());
    }
    let (head, tail) = message.split_once(": ")?;
    let is_name = !head.is_empty() && head.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    (is_name && tail.starts_with("Type ")).then(|| head.to_owned())
}

/// Groups error reports by signature: one [`ErrorRecord`] per distinct message, most frequent
/// first, plus the commands of the "Unimplemented command" errors with their counts.
pub fn group_errors<'a>(
    reports: impl IntoIterator<Item = &'a str>,
) -> (Vec<ErrorRecord>, Vec<CommandUse>) {
    let mut records: BTreeMap<String, ErrorRecord> = BTreeMap::new();
    let mut unimplemented: BTreeMap<String, usize> = BTreeMap::new();
    for report in reports {
        let signature = Signature::parse(report);
        if signature.message.starts_with("Unimplemented command: ")
            && let Some(command) = &signature.command
        {
            *unimplemented.entry(command.clone()).or_default() += 1;
        }
        records
            .entry(signature.message.clone())
            .and_modify(|r| r.count += 1)
            .or_insert(ErrorRecord {
                signature: signature.message,
                command: signature.command,
                file: signature.file,
                count: 1,
            });
    }
    let mut records: Vec<ErrorRecord> = records.into_values().collect();
    records.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.signature.cmp(&b.signature))
    });
    (records, ranked(unimplemented))
}

/// `counts` as [`CommandUse`]s, most used first, ties by name.
pub fn ranked(counts: BTreeMap<String, usize>) -> Vec<CommandUse> {
    let mut out: Vec<CommandUse> = counts
        .into_iter()
        .map(|(name, count)| CommandUse { name, count })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    out
}

/// The `(name, form, uses)` of compiled code that `keep` accepts, summed per command name, most
/// used first. Both static scans (no implementation, stub record) come from one walk of the
/// code.
pub fn ranked_uses(
    uses: impl IntoIterator<Item = (String, Form, usize)>,
    mut keep: impl FnMut(&str, Form) -> bool,
) -> Vec<CommandUse> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (name, form, count) in uses {
        if keep(&name, form) {
            *counts.entry(name).or_default() += count;
        }
    }
    ranked(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNDEFINED: &str = "Error in expression <_a = _x + 1;>\n  Error position: <_x + 1;>\n  Error Undefined variable in expression: _x\nFile a3\\missions_f\\x.altis\\init.sqf..., line 3";

    #[test]
    fn a_runtime_report_reduces_to_its_message_and_file() {
        let s = Signature::parse(UNDEFINED);
        assert_eq!(s.message, "Undefined variable in expression: _x");
        assert_eq!(s.file.as_deref(), Some("a3\\missions_f\\x.altis\\init.sqf"));
        assert_eq!(s.command, None);
    }

    #[test]
    fn type_errors_and_unimplemented_commands_name_their_command() {
        let s = Signature::parse(
            "Error in expression <x>\n  Error position: <x>\n  Error setPos: Type String, expected Array\nFile f.sqf..., line 1",
        );
        assert_eq!(s.command.as_deref(), Some("setPos"));
        let s = Signature::parse("Error Unimplemented command: allowDamage");
        assert_eq!(s.message, "Unimplemented command: allowDamage");
        assert_eq!(s.command.as_deref(), Some("allowDamage"));
        assert_eq!(s.file, None);
    }

    #[test]
    fn a_report_of_another_shape_is_its_first_line() {
        let s = Signature::parse("Script a3\\x\\missing.sqf not found\nmore");
        assert_eq!(s.message, "Script a3\\x\\missing.sqf not found");
        assert_eq!(s.command, None);
    }

    #[test]
    fn long_messages_are_cut() {
        let long = format!("Error {}", "x".repeat(500));
        let s = Signature::parse(&long);
        assert_eq!(s.message.chars().count(), MAX_MESSAGE + 3);
    }

    #[test]
    fn errors_group_by_message_most_frequent_first() {
        let unimplemented = "Error in expression <a>\n  Error position: <a>\n  Error Unimplemented command: enableMimics\nFile m.sqm..., line 1";
        let (records, commands) = group_errors([UNDEFINED, unimplemented, UNDEFINED, UNDEFINED]);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].signature, "Undefined variable in expression: _x");
        assert_eq!(records[0].count, 3);
        assert_eq!(records[1].command.as_deref(), Some("enableMimics"));
        assert_eq!(
            commands,
            [CommandUse {
                name: "enableMimics".into(),
                count: 1
            }]
        );
    }

    #[test]
    fn a_clean_run_passes_and_any_finding_makes_it_errors() {
        let scenario = Scenario::for_test("a3\\m\\x.vr");
        let mut result =
            ScenarioResult::failed(scenario, Status::Errors, Stage::Done, String::new());
        result.failure = None;
        assert_eq!(result.grade(), Status::Pass);
        result.sanity.player_expected = true;
        assert_eq!(result.grade(), Status::Errors, "player missing");
        result.sanity.player_present = true;
        result.sanity.player_alive = true;
        assert_eq!(result.grade(), Status::Pass);
        result.missing_models.push("a3\\x.p3d".into());
        assert_eq!(result.grade(), Status::Errors);
    }
}
