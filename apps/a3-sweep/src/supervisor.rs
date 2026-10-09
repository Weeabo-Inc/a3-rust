//! The supervisor side: runs scenarios on worker processes, so a scenario that crashes the
//! process (stack overflow, abort) or hangs is recorded and the sweep goes on.
//!
//! Each worker loads the game once and then takes one scenario at a time. The supervisor hands
//! a free worker a scenario on the world it ran last when there is one (so its terrain stays
//! cached), kills a worker whose scenario exceeds the hard time limit, and starts a replacement
//! for every worker that died or was killed.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::inventory::Scenario;
use crate::report::{ScenarioResult, Stage, Status};
use crate::worker::Message;

/// How the supervisor runs its workers.
#[derive(Debug, Clone)]
pub struct SupervisorOptions {
    /// Worker processes.
    pub jobs: usize,
    /// Wall-clock time a worker may spend on one scenario before it is killed.
    pub hard_timeout: Duration,
    /// Arguments that start this executable as a worker.
    pub worker_args: Vec<OsString>,
}

/// Lines of a worker's standard error kept for a crash report.
const STDERR_TAIL: usize = 40;

/// Consecutive workers that may die before saying ready before the sweep gives up.
const MAX_STARTUP_FAILURES: usize = 3;

enum Event {
    Message(Message),
    Closed,
}

struct Worker {
    generation: u64,
    child: Child,
    stdin: Option<ChildStdin>,
    ready: bool,
    /// The scenario it runs, since when, and its last stage.
    busy: Option<(Scenario, Instant, Stage)>,
    /// Whether the game it has loaded includes the optional DLC, and the world it ran last.
    last: (bool, String),
    stderr: Arc<Mutex<VecDeque<String>>>,
}

