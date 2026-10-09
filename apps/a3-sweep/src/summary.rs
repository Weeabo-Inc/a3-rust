//! Aggregates over a [`Sweep`] and the Markdown summary committed as
//! `docs/fidelity/scenario-sweep.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::inventory::Kind;
use crate::report::{ScenarioResult, Status, Sweep};

/// How many rows the ranked tables show.
pub const TOP: usize = 50;

/// One command the VM has no implementation for, over the whole sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRank {
    pub name: String,
    /// Scenarios in which it stopped a script at runtime.
    pub blocked: usize,
    /// Its "Unimplemented command" errors, over all scenarios.
    pub runtime_hits: usize,
    /// Scenarios whose own scripts use it.
    pub used_by: usize,
    /// Its static uses in those scripts.
    pub static_uses: usize,
}

/// Commands ranked by the scenarios they block at runtime, then by the scenarios whose scripts
/// use them.
pub fn command_ranking(results: &[ScenarioResult]) -> Vec<CommandRank> {
    // Command names are case-insensitive; the first spelling seen is kept.
    fn rank<'a>(ranks: &'a mut BTreeMap<String, CommandRank>, name: &str) -> &'a mut CommandRank {
        ranks
            .entry(name.to_ascii_lowercase())
            .or_insert_with(|| CommandRank {
                name: name.to_owned(),
                blocked: 0,
                runtime_hits: 0,
                used_by: 0,
                static_uses: 0,
            })
    }
    let mut ranks: BTreeMap<String, CommandRank> = BTreeMap::new();
    for result in results {
        for c in &result.unimplemented_runtime {
            let rank = rank(&mut ranks, &c.name);
            rank.blocked += 1;
            rank.runtime_hits += c.count;
        }
        for c in &result.unimplemented_static {
            let rank = rank(&mut ranks, &c.name);
            rank.used_by += 1;
            rank.static_uses += c.count;
        }
    }
    let mut out: Vec<CommandRank> = ranks.into_values().collect();
    out.sort_by(|a, b| {
        b.blocked
            .cmp(&a.blocked)
            .then(b.used_by.cmp(&a.used_by))
            .then(b.runtime_hits.cmp(&a.runtime_hits))
            .then(a.name.cmp(&b.name))
    });
    out
}

/// One error signature over the whole sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureRank {
    pub signature: String,
    /// Reports, over all scenarios.
    pub count: usize,
    /// Scenarios that reported it.
    pub scenarios: usize,
    /// A few of those scenarios (folder names).
    pub examples: Vec<String>,
}

/// Error signatures ranked by the scenarios they affect, then by count.
pub fn signature_ranking(results: &[ScenarioResult]) -> Vec<SignatureRank> {
    let mut ranks: BTreeMap<&str, SignatureRank> = BTreeMap::new();
    for result in results {
        for e in &result.errors {
            let rank = ranks.entry(&e.signature).or_insert_with(|| SignatureRank {
                signature: e.signature.clone(),
                count: 0,
                scenarios: 0,
                examples: Vec::new(),
            });
            rank.count += e.count;
            rank.scenarios += 1;
            if rank.examples.len() < 3 {
                rank.examples.push(result.scenario.name().to_owned());
            }
        }
    }
    let mut out: Vec<SignatureRank> = ranks.into_values().collect();
    out.sort_by(|a, b| {
        b.scenarios
            .cmp(&a.scenarios)
            .then(b.count.cmp(&a.count))
            .then(a.signature.cmp(&b.signature))
    });
    out
}

/// Counts per status for a set of results.
fn status_counts<'a>(
    results: impl IntoIterator<Item = &'a ScenarioResult>,
) -> BTreeMap<Status, usize> {
    let mut counts = BTreeMap::new();
    for r in results {
        *counts.entry(r.status).or_default() += 1;
    }
    counts
}

/// Text safe inside a Markdown table cell.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
        .replace(['\n', '\r'], " ")
        .replace('`', "'")
}

fn percent(part: usize, whole: usize) -> String {
    if whole == 0 {
        "-".to_owned()
    } else {
        format!("{:.1}%", part as f64 * 100.0 / whole as f64)
    }
}

