---
status: accepted
---

# SQF VM: frame-stack interpreter, generic host, open command registry

The SQF VM (`crates/a3-sqf`) compiles scripts to a postfix instruction stream and runs them on an
explicit stack of frames instead of recursing in Rust. Code frames execute instructions and own
one scope of private variables. Native frames are `Continuation` objects: the remaining work of a
command that runs code, such as a `forEach` loop waiting for its body, or `try` waiting for a
`throw`. The VM is generic over a `Host` trait for engine services. Commands live in an open
`Registry<H>` keyed by the ids of a data-driven `CommandTable`. Values use `Rc`/`RefCell` and are
not `Send`.

## Reasoning

- **Suspension anywhere.** Scheduled scripts can `sleep` or `waitUntil` inside `forEach` inside
  `call` inside `if`. A recursive interpreter would need stackful coroutines or threads to pause
  there. With frames on the heap, suspending a script is just stopping the loop: the scheduler
  keeps the script's frame stack and resumes it later. The same mechanism handles the scheduler's
  per-frame time budget, which can cut a script off between any two instructions.
- **Non-local exits are frame unwinding.** `exitWith`, `breakOut`/`breakTo`, `throw`/`catch`,
  `break`/`continue` pop frames until they reach their target (a code frame, a named scope, a
  `try`, a loop). This keeps the engine's semantics (exitWith inside a loop body ends the loop)
  without Rust panics or `Result` plumbing through every command.
- **Generic host over trait objects.** `Vm<H: Host>` and `Registry<H>` let world, UI and config
  crates add commands with static access to their own state: `a3-world` defines
  `trait WorldHost: Host { fn world(&mut self) -> &mut World; }` and registers
  `fn(&mut Ctx<H>, Value) -> ...` for `H: WorldHost`. No downcasting is needed, and the final
  engine binary picks the concrete `H`.
- **Command names come from data.** The parser must know every command's forms (nular, unary,
  binary) and binary precedence to parse SQF at all. `data/commands.tsv` lists every command
  signature, implemented or not. An unimplemented command still parses and raises
  "Unimplemented command" only when it runs. Registering an implementation also declares its
  signature, so host crates can add commands the table does not know.
- **Single-threaded values.** The engine runs SQF on the simulation thread. `Rc<RefCell<..>>`
  gives the shared, in-place-mutable array and hash map semantics cheaply. A command that must
  run SQF synchronously (an event handler fired by `setDamage`) uses `Ctx::call_unscheduled`,
  which runs a nested unscheduled script against the same globals.

## Considered options

- **Recursive tree-walking interpreter**: simpler and slightly faster per call. Rejected because
  scheduled suspension would need a coroutine library or one OS thread per script, and the
  per-frame budget could not interrupt deep call chains.
- **`dyn Any` host with downcasting in commands**: rejected. Every command would pay a runtime
  check, and type errors in host wiring would only show at runtime.
- **`Arc`/`Mutex` values**: rejected. SQF does not need cross-thread value sharing, and atomic
  reference counts and locks on every array access cost time in the hottest paths.

## Consequences

- Commands that run code are written as continuations (state machines), not straight-line
  loops. `Loop`, `WhileLoop`, `ForRange`, `TryCatch` and the others in `commands/control.rs` are
  the patterns to copy.
- `Vm` and values cannot be moved across threads. Background work (loading, preprocessing) hands
  over source text, not compiled `Code`.
- The interner for variable names (`Sym`) is process-wide and never frees names.
