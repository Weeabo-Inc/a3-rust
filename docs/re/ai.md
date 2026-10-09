# AI: groups, waypoints, targeting and the SQF commands around them

How Arma 3 2.22 drives AI groups, and what `a3-world` implements of it (issue #129). Addresses
are RVAs in `arma3_x64.exe` 2.22.0.154103 (Ghidra project `a3`), the form `sqf-commands.tsv`
lists them in; a `0x14…` address is the Ghidra VA (image base `0x140000000`). Implementation:
`crates/a3-world/src/ai/{mod.rs,waypoint.rs,target.rs}` (the AI itself) and
`crates/a3-world/src/script/ai.rs` (the commands). The frame this runs in is
`world-object-model.md`.

**Status.** High: the waypoint array's binary layout, the `addWaypoint`/`deleteWaypoint`
handlers, the delete re-index rule, every handler address below (from `sqf-commands.tsv`), the
formation table and slot computation, the leader's speed control and the waypoint arrival rule
(`docs/re/ai-formation.md`), and the FSM interpreters (`docs/re/ai-fsm.md`). Medium: the queue
semantics a mission sees (`currentWaypoint`, index 0, re-indexing), which come from the wiki and
are consistent with the handlers. Low, and marked as ours: the gait choice, the path following
and the knowledge constants of §5 (the engine's model is in `docs/re/ai-detection.md`).

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

From the `addWaypoint` handler (RVA `0x8f61b0`, VA `0x1408f61b0`) and the `deleteWaypoint`
handler (RVA `0x8f6e80`, VA `0x1408f6e80`):

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

1. **Targets** (`update_targets`, §5): what the group sees, before it decides anything.
2. **Turn** (`turn_to_waypoint`): the first tick on a new active waypoint turns the formation
   towards it and drops the leader's path, so he plans to it (the engine's `Turn` state).
3. **Formation slots** (`formation_positions`) and the **leader's speed** (`leader_speed`).
4. **Each unit**: his formation FSM (`think_unit`, `docs/re/ai-fsm.md` §3), then his steering
   (`steer_unit`), then his trail point (`record_trail`).
5. **Waypoint** (`check_waypoint`): arrival, the type's wait, the countdown.

The engine's waypoint loop and what we implement of it: `docs/re/ai-formation.md` §3 and §8. In
short: the AI leader arrives when he has walked his path to the waypoint or is within
`max(completionRadius, precision)` of it in 3-D (the default radius is 0 and a man's precision
1 m); then the type's condition must hold (`Completion`: MOVE and most types none, HOLD/GUARD/
SUPPORT/vehicle types never, SENTRY an identified enemy, SAD/DESTROY no contacts left, DISMISS
COMBAT); then the group waits `Rand_MinMidMax(timeout)` seconds (a random time between min and
max with median mid, from the World's seeded `EngineRng`); then the waypoint is done:
`advance_waypoint` moves to `index + 1` (0 after a CYCLE), applies the next waypoint's modes and
pushes `WorldEvent::WaypointCompleted { group, index }`.

`move group position` clears the queue and leaves **one** MOVE waypoint active at once; `move`
on a unit is the same call on his group. It is not the same as `doMove`.

## 4. Formations and movement

Detail and confidence: `docs/re/ai-formation.md` (§1 formations, §2 followers, §8 what we do).

- **Slots** come from `cfgFormations` read by position (COLUMN, STAG COLUMN, WEDGE, ECH LEFT,
  ECH RIGHT, VEE, LINE, DIAMOND, FILE): fixed entries, then a repeating pattern, each slot placed
  from a reference slot by the average `formationX`/`formationZ` of the two units' types (5 m for
  men). Slot 0 is the first unit in ID order; positions are relative to the leader's slot,
  turned by the **formation direction** (reset to the leader's facing by `setFormation`, set by
  `setFormDir`, and by us towards each waypoint the group turns to).
- **The leader's speed**: `formationCoef` slews at 0.1/s within 0.1..1.5 towards the speed that
  lets the follower furthest behind his slot catch up; LIMITED caps him at
  `maxSpeed x limitedSpeedCoef` (not in COMBAT), FULL lets him go at 1.5x `maxSpeed`.
- **Followers** in AWARE and above steer to their slots; in CARELESS and SAFE they walk in file
  on the trail of the man ahead.