/// A table of status counts per group.
fn status_table(out: &mut String, label: &str, groups: &BTreeMap<String, Vec<&ScenarioResult>>) {
    let _ = write!(out, "| {label} | scenarios | pass rate |");
    for status in Status::ALL {
        let _ = write!(out, " {} |", status.as_str());
    }
    out.push('\n');
    out.push_str("|---|--:|--:|");
    for _ in Status::ALL {
        out.push_str("--:|");
    }
    out.push('\n');
    for (name, results) in groups {
        let counts = status_counts(results.iter().copied());
        let pass = counts.get(&Status::Pass).copied().unwrap_or(0);
        let _ = write!(
            out,
            "| {} | {} | {} |",
            cell(name),
            results.len(),
            percent(pass, results.len())
        );
        for status in Status::ALL {
            let _ = write!(out, " {} |", counts.get(&status).copied().unwrap_or(0));
        }
        out.push('\n');
    }
    out.push('\n');
}

/// The Markdown summary of a sweep.
pub fn markdown(sweep: &Sweep) -> String {
    let results = &sweep.scenarios;
    let total = results.len();
    let counts = status_counts(results);
    let count = |s: Status| counts.get(&s).copied().unwrap_or(0);
    let ran = total
        - count(Status::LoadFailed)
        - count(Status::Panic)
        - count(Status::Crash)
        - count(Status::Timeout);
    let mut out = String::new();

    out.push_str("# Scenario sweep\n\n");
    out.push_str(
        "Generated by `a3-sweep` ([how to run it](README.md#scenario-sweep)); do not edit by hand.\n\n",
    );
    let _ = writeln!(
        out,
        "Sweep of {} (engine `{}`, {} build): **{total} scenarios**, {} simulated seconds each at \
         {} fps (wall budget {} s per scenario), {} worker(s), terrain {}{}. The sweep took {:.0} s.\n",
        sweep.started,
        if sweep.engine.is_empty() {
            "unknown"
        } else {
            &sweep.engine
        },
        if sweep.options.profile.is_empty() {
            "unknown-profile"
        } else {
            &sweep.options.profile
        },
        sweep.options.seconds,
        sweep.options.fps,
        sweep.options.budget_s,
        sweep.options.jobs,
        if sweep.options.terrain {
            "loaded"
        } else {
            "off"
        },
        if sweep.options.filters.is_empty() {
            String::new()
        } else {
            format!(", filters {:?}", sweep.options.filters)
        },
        sweep.elapsed_s,
    );
    out.push_str("A scenario **passes** when it loads, starts and simulates with no script error, no unimplemented command, no unspawned or misplaced unit, no missing model, every start-up script succeeding and the sanity checks holding (player present and alive, no non-finite positions, nothing below the terrain).\n\n");
    out.push_str("*ms/frame* is wall-clock time of one simulated frame in the build profile above; it is not comparable across profiles or machines. Every other number is reproducible from the same install.\n\n");

    out.push_str("## Totals\n\n");
    let _ = writeln!(
        out,
        "- **Pass rate: {} ({} of {total})**",
        percent(count(Status::Pass), total),
        count(Status::Pass)
    );
    let _ = writeln!(
        out,
        "- Ran to the end (pass or errors): {} ({ran})",
        percent(ran, total)
    );
    for status in Status::ALL {
        let _ = writeln!(out, "- {}: {}", status.as_str(), count(status));
    }
    out.push('\n');

    let mut by_kind: BTreeMap<String, Vec<&ScenarioResult>> = BTreeMap::new();
    let mut by_world: BTreeMap<String, Vec<&ScenarioResult>> = BTreeMap::new();
    let mut by_package: BTreeMap<String, Vec<&ScenarioResult>> = BTreeMap::new();
    for r in results {
        by_kind
            .entry(r.scenario.kind.as_str().to_owned())
            .or_default()
            .push(r);
        by_world
            .entry(r.scenario.world.clone())
            .or_default()
            .push(r);
        by_package
            .entry(r.scenario.package.clone())
            .or_default()
            .push(r);
    }
    out.push_str("### By kind\n\n");
    status_table(&mut out, "kind", &by_kind);
    out.push_str("### By world\n\n");
    status_table(&mut out, "world", &by_world);
    out.push_str("### By package\n\n");
    status_table(&mut out, "package", &by_package);
    let mut by_format: BTreeMap<String, Vec<&ScenarioResult>> = BTreeMap::new();
    for r in results {
        let format = match r.sqm_version {
            0 => "not loaded".to_owned(),
            v => format!("version {v}"),
        };
        by_format.entry(format).or_default().push(r);
    }
    out.push_str("### By `mission.sqm` format\n\n");
    status_table(&mut out, "format", &by_format);

    out.push_str("## Unimplemented commands, by scenarios blocked\n\n");
    out.push_str("*Blocked*: scenarios in which the command stopped a script at runtime (\"Unimplemented command\"). *Used by*: scenarios whose own scripts (the `.sqf` files of the mission folder, init fields, trigger expressions) contain it.\n\n");
    out.push_str("| # | command | blocked | runtime hits | used by | static uses |\n|--:|---|--:|--:|--:|--:|\n");
    for (i, c) in command_ranking(results).iter().take(TOP).enumerate() {
        let _ = writeln!(
            out,
            "| {} | `{}` | {} | {} | {} | {} |",
            i + 1,
            c.name,
            c.blocked,
            c.runtime_hits,
            c.used_by,
            c.static_uses
        );
    }
    out.push('\n');

    out.push_str("## Top error signatures\n\n");
    out.push_str("| # | signature | scenarios | reports | examples |\n|--:|---|--:|--:|---|\n");
    for (i, s) in signature_ranking(results).iter().take(TOP).enumerate() {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} |",
            i + 1,
            cell(&s.signature),
            s.scenarios,
            s.count,
            cell(&s.examples.join(", "))
        );
    }
    out.push('\n');

    let failures: Vec<&ScenarioResult> = results
        .iter()
        .filter(|r| {
            matches!(
                r.status,
                Status::LoadFailed | Status::Panic | Status::Crash | Status::Timeout
            )
        })
        .collect();
    out.push_str("## Load failures, panics, crashes and timeouts\n\n");
    if failures.is_empty() {
        out.push_str("None.\n\n");
    } else {
        out.push_str("| scenario | status | stage | failure |\n|---|---|---|---|\n");
        for r in failures {
            let failure = r.failure.as_deref().unwrap_or("");
            let first = failure.lines().next().unwrap_or("");
            let _ = writeln!(
                out,
                "| {} | {} | {:?} | {} |",
                cell(&r.scenario.folder),
                r.status.as_str(),
                r.stage,
                cell(&truncate(first, 200))
            );
        }
        out.push('\n');
    }

    let mut unspawned: BTreeMap<&str, (usize, BTreeSet<&str>)> = BTreeMap::new();
    let mut models: BTreeMap<&str, usize> = BTreeMap::new();
    for r in results {
        for u in &r.unspawned {
            let e = unspawned.entry(&u.class).or_default();
            e.0 += 1;
            e.1.insert(&u.reason);
        }
        for m in &r.missing_models {
            *models.entry(m).or_default() += 1;
        }
    }
    out.push_str("## Missing classes and models\n\n");
    if unspawned.is_empty() && models.is_empty() {
        out.push_str("None.\n\n");
    } else {
        let mut rows: Vec<_> = unspawned.into_iter().collect();
        rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
        out.push_str("| unspawned class | units | reason |\n|---|--:|---|\n");
        for (class, (n, reasons)) in rows.into_iter().take(TOP) {
            let reasons: Vec<&str> = reasons.into_iter().collect();
            let _ = writeln!(out, "| `{}` | {n} | {} |", class, cell(&reasons.join("; ")));
        }
        out.push('\n');
        let mut rows: Vec<_> = models.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        if !rows.is_empty() {
            out.push_str("| missing model | scenarios |\n|---|--:|\n");
            for (model, n) in rows.into_iter().take(TOP) {
                let _ = writeln!(out, "| `{model}` | {n} |");
            }
            out.push('\n');
        }
    }

    out.push_str("## Slowest simulations\n\n");
    let mut slow: Vec<&ScenarioResult> = results.iter().filter(|r| r.frames > 0).collect();
    slow.sort_by(|a, b| b.timings.frame_ms_mean.total_cmp(&a.timings.frame_ms_mean));
    out.push_str("| scenario | ms/frame (mean) | ms/frame (max) | entities | budget hit |\n|---|--:|--:|--:|---|\n");
    for r in slow.into_iter().take(15) {
        let _ = writeln!(
            out,
            "| {} | {:.2} | {:.1} | {} | {} |",
            cell(r.scenario.name()),
            r.timings.frame_ms_mean,
            r.timings.frame_ms_max,
            r.sanity.entities,
            if r.budget_exhausted { "yes" } else { "" }
        );
    }
    out.push('\n');

    out.push_str("## Scenarios\n\n");
    out.push_str("| scenario | kind | world | status | errors | blocked by | sim s | ms/frame | notes |\n|---|---|---|---|--:|---|--:|--:|---|\n");
    let mut rows: Vec<&ScenarioResult> = results.iter().collect();
    rows.sort_by(|a, b| {
        a.scenario
            .kind
            .cmp(&b.scenario.kind)
            .then(a.scenario.folder.cmp(&b.scenario.folder))
    });
    for r in rows {
        let blocked: Vec<&str> = r
            .unimplemented_runtime
            .iter()
            .take(3)
            .map(|c| c.name.as_str())
            .collect();
        let mut notes = r.sanity.failures();
        if !r.unspawned.is_empty() {
            notes.push(format!("{} unspawned", r.unspawned.len()));
        }
        if r.misplaced > 0 {
            notes.push(format!("{} misplaced", r.misplaced));
        }
        if !r.failed_scripts.is_empty() {
            notes.push(format!(
                "failed: {}",
                listed(&r.failed_scripts, FAILED_SCRIPTS_SHOWN)
            ));
        }
        if !r.compile_errors.is_empty() {
            notes.push(format!("{} files do not compile", r.compile_errors.len()));
        }
        if r.budget_exhausted {
            notes.push("budget hit".to_owned());
        }
        if let Some(failure) = &r.failure {
            notes.push(truncate(failure.lines().next().unwrap_or(""), 80));
        }
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {:.0} | {:.2} | {} |",
            cell(&r.scenario.folder),
            r.scenario.kind.as_str(),
            r.scenario.world,
            r.status.as_str(),
            r.error_count,
            cell(&blocked.join(", ")),
            r.sim_seconds,
            r.timings.frame_ms_mean,
            cell(&truncate(&notes.join("; "), MAX_NOTE)),
        );
    }
    out
}

