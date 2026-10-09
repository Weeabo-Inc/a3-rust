# AI: groups, waypoints, targeting and the SQF commands around them

How Arma 3 2.22 drives AI groups, and what `a3-world` implements of it (issue #129). Addresses
are RVAs in `arma3_x64.exe` 2.22.0.154103 (Ghidra project `a3`), the form `sqf-commands.tsv`
lists them in; a `0x14…` address is the Ghidra VA (image base `0x140000000`). Implementation:
`crates/a3-world/src/ai/{mod.rs,waypoint.rs,target.rs}` (the AI itself) and
`crates/a3-world/src/script/ai.rs` (the commands). The frame this runs in is
`world-object-model.md`.

**Status.** High: the waypoint array's binary layout, the `addWaypoint`/`deleteWaypoint`
handlers, the delete re-index rule, and every handler address below (from `sqf-commands.tsv`).
Medium: the queue semantics a mission sees (`currentWaypoint`, index 0, re-indexing), which come
from the wiki and are consistent with the handlers. Low, and marked as ours: the steering, the
formation offsets and the knowledge constants — the engine's values were not traced.

## 1. Where the AI runs in the frame

Phase 5 of [`World::simulate`](../../crates/a3-world/src/sim/mod.rs), after physics and attached
positions, before deletion flushes and the time advance. The engine has the same phase:
`World::PerformAI(float)`, a `World` method named by lambda RTTI
(`World::PerformAI()::<lambda_1>`, RTTI entry `0x1c7c568` in `rtti-classes.tsv`), run after
`World::UpdateAttachedPositions` (`world-object-model.md` §5). What runs *inside* it is not
traced; everything below is our model.

- **Locality gates the tick.** Only a group whose `Locality` is local thinks here; a group owned
  by another machine is driven there, and its state would arrive over the network — no AI
  state is replicated yet. Locality gates the AI, not the script layer: a mission script may
  set waypoints and orders on any group on any machine.
- The tick takes `dt` of World time and reads no wall clock, so a mission replays the same at
  any frame rate.
- State lives on `Group::ai: GroupAi` (queue, modes, target knowledge) and `Man::ai: ManAi`
  (per-unit orders); the output each tick is the unit's `ManInput`
  (`docs/re/sim-man-movement.md`) — the same struct a human player's input fills.

## 2. The waypoint queue

### 2.1 The index model

A group holds a `Vec<Waypoint>` and one index into it (`WaypointQueue::current`). The number a
mission sees is the index **plus one**; there is an implicit waypoint 0 — the group's start
position, already completed — that is never stored:

| state | `currentWaypoint g` | `count waypoints g` |
|---|---|---|
| no waypoints | 0 | 1 (the implicit 0 only) |
| working on the first added | 1 | n + 1 |
| every waypoint done | n + 1 | n + 1 |

- `addWaypoint` with index `-1` (the default) appends; so does any index `< 1` or `>= count`, and
  a missing or non-numeric one. The returned value is the Waypoint array `[group, index]` with
  the **one-based** index of the new waypoint.
- A **Waypoint array is a value snapshot**, not a reference to a slot: a variable holding
  `[g, 1]` keeps reading `[g, 1]` after an insert moves what lives there. The engine's own
  examples write `[g, index]` afresh; commands must too.
- `waypoints g` returns n + 1 entries: the implicit `[g, 0]` first, then the added ones.
- `deleteWaypoint [g, i]` re-indexes the queue at once; `i == 0` (the implicit waypoint) does
  nothing, and deleting the waypoint a group is working on does not stop the group.
- `setCurrentWaypoint [g, i]` makes waypoint `i` active and applies its modes. `i == count`
  means "every waypoint is done"; `i == 0` names no stored waypoint and is ignored.
- `UNCHANGED` on a `setWaypoint*` mode means "no change": the field stores nothing and the mode
  is left alone when the waypoint becomes active (`docs/re/sqf-commands.tsv` has the command
  list; the wiki pages `addWaypoint` / `deleteWaypoint` / `currentWaypoint` have the index
  conventions).

### 2.2 The binary layout (high)

From the `addWaypoint` handler (`0x1408f61b0`, VA `0x1408f61b0`) and the `deleteWaypoint`
handler (`0x1408f6e80`):

| offset | contents |
|---|---|
| group + 0x2f0 | pointer to the waypoint array |
| group + 0x2f8 | element count |
| group + 0x300 | capacity |
| element | 0x138 (312) bytes |
| group + 0x48 | pointer to the group's unit array |
| group + 0x50 | unit count |

`addWaypoint` reads the index as a float and rounds it. It inserts (memmove up) only when
`0 <= index < count`; otherwise it appends. The queue's **current index** is not fixed up inline
by the insert path; `deleteWaypoint` does fix it up inline: after the memmove-down removal and
the `count--`,

```
if (deleted < current)  current -= 1;
else if (deleted == current)  { advance path }
```

where the current index is read from an object reached through virtual slot `+0x50` on the
group's **last** unit (the units array/count above). The delete rule matches our
`delete_waypoint`, including that the special advance path is what keeps a group alive when its
active waypoint is deleted.

Both handlers end with a virtual notification on the world singleton: `addWaypoint` calls
`(*(*world + 0x7b0))(world, group, index, wp)` with `world = FUN_140bc1ed0()` — a runtime
assigned singleton (`DAT_1421ccaa0`, zero statically). What that slot does was not resolved; it
is the network/mission notification path, most likely "a waypoint changed".

**Unverified.** Whether inserting **at** the current index shifts the current pointer in the
engine's insert notification. Our approximation: an insert before the active index shifts it up
(the active waypoint stays the same object); an insert at the active index becomes the active
one. Recorded again under §7.

## 3. The group tick

`perform_ai` walks every group; `tick_group` returns early for a group that is not local or has
no units. Order per tick:

1. **Targets** (`update_targets`, §5) — what the group sees, before it decides anything.
2. **Steering** — the active waypoint's orders (`active_orders`) are read once, then
   `steer_group` fills every unit's `ManInput`; a group with no active waypoint (queue done, or
   empty) uses `stand_group`.