/// Runs every scenario on `options.jobs` worker processes. `on_result` hears each result as it
/// arrives. Results come back in completion order.
pub fn run(
    scenarios: Vec<Scenario>,
    options: &SupervisorOptions,
    mut on_result: impl FnMut(&ScenarioResult),
) -> anyhow::Result<Vec<ScenarioResult>> {
    let total = scenarios.len();
    let mut queue: VecDeque<Scenario> = scenarios.into();
    let mut results = Vec::with_capacity(total);
    let (tx, rx) = channel::<(usize, u64, Event)>();
    let jobs = options.jobs.clamp(1, total.max(1));
    let mut workers: Vec<Worker> = (0..jobs)
        .map(|id| spawn(id, 0, options, &tx))
        .collect::<anyhow::Result<_>>()?;
    let mut startup_failures = 0;

    while results.len() < total {
        dispatch(&mut workers, &mut queue);
        let event = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => anyhow::bail!("all workers are gone"),
        };
        if let Some((id, generation, event)) = event
            && workers[id].generation == generation
        {
            match event {
                Event::Message(Message::Ready { .. }) => {
                    workers[id].ready = true;
                    startup_failures = 0;
                }
                Event::Message(Message::Stage { stage }) => {
                    if let Some(busy) = &mut workers[id].busy {
                        busy.2 = stage;
                    }
                }
                Event::Message(Message::Result { result }) => {
                    let worker = &mut workers[id];
                    if let Some((scenario, _, _)) = worker.busy.take() {
                        worker.last = (scenario.optional_mods, scenario.world.clone());
                    }
                    on_result(&result);
                    results.push(*result);
                }
                Event::Closed => {
                    let worker = &mut workers[id];
                    let status = worker.child.wait().ok();
                    // Let the standard error reader catch up with the last lines.
                    std::thread::sleep(Duration::from_millis(200));
                    if let Some((scenario, _, stage)) = worker.busy.take() {
                        let failure = format!(
                            "worker exited ({}): {}",
                            status.map_or("unknown status".to_owned(), |s| s.to_string()),
                            tail(&worker.stderr, 6)
                        );
                        let result =
                            ScenarioResult::failed(scenario, Status::Crash, stage, failure);
                        on_result(&result);
                        results.push(result);
                    } else if !worker.ready {
                        startup_failures += 1;
                        if startup_failures >= MAX_STARTUP_FAILURES {
                            anyhow::bail!(
                                "workers die while loading the game: {}",
                                tail(&worker.stderr, 20)
                            );
                        }
                    }
                    if results.len() < total {
                        workers[id] = spawn(id, generation + 1, options, &tx)?;
                    }
                }
            }
        }
        // Hung workers.
        for (id, worker) in workers.iter_mut().enumerate() {
            let overdue = worker
                .busy
                .as_ref()
                .is_some_and(|(_, since, _)| since.elapsed() > options.hard_timeout);
            if overdue {
                let _ = worker.child.kill();
                let _ = worker.child.wait();
                let (scenario, since, stage) = worker.busy.take().expect("overdue means busy");
                let failure = format!(
                    "killed after {:.0} s in stage {stage:?}",
                    since.elapsed().as_secs_f64()
                );
                let result = ScenarioResult::failed(scenario, Status::Timeout, stage, failure);
                on_result(&result);
                results.push(result);
                *worker = spawn(id, worker.generation + 1, options, &tx)?;
            }
        }
    }
    for worker in &mut workers {
        worker.stdin = None;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    for worker in &mut workers {
        while Instant::now() < deadline {
            if let Ok(Some(_)) = worker.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = worker.child.kill();
        let _ = worker.child.wait();
    }
    drain(&rx);
    Ok(results)
}

/// Hands each idle, ready worker a scenario, preferring one on the world it ran last.
fn dispatch(workers: &mut [Worker], queue: &mut VecDeque<Scenario>) {
    for worker in workers.iter_mut() {
        if !worker.ready || worker.busy.is_some() || queue.is_empty() {
            continue;
        }
        // The same game and terrain, then the same game (switching reloads it), then any.
        let (optional, world) = &worker.last;
        let index = queue
            .iter()
            .position(|s| s.optional_mods == *optional && s.world == *world)
            .or_else(|| queue.iter().position(|s| s.optional_mods == *optional))
            .unwrap_or(0);
        let scenario = queue.remove(index).expect("index is in range");
        let line = serde_json::to_string(&scenario).expect("scenarios serialise");
        let sent = worker.stdin.as_mut().is_some_and(|stdin| {
            writeln!(stdin, "{line}")
                .and_then(|_| stdin.flush())
                .is_ok()
        });
        // A failed write means the worker died; its `Closed` event reports the scenario.
        let _ = sent;
        worker.busy = Some((scenario, Instant::now(), Stage::Queued));
    }
}

fn spawn(
    id: usize,
    generation: u64,
    options: &SupervisorOptions,
    tx: &Sender<(usize, u64, Event)>,
) -> anyhow::Result<Worker> {
    let exe = std::env::current_exe()?;
    let mut child = Command::new(exe)
        .args(&options.worker_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    let stdin = child.stdin.take();
    let tx = tx.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(message) = Message::parse(&line)
                && tx.send((id, generation, Event::Message(message))).is_err()
            {
                return;
            }
        }
        let _ = tx.send((id, generation, Event::Closed));
    });
    let tail_lines = Arc::new(Mutex::new(VecDeque::new()));
    let sink = tail_lines.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            if let Ok(mut tail) = sink.lock() {
                if tail.len() == STDERR_TAIL {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        }
    });
    Ok(Worker {
        generation,
        child,
        stdin,
        ready: false,
        busy: None,
        last: (false, String::new()),
        stderr: tail_lines,
    })
}

/// The last `n` lines a worker wrote to standard error, joined.
fn tail(lines: &Arc<Mutex<VecDeque<String>>>, n: usize) -> String {
    let Ok(lines) = lines.lock() else {
        return String::new();
    };
    let skip = lines.len().saturating_sub(n);
    lines
        .iter()
        .skip(skip)
        .cloned()
        .collect::<Vec<_>>()
        .join(" | ")
}

fn drain(rx: &Receiver<(usize, u64, Event)>) {
    while rx.try_recv().is_ok() {}
}
