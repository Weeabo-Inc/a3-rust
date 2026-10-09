//! `a3-tools sqf coverage`: the SQF command coverage ledger (`docs/fidelity/sqf-coverage.md`).
//!
//! One row per engine command overload (`docs/re/sqf-commands.tsv`, simple-expression commands
//! left out): whether a crate implements it, how the implementation was verified
//! (`docs/fidelity/sqf-verified.tsv`), and how often shipped code uses the command
//! ([`a3_gamedata::command_usage`]). The backlog sorts what is missing by usage.

use std::collections::HashMap;
use std::fmt::Write as _;

use a3_gamedata::{CommandUsage, VfsHost};
use a3_sqf::{CommandTable, Form, Host, Registry, Signature, TypeSet};

/// A crate's registered commands, for attribution.
pub struct CrateCommands {
    pub name: &'static str,
    /// Whether the crate's answers are fixed headless stand-ins (counted as stubs).
    pub stub: bool,
    overloads: HashMap<(String, FormKey), Vec<Signature>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum FormKey {
    Nular,
    Unary,
    Binary,
}

impl From<Form> for FormKey {
    fn from(f: Form) -> Self {
        match f {
            Form::Nular => FormKey::Nular,
            Form::Unary => FormKey::Unary,
            Form::Binary => FormKey::Binary,
        }
    }
}

impl CrateCommands {
    /// The commands `registry` implements.
    pub fn from_registry<H: Host>(name: &'static str, stub: bool, registry: &Registry<H>) -> Self {
        let mut overloads = HashMap::new();
        for (_, info) in registry.table().iter() {
            for form in [Form::Nular, Form::Unary, Form::Binary] {
                let sigs = registry.overloads(&info.name, form);
                if !sigs.is_empty() {
                    overloads.insert((info.name.to_ascii_lowercase(), form.into()), sigs);
                }
            }
        }
        Self {
            name,
            stub,
            overloads,
        }
    }

    fn get(&self, name: &str, form: Form) -> &[Signature] {
        self.overloads
            .get(&(name.to_ascii_lowercase(), form.into()))
            .map_or(&[], Vec::as_slice)
    }
}

/// Never constructed: gives `register_ui_commands` a host type so its registrations can be
/// listed.
struct UiProbe;

impl Host for UiProbe {}

impl a3_ui::UiHost for UiProbe {
    fn ui(&self) -> &a3_ui::Ui {
        unreachable!("registry probe only")
    }

    fn ui_mut(&mut self) -> &mut a3_ui::Ui {
        unreachable!("registry probe only")
    }

    fn ui_config(&self) -> Option<std::sync::Arc<a3_config::ConfigTree>> {
        None
    }
}

/// The command registrations of every crate that adds script commands.
pub fn crate_commands() -> Vec<CrateCommands> {
    fn empty<H: Host>() -> Registry<H> {
        Registry::new(CommandTable::new())
    }
    let mut core = empty::<a3_sqf::NullHost>();
    a3_sqf::commands::register_core(&mut core);
    let mut config = empty::<VfsHost>();
    a3_gamedata::register_config_commands(&mut config);
    let mut headless = empty::<VfsHost>();
    a3_gamedata::register_headless(&mut headless);
    let mut world = empty::<a3_world::script::ScriptWorld>();
    a3_world::script::register_world_commands(&mut world);
    let mut ui = empty::<UiProbe>();
    a3_ui::register_ui_commands(&mut ui);
    vec![
        CrateCommands::from_registry("a3-sqf", false, &core),
        CrateCommands::from_registry("a3-gamedata", false, &config),
        CrateCommands::from_registry("a3-world", false, &world),
        CrateCommands::from_registry("a3-ui", false, &ui),
        CrateCommands::from_registry("a3-gamedata (headless)", true, &headless),
    ]
}

/// One overload of `docs/re/sqf-commands.tsv`.
#[derive(Debug, Clone)]
pub struct EngineOverload {
    pub name: String,
    pub form: Form,
    /// Type strings as the TSV prints them (`SCALAR|NaN`, `?|ARRAY`).
    pub left: String,
    pub right: String,
    pub ret: String,
    /// Handler RVA (`0x8a6fc0`).
    pub handler: String,
}

/// Parses `docs/re/sqf-commands.tsv`, leaving out `Simple expression` commands.
pub fn parse_engine_commands(text: &str) -> Vec<EngineOverload> {
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or_default().split('\t').collect();
    let col = |name: &str| header.iter().position(|h| *h == name);
    let (Some(c_name), Some(c_kind), Some(c_left), Some(c_right), Some(c_ret), Some(c_handler)) = (
        col("name"),
        col("kind"),
        col("left_type"),
        col("right_type"),
        col("return_type"),
        col("handler"),
    ) else {
        return Vec::new();
    };
    let c_cat = col("category");
    let mut out = Vec::new();
    for line in lines {
        let c: Vec<&str> = line.trim_end_matches('\r').split('\t').collect();
        if c.len() < header.len().min(6) {
            continue;
        }
        if c_cat.is_some_and(|i| c.get(i) == Some(&"Simple expression")) {
            continue;
        }
        let form = match c[c_kind] {
            "nular" => Form::Nular,
            "unary" => Form::Unary,
            "binary" => Form::Binary,
            _ => continue,
        };
        out.push(EngineOverload {
            name: c[c_name].to_owned(),
            form,
            left: c[c_left].to_owned(),
            right: c[c_right].to_owned(),
            ret: c[c_ret].to_owned(),
            handler: c[c_handler].to_owned(),
        });
    }
    out
}

/// A record of `docs/fidelity/sqf-verified.tsv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    pub name: String,
    pub form: Form,
    /// The overload's handler RVA, or `*` for every overload of the form.
    pub handler: String,
    /// `verified` or `stub`.
    pub status: String,
    /// How: `decompiled <addr>`, `oracle`, `wiki`.
    pub source: String,
    pub note: String,
}