3. **Arrival checks** — `check_orders` moves the queue on.

`Orders` is the tick's view of the active waypoint: its position, `effective_completion_radius()`
(the mission's radius, or `DEFAULT_COMPLETION_RADIUS = 3.0` when it set none), `timeout[1]` for
the timeout, and the two type predicates:

- `completes_on_arrival()`: HOLD, SENTRY, GUARD, SUPPORT, AND, OR are never finished by
  arriving, and neither are the vehicle waypoints (GETIN, GETOUT, LOAD, UNLOAD, TR UNLOAD, HOOK,
  UNHOOK, GETIN NEAREST) — until #124/#127 exist, a group sent to one waits on the spot.
- `needs_a_clear_area()`: SAD and DESTROY also need the group's contact list empty.

A waypoint completes when the **leader** is within the radius (flat distance) and the clear-area
condition holds, or when its timeout has elapsed. Completing marks the queue:

- `advance_waypoint` sets `current` to 0 after a CYCLE waypoint, else to `index + 1`, restarts
  the timeout clock, applies the new waypoint's modes (only the fields it set), and pushes
  `WorldEvent::WaypointCompleted { group, index }` for mission scripts and the future FSM
  runtime.
- `started` (World time) is the clock `timeout[1]` runs on; it is set when the queue is moved
  (`add_waypoint` on an empty queue, `set_current_waypoint`, `move_group`, advance).

`move group position` clears the queue and leaves **one** MOVE waypoint active at once; `move`
on a unit is the same call on his group. It is not the same as `doMove`.

## 4. Steering and formations

`ManInput { forward, strafe, turn, sprint, stance }`. For each unit, in order:

| unit | goal |
|---|---|
| any unit with `move_order` (doMove/commandMove/moveTo) | his own order — overrides the group |
| `stopped` (doStop) | none: he stands, out of the formation |
| `disabled` (enableAI/disableAI, not yet a script command) | untouched — the script drives him |
| leader | the active waypoint's position |
| follower | `leader.position + rotate_flat(formation.offset(index), leader.heading())` |

On the way: `walk_towards` turns towards the goal and walks while the heading error is under
`FULL_TURN_DEGREES = 90°` (otherwise turning on the spot — walking while turning sharply would
carry him the wrong way), and stops within `ARRIVE_RADIUS = 1.0` m. Turn input is proportional
below 90° of error and saturated at ±1.0 (`turn = clamp(error / 90, -1, 1)`). Heading is
`atan2(dx, dz)` degrees, as `Entity::heading` reports.

A unit with no goal and a group on the offensive (`Behaviour::Combat` or a `pursues()` combat
mode — RED or WHITE) turns to face the best-known contact instead of standing still.

**Speed modes.** LIMITED walks (`sprint` off); NORMAL and FULL both use the fastest move the
graph has, because the sprint moves do not exist yet (#124). This is a deviation, not a
finding.

**Formations** (`FORMATION_SPACING = 5.0` m): COLUMN/FILE one behind the other, STAG COLUMN
alternating half a step, WEDGE/VEE arrowhead/V, ECH LEFT/ECH RIGHT diagonals, LINE abreast,
DIAMOND slots ahead/right/left/behind the leader. The offsets are ours (low confidence in the
exact spacing; the engine's spacing depends on the men and their weapons). `CombatMode::Red`'s
`breaks_formation` flag exists but is not used for movement yet — a RED group still walks in
formation while it has a waypoint.

**Per-unit arrival.** A `doMove` order is done on arrival (`ARRIVE_RADIUS`, flat) and cleared:
the man walks back into his slot. `doStop` leaves him out of the formation until a waypoint
moves the group or `doFollow` puts him back in. The wiki's note that "doStop'ed units return to
formation if their leader's behaviour isn't COMBAT" is not modelled — `stopped` here is
absolute.

**No path.** Steering is straight at the goal through the collision world; nothing routes around
terrain or buildings yet. The navigation grid and planner exist (`docs/re/navigation.md`, #240)
but the AI does not plan over them (#243).

## 5. Target knowledge

Knowledge is the **group's**, 0..=4, and every unit of the group answers `knowsAbout` with it.
Constants live in `target.rs`; they are our reading of the engine, not traced values:

| constant | value | basis |
|---|---|---|
| `VIEW_RANGE` | 500 m | the mission's `viewDistance` in the engine's default setup |
| `EYE_HEIGHT` | 1.55 m | a standing man's eyes above his feet |
| `KNOWLEDGE_PER_SECOND` | 4.0 | a full 4 takes one second of looking |
| `FORGET_TIME` | 120 s | the engine's loss-of-sight reset (community: "knowsAbout drops after 2 minutes") |
| behaviour view scale | CARELESS 0.2, SAFE 0.6, AWARE/COMBAT 1.0, STEALTH 0.9 | ours: alertness scales the eye |

Per tick, for the group's first unit as the observer (`eye = position + EYE_HEIGHT`):

- Candidates are every **alive enemy of the group's side** within `VIEW_RANGE × view_scale`
  (flat). Own units and the target itself are excluded from the ray.
- Line of sight is `collision.visibility(eye, target, keys)`: a3-physics' view layer, so terrain,
  buildings and walls block and foliage does not (`docs/re/physics-collision.md`).
- A target never seen before is only noticed when it is in plain sight. Seen: knowledge rises by
  `4 × dt` to a cap of 4, and the last-seen position and time are updated. Out of sight: the
  knowledge and the old position are kept.
- The dead and the deleted are dropped at once; a contact unseen for `FORGET_TIME` is dropped
  (and not re-added by that same tick's update).

`reveal` semantics (as the mission sees them): revealing to a group sets each man's knowledge to
the given accuracy, or, without one, to the best knowledge any group of that side has — floored
at 1 when nobody knows. Revealing never lowers knowledge. `forgetTarget` drops one contact
immediately. `knowsAbout` answers 4 for a member of one's own group and for oneself, the stored
knowledge for an enemy, and the best over the side's groups when asked of a side.

Not modelled: per-unit knowledge (the engine keeps it per unit and shares within the group),
weapon-range gating, `nearTargets` / `targetsQuery`, and any effect of knowledge on firing
(#127).

## 6. The SQF command surface

Registered by `crates/a3-world/src/script/ai.rs` (53 names incl. overloads). Handler RVAs from
`docs/re/sqf-commands.tsv`; `0x140…` = RVA + image base.

| command | form | handler | notes |
|---|---|---|---|
| `addWaypoint` | `group addWaypoint [center, radius, index, name]` -> `[group, i]` | 0x8f61b0 | radius ignored (§7); index `< 1`/absent/over-range appends |
| `deleteWaypoint` | `deleteWaypoint [group, i]` | 0x8f6e80 | `i >= 1`; re-indexes; `0` and missing are no-ops |
| `setCurrentWaypoint` | `left setCurrentWaypoint [group, i]` | 0x8f9650 | `i` one-based; `i == count` = all done |
| `currentWaypoint` | `currentWaypoint g` | 0x8f7790 | 0 with no waypoints, else 1-based, n+1 when done |
| `waypoints` | `waypoints g` | 0x190d20 | n+1 entries, implicit `[g, 0]` first |
| `move` | `grp move [x, y, z]` / `obj move pos` | 0x191210 | clears the queue, one active MOVE waypoint |
| `setWaypointType` / `waypointType` | | 0x8fb2a0 / 0x8f8dc0 | 24 type names, case-insensitive |
| `setWaypointPosition` / `setWPPos` / `waypointPosition` | | 0x8fa710 / 0x8fa710 / 0x8fc010 | the engine takes `[center, radius]`; the radius element is ignored (§7) |
| `setWaypointBehaviour` / `waypointBehaviour` | | 0x8f9080 / 0x8f7330 | UNCHANGED = None |
| `setWaypointCombatMode` / `waypointCombatMode` | | 0x8f92c0 / 0x8f74e0 | |
| `setWaypointSpeed` / `waypointSpeed` | | 0x8fac00 / 0x8f8620 | |
| `setWaypointFormation` / `waypointFormation` | | 0x8f9ae0 / 0x8f7a30 | |
| `setWaypointCompletionRadius` / `waypointCompletionRadius` | | 0x8f9500 / 0x8f7690 | |
| `setWaypointDescription` / `waypointDescription` | | 0x8f9790 / 0x8f77e0 | |
| `setWaypointName` / `waypointName` | | 0x8fa550 / 0x8f81c0 | |
| `setWaypointVisible` / `waypointVisible` | | 0x8fb500 / 0x8f8f70 | |
| `setWaypointTimeout` / `waypointTimeout` | | 0x8fb080 / 0x8f8a90 | `[min, mid, max]`; mid drives the timer |
| `setWaypointStatements` / `waypointStatements` | | 0x8fae40 / 0x8f87d0 | stored as text, never run (§7) |
| `setWaypointScript` / `waypointScript` | | 0x8faa80 / 0x8f8310 | stored, never run (§7) |
| `setWaypointHousePosition` / `waypointHousePosition` | | 0x8f9d20 / 0x8f7be0 | stored only |
| `setWaypointLoiterRadius` / `waypointLoiterRadius` | | 0x8fa070 / 0x8f7ec0 | stored only |
| `setWaypointLoiterType` / `waypointLoiterType` | | 0x8fa260 / 0x8f8020 | stored only |
| `setBehaviour` / `behaviour` | group or unit | 0x191810 / 0x525e10 | a unit speaks for his group |
| `setCombatMode` / `combatMode` | | 0x191a20 / 0x190840 | |
| `setSpeedMode` / `speedMode` | | 0x192270 / 0x1924f0 | |
| `setFormation` / `formation` | | 0x191de0 / 0x190b40 | |
| `doMove`, `commandMove`, `moveTo` | `unit(s) cmd pos` | 0x5690d0 / 0x568340 / 0x47de90 | left may be an array of units |
| `doStop` | `doStop unit(s)` | 0x5690f0 | postfix works with an object (`a doStop`); the engine's own array examples use the prefix form `doStop [a, b]` |
| `stopped` | `stopped unit` | 0x541a10 | |
| `doFollow` | `unit(s) doFollow lead` | 0x569090 | clears his order, back into the formation |
| `knowsAbout` | `who knowsAbout target` | 0x1c8640 / 0x1cbe50 | object/group/side on the left |
| `reveal` | `who reveal target` / `who reveal [target, accuracy]` | 0x1c86f0 | a unit reveals to his group |
| `forgetTarget` | `who forgetTarget target` | 0x1c82d0 | drops the contact at once |

Engine commands in this area we have **not** registered: `copyWaypoints` (0x8fbec0),
`enableAI`/`disableAI` (0x527420/0x526960), `setUnitPos` (0x53fc20), `commandStop` (0x568360),
`stop` (0x541980), `unitReady` (0x569170), `waypointTimeoutCurrent` (0x8f8d20),
`lockWp` (0x1911a0), the waypoint attachment commands (`waypointAttachVehicle` etc.),
`forceSpeed` (0x569ed0), `nearTargets` (0x1cb6a0).

## 7. Deviations from the engine, and what is left out

- **The unit FSMs do not exist.** #129 lists `formationFSM` / danger FSM through the SQF/FSM
  runtime; this slice implements the group state machine in Rust instead. Waypoint statements
  and scripts are stored on the waypoint and never executed; there is no FSM runtime to run
  them in (#242).
- **`addWaypoint`'s radius is ignored** and the waypoint lands on the centre: random placement
  needs a seeded RNG the engine does not have yet. The same for the middle element of
  `setWaypointPosition`'s `[center, radius]`.
- **Waypoint timeouts use `timeout[1]`** (and completions are exact, not randomised between
  min and max) — again no random source.
- **Insert-at-current is an approximation.** The binary's insert path does not fix the current
  index up inline (the delete path does), and its trailing notification (world vtable `+0x7b0`)
  could not be resolved, so whether the engine shifts the pointer when a waypoint is inserted at
  the current index is unverified. Ours: insert before the active index shifts it up; insert at
  the active index becomes active.
- **Vehicle waypoints wait.** GETIN/GETOUT/LOAD/UNLOAD/TR UNLOAD/HOOK/UNHOOK/GETIN NEAREST never
  complete (no vehicle boarding yet, #124/#127); LOITER's radius and type and
  `setWaypointHousePosition` are stored and unused.
- **Speed modes collapse to walk / fastest** (#124), and combat mode does not yet break
  formation for movement (see §4).
- **Knowledge is per group, not per unit**; no weapon-range gate, no firing effect (#127), no
  `nearTargets`/`targetsQuery`.
- **Steering is straight-line**; no path through the navigation grid (#243).
- **Locality gates only the AI tick.** No network messages are produced or consumed for any of
  this; the network-facing World operations do not carry AI state, so a client-side mission
  setting waypoints changes its local copy only (#244).
- `Behaviour::view_scale`, `FORMATION_SPACING`, the knowledge constants and `ARRIVE_RADIUS` are
  our values, chosen to be plausible, not traced from the binary.
