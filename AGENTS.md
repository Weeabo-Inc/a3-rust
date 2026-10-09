# AGENTS.md

a3-rust reimplements the Arma 3 engine (Real Virtuality 4, game build 2.22.0.154103) in Rust. It
loads the user's own game installation at runtime. Many agents work on this repo in parallel; these
conventions keep their work mergeable.

Domain vocabulary: read `CONTEXT.md` before naming types, modules or functions. Use its terms
exactly. Architecture decisions: `docs/adr/`. Phases and scope: `docs/ROADMAP.md`.

## Hard guardrails

- Commit only original work. Files from the game install (`oirignal/`), RE tool outputs
  (`.resources/`, `.work/`, Ghidra/IDA databases) and decompiler output stay out of git. Write
  behaviour as idiomatic Rust from your understanding of it.
- Push to `main` only through a squash-merged PR with green CI.

## Workflow: one issue, one branch, one PR

1. Work from a GitHub issue. If none exists for your unit of work, create one.
2. Branch from up-to-date `main`: `<issue-number>-<slug>`, e.g. `12-pbo-header-reader`.
3. Build test-first (see Testing). Commit in small steps.
4. Before every push, all three commands pass locally:
   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
5. Open the PR with `gh pr create`. The body contains `Closes #N` and ends with
   `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
6. Done when CI is green on both ubuntu and windows jobs and the PR is squash-merged
   (`gh pr merge --squash`). Merge when CI is green and the PR is mergeable; rebase only on
   conflicts; fix semantic breakage on main forward.

Every commit message ends with the trailer:

```
Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

### The gate is evidence only if it ran to the end

Step 4's three commands must run **to completion**, judged by their exit code. A gate that was
truncated by the way its output was piped is not a gate, and it has already cost this repo a red
CI run on a branch that looked locally green.

In PowerShell, `Select-Object -First N` stops the upstream pipeline once it has N objects:

```powershell
# WRONG: cargo is killed after the first 20 matches, so the remaining test
# binaries never run and $LASTEXITCODE means nothing.
cargo test --workspace 2>&1 | Select-String 'FAILED' | Select-Object -First 20
```

The same applies to anything that can stop consuming its input early. Run the command in full and
read the exit code, then filter the log file:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace 2>&1 | Out-File -Encoding utf8 .work/gate-test.log; "exit=$LASTEXITCODE"
```

`Select-Object -Last N` is safe: it consumes the whole stream. When CI disagrees with a local
green, suspect the pipeline before the platform.

## Testing

Use the `mattpocock-skills:tdd` skill: red, green, refactor.

- **Synthetic fixtures first.** Build the bytes of a PBO, rapified config, PAA, etc. inside the
  test so the test runs everywhere. Small hand-made fixture files go under the crate's
  `tests/fixtures/`.
- **Real game data** tests read the `A3_ROOT` env var and return early with a printed note when it
  is unset, so CI stays green without game data:
  ```rust
  let Some(root) = std::env::var_os("A3_ROOT") else {
      eprintln!("skipping: A3_ROOT not set");
      return;
  };
  ```
  Locally, `A3_ROOT=P:\a3-rust\oirignal`.
- `proptest` for round-trips (read/write, rapify/derap); `insta` for snapshot output of parsers.

## Build economy

Rapid iteration is the project's priority: writing systems outranks waiting on rustc. Scope every
build to the crates you touch.

- While iterating: `cargo check -p <crate> --all-targets` after each edit, `cargo test -p <crate>`
  for its tests. Reach for `--workspace` mid-iteration only when a change ripples into many crates.
- One full gate run, immediately before each push (Workflow step 4); CI guards `main` from there.

## Code conventions

- Crates: libraries are `crates/a3-<area>` (`a3-pbo`, `a3-config`, `a3-sqf`); binaries are
  `apps/<name>`. Add new dependencies to `[workspace.dependencies]` and reference them with
  `.workspace = true`. Each crate opts into `[lints] workspace = true`.
- Errors: `thiserror` enums in libraries; `anyhow` only in `apps/`.
- `unsafe` only when necessary, each block preceded by a `// SAFETY:` comment stating the invariant.
- Windows is the primary dev platform; code builds and passes tests on Linux too. Use `Path`/
  `PathBuf`, and treat in-game paths (backslash-separated, case-insensitive) as their own type,
  separate from OS paths.
- Game data location comes from `A3_ROOT` or a `--game-dir` flag; hard-code no install paths.

## Disk hygiene

Every git worktree has its own `target/`, and many worktrees exist at once. Debug builds are
already slimmed in the root `Cargo.toml` (line-tables-only debuginfo, none for dependencies); keep
those settings.

- Run `cargo clean` in your worktree after your last PR merges, or when `target/` exceeds ~8 GB.
- Never delete another agent's worktree or `target/`.
- Keep scratch output in `.work/` small; delete big intermediates when done.

## Reverse engineering

RE tools live in `.resources/` (Ghidra, IDA Free); scratch output goes in `.work/`. Both are
ignored by git.

`docs/re/` is the bridge from reverse engineering to implementation. Record every finding there
as Markdown, one file per topic (`docs/re/pbo.md`, `docs/re/sqf-commands.md`): function
addresses, struct layouts with offsets and types, observed behaviour and edge cases, and how
sure you are. An implementer reads `docs/re/` and writes Rust from it, without opening the
decompiler.

## Reference sources

Consult in this order; the binary outranks every other source when they disagree.

1. **Ground truth**: `docs/re/` (our findings), then the decompiled binary through the RE tooling
   (`python tools/re/mcp_call.py ...`, see `docs/re/TOOLING.md`).
2. **Offline Arma wiki** (optional, local machine only): `P:\ArmaWiki`, or `$ARMA_WIKI` when set.
   Check it exists first. Reach for it instead of the live wiki, which blocks automated access.
   Built 2026-08-21 from the BI community wiki: 2,654 commands and 2,062 functions (typed
   signatures, examples, locality), 1,657 guide pages, HEMTT docs.
   - CLI: `python -I P:\ArmaWiki\arma3db.py show <name> | search "<text>" | list --group <g> --arma3 | stats`
   - SQL on `arma3.sqlite`, e.g. `SELECT name FROM commands WHERE is_arma3=1 AND effect_locality LIKE '%global%'`
   - Markdown: `export/commands/*.md`, `export/docs.json`
   This path is a doc pointer for local dev; code takes install paths from flags or env (see Code conventions).
3. **Community tools** (HEMTT, armake2, bis-file-formats): read them as format documentation, and
   write our own implementation. Their code stays out of this repo.

## Domain docs

- New or sharpened domain term: update `CONTEXT.md` (use the `mattpocock-skills:domain-modeling`
  skill and follow its format).
- Hard-to-reverse decision with real alternatives: add an ADR in `docs/adr/`, numbered
  sequentially.