/// Parses `docs/fidelity/sqf-verified.tsv`: `name, form, handler, status, source, note`;
/// lines starting with `#` are comments.
pub fn parse_verified(text: &str) -> Vec<Verification> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() < 5 {
            continue;
        }
        let form = match c[1] {
            "nular" => Form::Nular,
            "unary" => Form::Unary,
            "binary" => Form::Binary,
            _ => continue,
        };
        out.push(Verification {
            name: c[0].to_owned(),
            form,
            handler: c[2].to_owned(),
            status: c[3].to_owned(),
            source: c[4].to_owned(),
            note: c.get(5).map(|s| s.to_string()).unwrap_or_default(),
        });
    }
    out
}

/// The implementation state of one overload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Implemented and checked against the decompiled handler or the oracle.
    Verified,
    /// Implemented; how it was checked is not recorded.
    Implemented,
    /// Implemented for some of the engine's argument types.
    Partial,
    /// The command contract is implemented; its side effect is a stand-in.
    Stub,
    Missing,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Verified => "verified",
            Status::Implemented => "implemented",
            Status::Partial => "partial",
            Status::Stub => "stub",
            Status::Missing => "missing",
        }
    }
}

/// One ledger row.
#[derive(Debug, Clone)]
pub struct Row {
    pub overload: EngineOverload,
    pub status: Status,
    pub crates: Vec<&'static str>,
    pub verified: String,
    pub uses: [usize; 3],
}

/// A table type string as a set; `?` parts (types the extractor did not resolve) are dropped
/// and reported as `unknown`.
fn engine_types(s: &str) -> (TypeSet, bool) {
    let mut unknown = false;
    let known: Vec<&str> = s
        .split('|')
        .filter(|t| {
            let q = t.trim() == "?";
            unknown |= q;
            !q && !t.trim().is_empty()
        })
        .collect();
    let set = TypeSet::parse(&known.join("|")).unwrap_or(TypeSet::EMPTY);
    (set, unknown || (set.is_empty() && !s.trim().is_empty()))
}

