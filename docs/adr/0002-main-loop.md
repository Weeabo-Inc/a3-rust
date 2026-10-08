---
status: accepted
---

# Frame-coupled main loop with variable dt, plus a fixed-step accumulator for physics

The main loop runs **one simulation update per rendered frame with a variable, clamped `dt`**,
as Real Virtuality does. A fixed-step accumulator runs alongside it: each frame reports how many
fixed steps (default 60 Hz) are due and an interpolation `alpha`, and subsystems that need a
stable integration step (rapier physics, later network ticks) consume those steps. The loop is
implemented by `a3_platform::FrameClock`; `a3_platform::run` drives it from the winit event loop
with `ControlFlow::Poll` and one redraw per iteration.

## What RV does

RV's simulation is frame-coupled: each frame the World advances every Entity by the frame's
`deltaT`, then renders. Evidence visible from script and observed behaviour:

- `time` advances by the frame delta; `diag_frameNo` counts frames; `diag_deltaTime` is the
  last frame's delta. There is no separate script-visible tick counter.
- The `EachFrame` mission event handler and `onEachFrame` run once per rendered frame, and
  scripts in them read positions that match what is drawn that frame (e.g. `drawIcon3D`).
- The scheduler gives scheduled scripts a time budget (about 3 ms) **per frame**; low FPS slows
  scheduled scripts, AI and simulation fidelity, which players observe as "low FPS = slow AI".
- `setAccTime` scales the simulation delta; pausing in single player stops `time`.
- PhysX (vehicles) sub-steps internally at a fixed rate inside the frame _(uncertain: exact
  rate and how RV distributes the remainder; to be confirmed by reverse engineering)_.

## Decision

- Top level: variable `dt` per frame, clamped to `max_frame_dt` (default 0.25 s, so a hitch
  does not teleport the world) and scaled by a time scale (`accTime`) or zeroed when paused.
  SQF, the World and Entity simulation see exactly one update per frame, matching RV.
- Fixed steps: an accumulator over the scaled `dt` yields `fixed_steps` of `fixed_dt` per frame
  (capped at `max_fixed_steps`, excess time dropped) and `alpha` for interpolating
  physics-owned transforms to the frame.
- Rendering happens after the update in the same frame; vsync or the present mode caps the rate.

## Considered options

- **Pure fixed tick for everything, render interpolated** (the common game-engine pattern):
  rejected as the top level. SQF semantics are frame-based (`EachFrame`, per-frame scheduler
  budget, `diag_frameNo`); scripts that read a position and draw at it would see the
  un-interpolated tick state; running scripts on a tick would change observable timing of
  unmodified missions. Kept for physics, where determinism and stability matter and nothing
  script-visible depends on the step size.
- **Pure variable dt including physics**: rejected. rapier (like PhysX) is only stable with a
  bounded step; vehicle behaviour would depend on frame rate.
- **Separate simulation thread**: deferred. SQF needs synchronous world access (ADR 0001); a
  threaded split can come later for rendering preparation without changing these semantics.

## Consequences

- Behaviour that depends on frame rate in RV (scheduler throughput, AI think rate) depends on it
  here too, which is what fidelity requires.
- Physics state must be interpolated with `alpha` for smooth rendering, or rendered at the last
  fixed step.
- Headless runs (CI smoke tests, dedicated server later) use the same `FrameClock` driven by
  their own timer instead of a window.
