---
status: accepted
---

# World and Entity model: generational arena, closed simulation-class enum, network identity from day one

`crates/a3-world` keeps every Entity in a **generational arena** owned by `World`, addressed by
an **Entity ID**. Every Entity carries its **Network object ID** and its **Locality** from the
moment it is created. The engine's class tree (`Object` → `Entity` → `EntityAI` → `Transport` →
`Car` …) becomes a **closed `SimulationClass` enum**, chosen from the config `simulation` value.
Class-specific state lives in one enum field of the Entity. Static objects stay in a compact
table loaded from the WRP and become Entities only when they need simulation. `World::simulate`
reproduces the original's list order and per-entity step accumulator. The reverse engineering
behind this is in `docs/re/world-object-model.md` and `docs/re/net-object-model.md`.

## Decisions

**Storage and identity.**

- **Entity ID** `{index: u32, generation: u32}` indexes the arena. Deleting an Entity bumps its
  slot's generation, so stale IDs (in SQF variables, AI targets, attachments) resolve to
  nothing, which is `objNull` semantics for free.
- **Network object ID** `{creator: u32, id: u32}` sits on every Entity and in a
  `NetworkId → EntityId` index:
  - On the creating machine, `creator` is this machine's client ID and `id` comes from one
    per-World serial counter. This holds outside multiplayer too, so single player uses the same
    path: the "client" is the local player.
  - Messages from other machines carry their own pairs, and we store them unchanged.
  - Creator 0 is null; creator 1 is reserved for Static objects.
  - A local-only Entity (`createVehicleLocal`) has no Network object ID.
- **Locality**: `Local` or `Remote { owner: Option<ClientId> }`. The server knows the owner of
  every Object; a client knows only whether an Object is its own (the original's `owner` command
  returns 0 on clients). Locality changes go through a single `World` method, which queues the
  `Local` event.
- **Static objects**: about 1.8 million per terrain on Altis. They stay as WRP records in a
  per-land-cell table, addressed by Object ID, with no per-object heap allocation. Their Network
  object ID is `{1, packed cell/index key}`, computed, never stored. They are promoted to an
  Entity in the arena (keeping that Network object ID) when they need behaviour: a config class
  with a non-trivial `simulation` (houses, lamps), damage, animation, or a script-set variable.
  Promotion is invisible to scripts.
- **No ECS** (ADR 0001). The World is the single owner; systems are methods on it.

**Class tree.**

- `SimulationClass` lists the engine's concrete classes (`Man`, `Car`, `CarEpe`, `Tank`,
  `TankEpe`, `Helicopter`, `HelicopterRtd`, …, `Building`, `Thing`, `Shot`, …). It is parsed
  from the `simulation` string with the table in `world-object-model.md`.
- `SimulationClass::is_kind_of(EngineClass)` answers the engine-level kind questions
  (`EntityAi`, `Transport`, `Person`, `Air`) that decide World lists, network update classes and
  command behaviour. Config `isKindOf` (class inheritance) is a different question and stays in
  `a3-config`.
- Common state (transform and visual-state history, type, flags, damage, simulation step and
  accumulator, identity, Locality) is in `Entity`. Class-specific state is `Entity::class_state:
  ClassState`, an enum with one variant per family (`Man(ManState)`, `Vehicle(VehicleState)`,
  `Shot(ShotState)`, `Static`, …).
- Behaviour dispatches with `match` in `World::simulate` and per-family modules (`man.rs`,
  `car.rs`), not through `dyn` traits. The class set is closed and known from RE, so `match`
  keeps exhaustiveness checks, and borrowing the rest of the World while simulating one Entity
  stays explicit.

**Simulation order** (`World::simulate(dt)`, called once per frame, ADR 0002):

1. Apply received network state to remote Entities.
2. Projectiles (the original's "fast vehicles"), each at its own step.
3. Vehicles, agents and other simulated Entities: the frame is cut into 0.025 s sub-steps (the
   original's catch-up threshold), then the remainder. Each Entity accumulates time and runs one
   step of its simulation step's length when the accumulator reaches it. The original's
   interleaved catch-up loop is not reproduced exactly (`world-object-model.md`, issue #117);
   every Entity covers exactly the frame time. Physics (rapier) steps inside this phase on the ADR 0002 fixed
   accumulator.
4. Attached positions.
5. AI.
6. Deferred deletions and Locality events; event handlers and scripts run after the step, never
   in the middle of an Entity update.

On every machine `simulate_entity` runs for local and remote Entities alike. Authoritative work
(physics forces, damage, AI decisions, weapon fire) runs only when the Entity is Local, as in the
original's `IsLocal()`-guarded `Simulate`. Remote Entities advance from their last network state.

**SQF handles.** `a3-sqf`'s `Handle { kind: Object, id: u64 }` encodes:

- an Entity as `generation << 32 | (index + 1)`, never 0;
- a not-promoted Static object as `1 << 63 | packed key`.

`isNull` asks the World whether the handle still resolves. `netId` and `objectFromNetId` go
through the Network object ID index (and the reserved creator 1). Groups get their own arena and
`HandleKind::Group` handles with the same generational rule.

## Considered options

- **Engine IDs as the only key** (a `HashMap<NetworkId, Entity>`): rejected. Local-only Entities
  have no Network object ID, single player would need fake ones, and every access would pay a
  hash lookup. Keeping both IDs costs one index map.
- **ECS** (bevy_ecs, hecs): rejected in ADR 0001. SQF's synchronous get/set across many
  components, and the engine's closed class tree, gain nothing from archetypes.
- **`dyn EntityBehaviour` trait objects per class**: rejected. The class set is closed, so
  dynamic dispatch buys no extensibility, loses `match` exhaustiveness and makes borrowing the
  World during an update awkward.
- **Every Static object as an Entity**: rejected. 1.8 million arena slots of hundreds of bytes
  each for objects that mostly never change. Promotion on demand matches how the original keeps
  plain map Objects separate from simulated ones.
- **Ownership as a `ClientId` on every machine**: rejected. Clients do not have that
  information in the original protocol, and inventing it would hide bugs that the official
  client exposes.

## Consequences

- Phase 6 needs no retrofit: create, update, delete and ownership messages map one-to-one onto
  `World` operations keyed by Network object ID.
- Code holding an Entity across frames holds an Entity ID and must handle `None`.
- The `simulation` mapping table is shared data. A missing value is a load-time error that
  names the config class, so gaps show up on real data.
- Promotion of Static objects is a World operation. Readers of Static objects must go through
  `World` lookups, never through the WRP arrays directly.
