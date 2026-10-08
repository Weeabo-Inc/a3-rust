# a3-rust

A reimplementation of the Arma 3 engine (Bohemia Interactive's Real Virtuality 4, targeting game
build **2.22.0.154103**) in Rust.

The goal is an engine that loads an unmodified Arma 3 installation — its PBOs, configs, models,
textures, terrains and scripts — and runs it: first the data formats, then the SQF scripting VM,
then rendering, simulation, UI, and finally multiplayer. See [docs/ROADMAP.md](docs/ROADMAP.md).

## Legal note

- This repository is **private and unlicensed**. No rights are granted to anyone outside the
  project.
- **No game data is in this repository.** The engine loads the user's own, legally obtained Arma 3
  installation at runtime, located via the `A3_ROOT` environment variable or the `--game-dir`
  command-line flag.
- Reverse engineering of the original executable is used **only to understand behaviour** (file
  formats, data layouts, algorithms, script command semantics). Findings are written up as notes
  in [`docs/re/`](docs/re/) and implemented as idiomatic Rust. Decompiler output is never pasted
  into the codebase.

## Layout

| Path            | Contents                                                            |
| --------------- | ------------------------------------------------------------------- |
| `crates/a3-*`   | Library crates, one per engine area (`a3-core`, later `a3-pbo`, ...) |
| `apps/*`        | Binaries (`a3-tools` CLI, `arma3` game client; later the server)   |
| `docs/adr/`     | Architecture decision records                                       |
| `docs/re/`      | Reverse-engineering notes (addresses, struct layouts, behaviours)   |
| `docs/ROADMAP.md` | Phases and exit criteria                                          |
| `CONTEXT.md`    | Domain glossary (PBO, rapify, ODOL, Locality, ...)                  |
| `AGENTS.md`     | Working conventions for contributors and coding agents              |

## Build and test

Requires a stable Rust toolchain (pinned by `rust-toolchain.toml`).

```sh
cargo build --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Tests that need real game data read `A3_ROOT` and skip when it is unset, so the suite passes
without a game install. To run them:

```sh
# PowerShell
$env:A3_ROOT = "C:\path\to\Arma 3"; cargo test --workspace
# bash
A3_ROOT="/path/to/Arma 3" cargo test --workspace
```

Optional: set `ARMA_WIKI` to an offline Arma wiki database (default `P:\ArmaWiki`) for command
signatures and locality; see "Reference sources" in [AGENTS.md](AGENTS.md).

Try the CLI:

```sh
cargo run -p a3-tools -- version
```

Run the client (free-fly debug camera; WASD/Q/Z move, Shift/Ctrl faster, click to mouse-look,
Tab releases the mouse, Esc quits):

```sh
cargo run -p arma3 -- --windowed --width 1600 --height 900
# Render N frames offscreen and save the last as PNG (for checking rendering changes):
cargo run -p arma3 -- --screenshot .work/shot.png --frames 10
# Main loop without window or GPU (smoke test):
cargo run -p arma3 -- --headless --frames 120
```

Windows is the primary development platform; CI also builds and tests on Linux.