- **Pace** (ours, the engine's gait choice is not traced): stand below 0.3 m/s of speed cap, walk
  below 3 m/s, run above; CARELESS and SAFE walk; a follower far behind his slot runs;
  `forceSpeed` caps it.
- **Stance**: `setUnitPos` (the script's) beats the formation FSM's own request
  (`setUnitPosWeak`); AUTO leaves the stance to the move graph.
- **Paths**: the leader and a unit with `doMove` plan on the World's `a3-nav` navigator
  (`World::set_navigator`; a straight line without one) and walk point to point.
- **Per-unit orders**: `doMove` is done within the unit's `precision` and cleared, and the man
  walks back into his slot. `doStop` leaves him out of the formation until a waypoint moves the
  group or `doFollow` puts him back in. `disableAI` "MOVE", "PATH" or "ANIM" leaves his input to
  the script.

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

Unit movement and features (`crates/a3-world/src/script/ai_unit.rs`):

| command | form | handler | notes |
|---|---|---|---|
| `setUnitPos` / `setUnitPosWeak` / `unitPos` | `unit setUnitPos "UP"` | 0x53fc20 / 0x53fd50 / 0x533390 | the script's stance beats the AI's ("weak") one |
| `forceSpeed` | `unit forceSpeed mps` | 0x569ed0 | negative removes the cap |
| `disableAI` / `enableAI` / `checkAIFeature` | `unit disableAI "FSM"` | 0x526960 / 0x527420 / 0x481de0, 0x481cc0 | `FSM` and `COVER` act; the global form is always true |
| `setFormDir` / `formationDirection` | `group setFormDir deg` | 0x191d00 / 0x192bf0 | |
| `formationPosition` | `formationPosition unit` | 0x56ae60 | the slot, above the terrain |
| `formationLeader` / `isFormationLeader` | | 0x56aba0 / 0x56cd00 | the group leader |
| `unitReady` | `unitReady unit(s)` | 0x569170 | false for a leader with a move to make, or a unit with his own order |
| `moveToCompleted` / `moveToFailed` | | 0x47de00 / 0x47de70 | `moveToFailed` is always false |
| `execFSM` and the FSM commands | | | `docs/re/ai-fsm.md` §4.4 |

Engine commands in this area we have **not** registered: `commandStop` (0x568360), `stop`
(0x541980), `waypointTimeoutCurrent` (0x8f8d20), `lockWp` (0x1911a0), the waypoint attachment
commands (`waypointAttachVehicle` etc.), `limitSpeed` (0x536610), `setDestination`,
`expectedDestination` (0x56a880), `doFSM`/`commandFSM`, `nearTargets` (0x1cb6a0).

## 7. Deviations from the engine, and what is left out

- **Unit FSMs.** The native formation FSM (`CfgFSMs >> Formation`, the soldiers'
  `fsmFormation`) runs per unit with the engine's Man functions; without a cover search,
  `coverReached` holds only when cover is switched off (`docs/re/ai-fsm.md` §2.3). Scripted unit
  FSMs (civilians' `formationC.fsm`) and danger FSMs need the VM in the AI tick and do not run;
  soldiers have `fsmDanger = "-"` (danger handling off), so only civilians and module units miss
  theirs.
- **Waypoint statements and scripts** are stored and never run: the World has no VM. The engine
  checks the condition after arrival and runs the statements after the countdown
  (`docs/re/ai-formation.md` §3.2); `WorldEvent::WaypointCompleted` is the hook for a host.
- **`addWaypoint`'s radius is ignored** and the waypoint lands on the centre; the same for the
  middle element of `setWaypointPosition`'s `[center, radius]`.
- **Insert-at-current is an approximation.** The binary's insert path does not fix the current
  index up inline (the delete path does), and its trailing notification (world vtable `+0x7b0`)
  could not be resolved. Ours: insert before the active index shifts it up; insert at the active
  index becomes active; adding to a group that has finished its queue makes the new waypoint
  active.
- **Vehicle waypoints wait.** GETIN/GETOUT/LOAD/UNLOAD/TR UNLOAD/HOOK/UNHOOK/GETIN NEAREST never
  complete (no vehicle boarding yet, #124/#127); LOITER's radius and type and
  `setWaypointHousePosition` are stored and unused.
- **Knowledge is per group, not per unit**; no weapon-range gate, no firing effect (#127), no
  `nearTargets`/`targetsQuery`. The engine's detection model is in `docs/re/ai-detection.md`.
- **Locality gates only the AI tick.** No network messages are produced or consumed for any of
  this.
- `Behaviour::view_scale`, the knowledge constants, the pace thresholds and the trail radius are
  our values, chosen to be plausible, not traced from the binary.