/// How many names [`listed`] shows before it counts the rest.
const FAILED_SCRIPTS_SHOWN: usize = 6;

/// Longest notes cell in the per-scenario table.
const MAX_NOTE: usize = 300;

/// A few names, then `(+N more)`: the per-scenario table must stay readable when a mission has
/// hundreds of failed init fields (full lists are in the JSON).
fn listed(names: &[String], show: usize) -> String {
    let shown = names
        .iter()
        .take(show)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > show {
        format!("{shown} (+{} more)", names.len() - show)
    } else {
        shown
    }
}

/// The inventory as a Markdown table of counts per kind and world.
pub fn inventory_table(scenarios: &[crate::inventory::Scenario]) -> String {
    let mut worlds = BTreeSet::new();
    let mut counts: BTreeMap<(Kind, String), usize> = BTreeMap::new();
    for s in scenarios {
        worlds.insert(s.world.clone());
        *counts.entry((s.kind, s.world.clone())).or_default() += 1;
    }
    let mut out = String::from("| kind | total |");
    for w in &worlds {
        let _ = write!(out, " {w} |");
    }
    out.push_str("\n|---|--:|");
    for _ in &worlds {
        out.push_str("--:|");
    }
    out.push('\n');
    for kind in Kind::ALL {
        let total: usize = worlds
            .iter()
            .map(|w| counts.get(&(kind, w.clone())).copied().unwrap_or(0))
            .sum();
        if total == 0 {
            continue;
        }
        let _ = write!(out, "| {} | {total} |", kind.as_str());
        for w in &worlds {
            let _ = write!(
                out,
                " {} |",
                counts.get(&(kind, w.clone())).copied().unwrap_or(0)
            );
        }
        out.push('\n');
    }
    let _ = write!(out, "| **all** | **{}** |", scenarios.len());
    for w in &worlds {
        let n: usize = Kind::ALL
            .iter()
            .map(|k| counts.get(&(*k, w.clone())).copied().unwrap_or(0))
            .sum();
        let _ = write!(out, " {n} |");
    }
    out.push('\n');
    out
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}...", &text[..cut]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::Scenario;
    use crate::report::{CommandUse, ErrorRecord, Stage, SweepOptions};

    fn result(folder: &str, status: Status) -> ScenarioResult {
        let mut r = ScenarioResult::failed(
            Scenario::for_test(folder),
            status,
            Stage::Done,
            String::new(),
        );
        if matches!(status, Status::Pass | Status::Errors) {
            r.failure = None;
        }
        r
    }

    fn uses(names: &[(&str, usize)]) -> Vec<CommandUse> {
        names
            .iter()
            .map(|(n, c)| CommandUse {
                name: (*n).to_owned(),
                count: *c,
            })
            .collect()
    }

    fn sweep() -> Sweep {
        let mut a = result("a3\\m\\a.altis", Status::Errors);
        a.unimplemented_runtime = uses(&[("allowDamage", 3)]);
        a.unimplemented_static = uses(&[("allowDamage", 5), ("enableMimics", 1)]);
        a.errors = vec![ErrorRecord {
            signature: "Unimplemented command: allowDamage".into(),
            command: Some("allowDamage".into()),
            file: None,
            count: 3,
        }];
        a.error_count = 3;
        let mut b = result("a3\\m\\b.stratis", Status::Errors);
        b.unimplemented_runtime = uses(&[("allowdamage", 1), ("setMarkerPos", 2)]);
        b.unimplemented_static = uses(&[("allowDamage", 1)]);
        b.errors = vec![ErrorRecord {
            signature: "Unimplemented command: allowDamage".into(),
            command: Some("allowDamage".into()),
            file: None,
            count: 1,
        }];
        b.error_count = 1;
        let c = result("a3\\m\\c.altis", Status::Pass);
        let d = result("a3\\m\\d.altis", Status::Crash);
        Sweep {
            started: "2026-10-09 12:00:00".into(),
            engine: "abc123".into(),
            options: SweepOptions {
                seconds: 60.0,
                fps: 20.0,
                budget_s: 120.0,
                jobs: 2,
                terrain: true,
                filters: vec![],
                profile: "release".into(),
            },
            elapsed_s: 10.0,
            scenarios: vec![a, b, c, d],
        }
    }

    #[test]
    fn commands_rank_by_blocked_scenarios_ignoring_case() {
        let ranks = command_ranking(&sweep().scenarios);
        assert_eq!(ranks[0].name, "allowDamage");
        assert_eq!(
            (
                ranks[0].blocked,
                ranks[0].runtime_hits,
                ranks[0].used_by,
                ranks[0].static_uses
            ),
            (2, 4, 2, 6)
        );
        assert_eq!(ranks[1].name, "setMarkerPos");
        assert_eq!(ranks[2].name, "enableMimics");
        assert_eq!(ranks[2].blocked, 0);
    }

    #[test]
    fn signatures_rank_by_affected_scenarios_with_examples() {
        let ranks = signature_ranking(&sweep().scenarios);
        assert_eq!(ranks.len(), 1);
        assert_eq!((ranks[0].scenarios, ranks[0].count), (2, 4));
        assert_eq!(ranks[0].examples, ["a.altis", "b.stratis"]);
    }

    #[test]
    fn the_summary_has_totals_rankings_and_a_row_per_scenario() {
        let md = markdown(&sweep());
        assert!(md.contains("**Pass rate: 25.0% (1 of 4)**"), "{md}");
        assert!(md.contains("| 1 | `allowDamage` | 2 | 4 | 2 | 6 |"), "{md}");
        assert!(md.contains("| a3\\m\\d.altis | crash |"), "{md}");
        assert!(md.contains("engine `abc123`, release build"), "{md}");
        for folder in ["a.altis", "b.stratis", "c.altis", "d.altis"] {
            assert!(
                md.contains(&format!("a3\\m\\{folder} | unlisted |")),
                "{folder}"
            );
        }
    }

    #[test]
    fn a_long_failed_script_list_is_shortened() {
        assert_eq!(listed(&[], 3), "");
        let names: Vec<String> = (0..10).map(|i| format!("init of unit {i}")).collect();
        assert_eq!(
            listed(&names, 3),
            "init of unit 0, init of unit 1, init of unit 2 (+7 more)"
        );
        assert_eq!(listed(&names[..2], 3), "init of unit 0, init of unit 1");
    }

    #[test]
    fn cells_escape_table_syntax() {
        assert_eq!(cell("a|b\nc`d"), "a\\|b c'd");
    }

    #[test]
    fn the_inventory_table_counts_kinds_per_world() {
        let mut a = Scenario::for_test("a3\\m\\a.altis");
        a.kind = Kind::Campaign;
        let b = Scenario::for_test("a3\\m\\b.altis");
        let c = Scenario::for_test("a3\\m\\c.vr");
        let table = inventory_table(&[a, b, c]);
        assert!(table.contains("| campaign | 1 | 1 | 0 |"), "{table}");
        assert!(table.contains("| unlisted | 2 | 1 | 1 |"), "{table}");
        assert!(table.contains("| **all** | **3** | 2 | 1 |"), "{table}");
    }
}
