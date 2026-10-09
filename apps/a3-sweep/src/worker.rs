//! The worker side: a process that loads the game once, then runs the scenarios it is sent on
//! standard input, one JSON [`Scenario`] per line, answering with [`Message`] lines on standard
//! output. A panic is caught and becomes that scenario's result; a crash (stack overflow, abort)
//! ends the process, which the supervisor sees and reports.

use std::cell::Cell;
use std::io::{BufRead, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::inventory::Scenario;
use crate::report::{ScenarioResult, Stage, Status};
use crate::runner::{Engine, RunOptions};
use crate::stubs::Stubs;

/// Marks the protocol's lines on standard output; anything else there is ignored.
pub const PREFIX: &str = "@@sweep ";

/// Stack of the thread scenarios run on: deeply recursive scripts need more than the default.
pub const STACK_SIZE: usize = 256 * 1024 * 1024;

/// What a worker tells the supervisor.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Message {
    /// The game is loaded; send scenarios.
    Ready { load_ms: f64, functions: usize },
    /// The current scenario entered a stage.
    Stage { stage: Stage },
    /// The current scenario is done.
    Result { result: Box<ScenarioResult> },
}

impl Message {
    /// The protocol line for this message.
    pub fn line(&self) -> String {
        format!(
            "{PREFIX}{}",
            serde_json::to_string(self).expect("messages serialise")
        )
    }

    /// The message on a protocol line, or `None` for any other output.
    pub fn parse(line: &str) -> Option<Message> {
        serde_json::from_str(line.strip_prefix(PREFIX)?).ok()
    }
}

/// The last panic's location and message, recorded by the hook [`install_panic_hook`] sets.
static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

/// Records each panic's message and location (and still prints it, as the default hook does).
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(non-string panic payload)".to_owned());
        let location = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        if let Ok(mut last) = LAST_PANIC.lock() {
            *last = Some(format!("{message}{location}"));
        }
        default(info);
    }));
}

/// Runs one scenario, turning a panic into a [`Status::Panic`] result.
pub fn run_caught(
    engine: &mut Engine,
    scenario: &Scenario,
    mut on_stage: impl FnMut(Stage),
) -> ScenarioResult {
    let stage = Cell::new(Stage::Queued);
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        stage.set(Stage::Game);
        on_stage(Stage::Game);
        match engine.runner(scenario.optional_mods) {
            Ok(runner) => runner.run(scenario, |s| {
                stage.set(s);
                on_stage(s);
            }),
            Err(e) => ScenarioResult::failed(
                scenario.clone(),
                Status::LoadFailed,
                Stage::Game,
                format!("{e:#}"),
            ),
        }
    }));
    match outcome {
        Ok(result) => result,
        Err(_) => {
            let message = LAST_PANIC
                .lock()
                .ok()
                .and_then(|mut last| last.take())
                .unwrap_or_else(|| "panic".to_owned());
            ScenarioResult::failed(scenario.clone(), Status::Panic, stage.get(), message)
        }
    }
}

/// The worker process: loads the game, says [`Message::Ready`], then serves scenarios until
/// standard input closes.
pub fn serve(game_dir: &Path, options: RunOptions, stubs: &Path) -> anyhow::Result<()> {
    install_panic_hook();
    let stubs = Arc::new(match Stubs::load(stubs) {
        Ok(stubs) => stubs,
        Err(e) => {
            eprintln!("warning: {e:#}: stubbed commands are not counted");
            Stubs::unknown()
        }
    });
    let mut engine = Engine::new(game_dir, options, stubs);
    // Most scenarios run on the base game; load it before saying ready.
    let runner = engine.runner(false)?;
    send(&Message::Ready {
        load_ms: runner.load_time.as_secs_f64() * 1000.0,
        functions: runner.functions,
    })?;
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let scenario: Scenario = serde_json::from_str(&line)?;
        let result = run_caught(&mut engine, &scenario, |stage| {
            let _ = send(&Message::Stage { stage });
        });
        send(&Message::Result {
            result: Box::new(result),
        })?;
    }
    Ok(())
}

fn send(message: &Message) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", message.line())?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_through_their_line_and_other_output_is_ignored() {
        let line = Message::Stage {
            stage: Stage::Simulate,
        }
        .line();
        assert!(line.starts_with(PREFIX));
        match Message::parse(&line) {
            Some(Message::Stage { stage }) => assert_eq!(stage, Stage::Simulate),
            other => panic!("{other:?}"),
        }
        assert!(Message::parse("some log output").is_none());
        assert!(Message::parse(&format!("{PREFIX}not json")).is_none());
    }
}