/// Builds the ledger rows.
pub fn ledger_rows(
    engine: &[EngineOverload],
    crates: &[CrateCommands],
    verified: &[Verification],
    usage: Option<&CommandUsage>,
) -> Vec<Row> {
    let mut rows = Vec::with_capacity(engine.len());
    for o in engine {
        let (left, left_unknown) = engine_types(&o.left);
        let (right, right_unknown) = engine_types(&o.right);
        let side_matches = |want: TypeSet, unknown: bool, have: TypeSet| {
            unknown || want.is_empty() || want.intersects(have)
        };
        let mut names = Vec::new();
        let mut stub = false;
        let (mut have_left, mut have_right) = (TypeSet::EMPTY, TypeSet::EMPTY);
        for c in crates {
            let matching: Vec<&Signature> = c
                .get(&o.name, o.form)
                .iter()
                .filter(|s| match o.form {
                    Form::Nular => true,
                    Form::Unary => side_matches(right, right_unknown, s.right),
                    Form::Binary => {
                        side_matches(left, left_unknown, s.left)
                            && side_matches(right, right_unknown, s.right)
                    }
                })
                .collect();
            if matching.is_empty() {
                continue;
            }
            // A stand-in only counts when no real implementation exists.
            if c.stub && !names.is_empty() {
                continue;
            }
            names.push(c.name);
            stub |= c.stub;
            for s in matching {
                have_left = have_left.union(s.left);
                have_right = have_right.union(s.right);
            }
        }
        let record = verified.iter().find(|v| {
            v.form == o.form
                && v.name.eq_ignore_ascii_case(&o.name)
                && (v.handler == "*" || v.handler.eq_ignore_ascii_case(&o.handler))
        });
        let covers = match o.form {
            Form::Nular => true,
            Form::Unary => right_unknown || have_right.covers(right),
            Form::Binary => {
                (left_unknown || have_left.covers(left))
                    && (right_unknown || have_right.covers(right))
            }
        };
        let status = if names.is_empty() {
            Status::Missing
        } else if stub || record.is_some_and(|r| r.status == "stub") {
            Status::Stub
        } else if !covers {
            Status::Partial
        } else if record.is_some_and(|r| r.status == "verified") {
            Status::Verified
        } else {
            Status::Implemented
        };
        let verified = match (record, status) {
            (_, Status::Missing) => String::new(),
            (Some(r), _) => r.source.clone(),
            (None, Status::Stub) => "headless stand-in".to_owned(),
            (None, _) => "unrecorded".to_owned(),
        };
        rows.push(Row {
            overload: o.clone(),
            status,
            crates: names,
            verified,
            uses: usage.map_or([0; 3], |u| u.get(&o.name, o.form)),
        });
    }
    rows
}

fn signature(o: &EngineOverload) -> String {
    match o.form {
        Form::Nular => format!("→ {}", o.ret),
        Form::Unary => format!("{} → {}", o.right, o.ret),
        Form::Binary => format!("{} · {} → {}", o.left, o.right, o.ret),
    }
}

fn md_escape(s: &str) -> String {
    s.replace('|', "\\|")
}

/// Totals by status: `(overloads, names)`; a name counts under its worst overload status.
fn totals(rows: &[Row]) -> Vec<(Status, usize, usize)> {
    let mut by_name: HashMap<String, Status> = HashMap::new();
    for r in rows {
        let e = by_name
            .entry(r.overload.name.to_ascii_lowercase())
            .or_insert(r.status);
        *e = (*e).max(r.status);
    }
    [
        Status::Verified,
        Status::Implemented,
        Status::Partial,
        Status::Stub,
        Status::Missing,
    ]
    .into_iter()
    .map(|s| {
        (
            s,
            rows.iter().filter(|r| r.status == s).count(),
            by_name.values().filter(|v| **v == s).count(),
        )
    })
    .collect()
}

/// The ledger rows as TSV, for scripts: `name, form, left, right, return, handler, status,
/// crates, verified, uses_sqf, uses_fsm, uses_config`.
pub fn render_tsv(rows: &[Row]) -> String {
    let mut out = String::from(
        "name	form	left	right	return	handler	status	crates	verified	uses_sqf	uses_fsm	uses_config
",
    );
    for r in rows {
        let o = &r.overload;
        let _ = writeln!(
            out,
            "{}	{}	{}	{}	{}	{}	{}	{}	{}	{}	{}	{}",
            o.name,
            o.form.as_str(),
            o.left,
            o.right,
            o.ret,
            o.handler,
            r.status.as_str(),
            r.crates.join(","),
            r.verified,
            r.uses[0],
            r.uses[1],
            r.uses[2]
        );
    }
    out
}

/// Renders the ledger as Markdown.
pub fn render(rows: &[Row], usage: Option<&CommandUsage>) -> String {
    let mut out = String::new();
    let names: std::collections::HashSet<String> = rows
        .iter()
        .map(|r| r.overload.name.to_ascii_lowercase())
        .collect();
    let _ = writeln!(out, "# SQF command coverage ledger\n");
    let _ = writeln!(
        out,
        "Generated by `a3-tools sqf --all-mods coverage` (see `apps/a3-tools/src/coverage_cmd.rs`); \
         do not edit by hand. Rows are the engine's command overloads from \
         `docs/re/sqf-commands.tsv` (arma3_x64.exe 2.22.0.154103), simple-expression commands \
         left out: {} overloads, {} names. Verification records live in \
         `docs/fidelity/sqf-verified.tsv`.\n",
        rows.len(),
        names.len()
    );
    let _ = writeln!(out, "Status:\n");
    let _ = writeln!(
        out,
        "- **verified**: implemented and checked against the decompiled handler (or the oracle)."
    );
    let _ = writeln!(
        out,
        "- **implemented**: implemented before verification was recorded (`unrecorded`)."
    );
    let _ = writeln!(
        out,
        "- **partial**: implemented for some of the overload's argument types only."
    );
    let _ = writeln!(
        out,
        "- **stub**: the command contract (arguments, errors, return) is implemented; its side \
         effect is a stand-in until the subsystem exists, or it returns a fixed headless answer."
    );
    let _ = writeln!(
        out,
        "- **missing**: raises `Unimplemented command` at run time.\n"
    );
    let _ = writeln!(
        out,
        "Uses are static call sites in shipped code (base game and every DLC mounted): `.sqf` \
         files, `.fsm` state/link scripts, and config-embedded SQF (`on<Event>` handlers, \
         `EventHandlers` classes, `statement`/`condition`/`expression`/`action`/`init` entries; \
         identical texts once). They are per name and form, so overloads of one form share them.\n"
    );
    if let Some(u) = usage {
        let _ = writeln!(
            out,
            "Scanned: {} `.sqf` files ({} failed), {} FSM scripts ({} failed), {} config scripts \
             ({} failed).\n",
            u.compiled[0], u.failed[0], u.compiled[1], u.failed[1], u.compiled[2], u.failed[2]
        );
    }
    let _ = writeln!(out, "## Totals\n");
    let _ = writeln!(out, "| Status | Overloads | Names |\n|---|---:|---:|");
    for (s, o, n) in totals(rows) {
        let _ = writeln!(out, "| {} | {o} | {n} |", s.as_str());
    }
    let _ = writeln!(
        out,
        "\nA name counts under the least complete status among its overloads.\n"
    );

    // Backlog: per name and form, the overloads still missing (or partial), by usage.
    let mut backlog: Vec<(&Row, Vec<&Row>)> = Vec::new();
    for r in rows
        .iter()
        .filter(|r| matches!(r.status, Status::Missing | Status::Partial))
    {
        match backlog.iter_mut().find(|(first, _)| {
            first.overload.form == r.overload.form
                && first.overload.name.eq_ignore_ascii_case(&r.overload.name)
        }) {
            Some((_, all)) => all.push(r),
            None => backlog.push((r, vec![r])),
        }
    }
    backlog.sort_by(|a, b| {
        let ta: usize = a.0.uses.iter().sum();
        let tb: usize = b.0.uses.iter().sum();
        tb.cmp(&ta).then_with(|| {
            a.0.overload
                .name
                .to_ascii_lowercase()
                .cmp(&b.0.overload.name.to_ascii_lowercase())
        })
    });
    let used = backlog
        .iter()
        .filter(|(r, _)| r.uses.iter().sum::<usize>() > 0)
        .count();
    let _ = writeln!(out, "## Backlog by usage\n");
    let _ = writeln!(
        out,
        "{} name/form pairs have missing or partial overloads; {used} of them are used by shipped \
         code. Implement from the top.\n",
        backlog.len()
    );
    let _ = writeln!(
        out,
        "| # | Command | Form | Missing overloads | Handler | sqf | fsm | cfg | Total |\n\
         |---:|---|---|---|---|---:|---:|---:|---:|"
    );
    for (i, (first, all)) in backlog.iter().enumerate() {
        let sigs: Vec<String> = all
            .iter()
            .map(|r| {
                let s = md_escape(&signature(&r.overload));
                if r.status == Status::Partial {
                    format!("{s} (partial)")
                } else {
                    s
                }
            })
            .collect();
        let handlers: Vec<&str> = all.iter().map(|r| r.overload.handler.as_str()).collect();
        let u = first.uses;
        let _ = writeln!(
            out,
            "| {} | `{}` | {} | {} | {} | {} | {} | {} | {} |",
            i + 1,
            md_escape(&first.overload.name),
            first.overload.form.as_str(),
            sigs.join("<br>"),
            handlers.join("<br>"),
            u[0],
            u[1],
            u[2],
            u.iter().sum::<usize>()
        );
    }

    let _ = writeln!(out, "\n## All overloads\n");
    let _ = writeln!(
        out,
        "| Command | Form | Signature | Handler | Status | Crate | Verified | Uses |\n\
         |---|---|---|---|---|---|---|---:|"
    );
    let mut sorted: Vec<&Row> = rows.iter().collect();
    sorted.sort_by(|a, b| {
        a.overload
            .name
            .to_ascii_lowercase()
            .cmp(&b.overload.name.to_ascii_lowercase())
            .then_with(|| a.overload.form.as_str().cmp(b.overload.form.as_str()))
    });
    for r in sorted {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} | {} | {} |",
            md_escape(&r.overload.name),
            r.overload.form.as_str(),
            md_escape(&signature(&r.overload)),
            r.overload.handler,
            r.status.as_str(),
            r.crates.join(", "),
            md_escape(&r.verified),
            r.uses.iter().sum::<usize>()
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_sqf::{NullHost, Type, Value};

    const ENGINE: &str = "name\tkind\tleft_type\tright_type\treturn_type\thandler\tpriority\tcategory\n\
        count\tunary\t\tARRAY|STRING\tSCALAR\t0x1\t\tGeneral\n\
        count\tunary\t\tHASHMAP\tSCALAR\t0x2\t\tGeneral\n\
        get\tbinary\tHASHMAP\t?|NaN\tANY\t0x3\t4\tGeneral\n\
        time\tnular\t\t\tSCALAR\t0x4\t\tGeneral\n\
        fooBar\tunary\t\tOBJECT\tNOTHING\t0x5\t\tGeneral\n\
        interpolate\tbinary\tEXPRESSION\tEXPRESSION\tEXPRESSION\t0x6\t4\tSimple expression\n";

    fn crates() -> Vec<CrateCommands> {
        let mut core = Registry::<NullHost>::new(CommandTable::new());
        core.unary(
            "count",
            TypeSet::of(Type::Array),
            TypeSet::NUMBER,
            |_, _| Ok(Value::Number(0.0)),
        );
        core.binary(
            "get",
            TypeSet::of(Type::HashMap),
            TypeSet::ANYTHING,
            TypeSet::ANYTHING,
            |_, _, _| Ok(Value::Nothing),
        );
        let mut headless = Registry::<NullHost>::new(CommandTable::new());
        headless.nular("time", TypeSet::NUMBER, |_| Ok(Value::Number(0.0)));
        headless.unary(
            "count",
            TypeSet::of(Type::Array),
            TypeSet::NUMBER,
            |_, _| Ok(Value::Number(0.0)),
        );
        vec![
            CrateCommands::from_registry("a3-sqf", false, &core),
            CrateCommands::from_registry("headless", true, &headless),
        ]
    }

    #[test]
    fn rows_classify_overloads() {
        let engine = parse_engine_commands(ENGINE);
        assert_eq!(engine.len(), 5, "simple expressions are left out");
        let verified = parse_verified(
            "# name\tform\thandler\tstatus\tsource\tnote\n\
             get\tbinary\t*\tverified\tdecompiled 0x3\t\n",
        );
        let rows = ledger_rows(&engine, &crates(), &verified, None);
        let status: Vec<(&str, Status)> = rows
            .iter()
            .map(|r| (r.overload.handler.as_str(), r.status))
            .collect();
        assert_eq!(
            status,
            vec![
                ("0x1", Status::Partial),
                ("0x2", Status::Missing),
                ("0x3", Status::Verified),
                ("0x4", Status::Stub),
                ("0x5", Status::Missing),
            ]
        );
        assert_eq!(
            rows[0].crates,
            vec!["a3-sqf"],
            "a stand-in does not add to a real crate"
        );
        assert_eq!(rows[2].verified, "decompiled 0x3");
        let md = render(&rows, None);
        assert!(md.contains("| verified | 1 | 1 |"), "{md}");
        assert!(md.contains("| missing | 2 | 2 |"), "{md}");
        assert!(md.contains("| 1 | `count` | unary |"), "{md}");
    }
}
