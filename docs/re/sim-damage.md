# Hit points, damage, destruction and ruins

Where an Object's damage lives, how a hit reaches the total and the hit points, when an Object is
destroyed, and what the engine puts in its place: the model and config side of destruction
(`CfgVehicles >> HitPoints`, `DestructionEffects`, ruins, `destrType`), the SQF commands and the
event handlers around it. Source: `arma3_x64.exe` (RVAs below), read with the Ghidra tooling in
`docs/re/TOOLING.md`; the class and object background is in `world-object-model.md` and ADR 0006.

The other half — how a projectile computes a hit and distributes it over the hit points — is
`sim-ballistics.md` §5–§7. That doc ends with "how hit-point damage feeds back into total damage"
as an open point; this one answers it.

**Confidence.** (high) is transcribed from the decompiled code with its constants. (medium) means
the mechanism is certain but a threshold or an input is inferred. (low) is a guess. Facts taken
from the community wiki rather than the binary are marked (wiki).

## 1. Where damage lives (high)

| state | where |
|---|---|
| total damage | `Object+0xf0` (a `f32` behind the anti-tamper wrapper); stored capped at **1000**, read back clamped to 1 |
| per-hit-point damage | `Object+0x260`: `f32*` array, one `f32` per hit point of the type |
| hit point count | `Object+0x268` |
| read one hit point | `FUN_140e850d0(obj, index)` — the raw stored value, `0` for an out-of-range index |
| destroyed flag | `EntityAI+0x5e4` bit 0 |
| kind byte | `Object+0x15a` (which class the Object really is; see `world-object-model.md`) |

The array at `+0x260` is indexed by the **model's** HitPoints LOD order: the type builder resolves
each config hit point's name (`HitHead`, `HitFuel`, …) to the index of the LOD entry with that
name and stores the index in the type. The Man type keeps two of them for its destruction rule:
`type+0x28d8` = `HitHead`, `type+0x28dc` = `HitBody`.

## 2. Applying damage (high)

Two entry points, both fed by `sim-ballistics.md` §7.2:

- **Accumulate** — `FUN_141026a00(obj, hit_info, ctx)`. `hit_info+0x4` is the value as `f32`;
  `hit_info+0x9` is the *additive* flag: `0` replaces the current value, `1` adds to it. The total
  delta and the per-hit-point deltas of one hit travel in one `hit_info`.
- **Store the total** — `FUN_14102cb00` (`SetDamage`):
  - a change smaller than `1e-6` is a no-op;
  - a value `≤ 0` clears the field to 0;
  - a value above 1000 is clamped;
  - after storing, if the Object went **alive → destroyed** and its kind byte `+0x15a` is one of
    `{3, 4, 5, 9}`, the destruction handler `vfunc +0x1f0` runs.
- **HandleDamage first**: both the total and each hit point pass through the `HandleDamage` event
  handler (event id **30**, invoker `vfunc +0xb00` = `0x140fb6590`) before they are stored; the
  handler's return value replaces the value that would have been applied (see §5).

### 2.1 Destroyed

`0x14013df60` is `damage ≥ 1.0`; `0x14013df30` is the flag `+0x5e4 & 1`. An Object is destroyed
when either holds, so damage reaching 1 destroys it without any class rule, and a class rule can
destroy it at a lower total.

**Man** — `FUN_1407442e0`, Man's `vfunc +0xb60` (high on structure, low on thresholds): destroyed
when

```
total ≥ 1.0                                    // 0x14013df60
    or hit_point[HitHead] ≥ tunable(+0x5cc)    // type+0x28d8
    or hit_point[HitBody] ≥ tunable(+0x5c8)    // type+0x28dc
```

and it then sets `flag5e4 |= 1`. The two thresholds are read from the global settings struct
`DAT_14225db68` (229 references; a tunable set, not a config value of the type). Their defaults
were not traced; 1.0 fits every observed behaviour ("a hit point fully damaged kills"), and the
engine's `setHitPointDamage ["HitBody", 1]` on a soldier is expected to kill.

**Other classes** — no vehicle destruction rule was traced. The wiki documents the same shape for
vehicles (medium): `HitHull` "automatically destroys the vehicle when depleted" (tracked
vehicles), `HitFuel` for wheeled ones, and "HitHull … when damaged over 0.9, vehicle will
explode" for tanks. Which threshold the code uses (1.0 or 0.9) is unverified.

## 3. `depends`: hit points derived from other hit points (config + wiki, high on syntax)

`depends` (a string on a `HitPoints` entry) makes one hit point's damage a function of others.
The dependent (subordinate) entry must be listed **after** the ones it depends on. The expression
language is the config one — hit point names, numbers, `Total`, `+`, `-`, `*`, `/`, `max`, `min`
and parentheses. `depends = "0"` clears it (an empty string is an error).

Soldier (`B_Soldier_F`, resolved dump), which exercises every form:

| hit point | selection `name` | armor | radius | passThrough | explosionShielding | `depends` |
|---|---|---|---|---|---|---|
| `HitFace` | head | 1 | 0.2 | 0.8 | 0.5 | — |
| `HitNeck` | neck | 1 | 0.2 | 0.8 | 0.5 | — |
| `HitHead` | head | 1 | 0.2 | 0.8 | 0.5 | `HitFace max HitNeck` |
| `HitPelvis` | pelvis | 6 | 0.24 | 0.8 | 1 | `0` |
| `HitAbdomen` | spine1 | 1 | 0.16 | 0.8 | 1 | `0` |
| `HitDiaphragm` | spine2 | 1 | 0.18 | 0.8 | 6 | `0` |
| `HitChest` | spine3 | 1 | 0.18 | 0.8 | 6 | `0` |
| `HitBody` | body | 1000 | 0 | 1 | 6 | `HitPelvis max HitAbdomen max HitDiaphragm max HitChest` |
| `HitArms` | arms | 3 | 0.1 | 1 | 1 | `0` |
| `HitHands` | hands | 3 | 0.1 | 1 | 1 | `HitArms` |
| `HitLegs` | legs | 3 | 0.14 | 1 | 1 | `0` |
| `Incapacitated` | body | 1000 | 0 | 1 | 1 | `(((Total - 0.25) max 0) + ((HitHead - 0.25) max 0) + ((HitBody - 0.25) max 0)) * 2` |

Two things this shows: `Total` is readable from a `depends` expression (so a hit point can mirror
the total, which leaves it to be damaged only through `passThrough`), and the fatal `HitBody` is
not hit directly at all — it is the worst of the four torso hit points, which is why its own
`armor = 1000` never matters. `Incapacitated` is the "downed" state: it turns any damage past 0.25
on the total, the head or the body into a hit point value, doubling the excess.

The expression is evaluated when a hit point's damage changes, in config order, so evaluating the
dependent entries in order is enough (real chains are acyclic and read only entries listed before
them).

## 4. Hard-coded hit point names (wiki, medium)

Besides `HitHead`/`HitBody` (§2.1, verified for Man) the engine knows other names by convention.
None of these were traced in the binary:

| name | effect when the hit point is depleted |
|---|---|
| `HitHull` | destroys the vehicle (tracked vehicles; tanks "over 0.9" explode) |
| `HitFuel` | destroys the vehicle (wheeled vehicles); also drains `Fuel` as it accumulates |
| `HitEngine` | restricts mobility |
| `HitTrack`, `HitWheel` | restrict mobility |
| `HitTurret` | turret cannot rotate |
| `HitGun` | gun stuck at minimum elevation |
| `HitAmmo` | nothing hard-coded (vanilla does not use it; `depends` is the only way it matters) |
| `HitAvionics`, `HitHRotor`, `HitVRotor` | HUD flicker, loss of power, spin |

`HitFuel` and `HitHull` both exist on plain wheeled cars (e.g. `C_Offroad_01_F`: `HitFuel`
armor 2 / passThrough 0.1, `HitHull` armor 1.5 / passThrough 0.5 / minimalHit 0.1), on top of
`HitEngine`, `HitBody` and the `HitGlass*`/`HitLFire*` entries inherited from `Car_F`.

## 5. The event handlers (wiki, high on signatures)

| handler | argument locality | parameters |
|---|---|---|
| `HandleDamage` | global | `[_unit, _selection, _damage, _source, _projectile, _hitPartIndex, _instigator, _hitPoint, _directHit, _context]` |
| `Killed` | local | `[_unit, _killer, _instigator, _useEffects, _shot, _real]` (`_shot`, `_real` are 2.22 additions used by unit tracking) |
| `MPKilled` | global | `[_unit, _killer, _instigator, _useEffects]` |
| `Dammaged` | local | `[_unit, _hitSelection, _damage, _hitPartIndex, _hitPoint, _shooter, _projectile]` |
| `Hit` | local | `[_unit, _source, _damage, _instigator]`; not fired for `allowDamage false` |
| `HitPart` | local | nested array, one entry per hit point |

`HandleDamage` runs where the Object is local, and its return value replaces the damage that
would have been applied — the *absolute* new value of the selection, not a delta ("returning 0
makes the selection invulnerable", and the handler is the way to change how damage is
distributed). `_context` selects what the call is about: `0` total damage, `1` a hit point,
`2` last hit point, `3` fake head hit, `4` total damage before bleeding. Event 30 is the invoker
`vfunc +0xb00` (`0x140fb6590`).

`Killed` also fires from `setDamage`/`setHitPointDamage` (their EH table lists Killed and
MPKilled as triggered), so a script can kill an Object directly.

## 6. SQF surface (wiki, high — locality and side effects quoted)

| command | locality | notes |
|---|---|---|
| `damage` / `getDammage` | argument global | total damage, 0..1 |
| `setDamage n` | argument local, effect global | sets the total; EH: Killed, MPKilled, not Dammaged |
| `setDamage [d, useEffects, killer, instigator, allowResurrection, shot]` | same | `killer`/`instigator` reach `Killed`; `shot` the projectile |
| `getHitPointDamage [name, …]` | argument global | 0..1; a single name returns a number, several an array; `0` for an invalid name since 1.94 |
| `setHitPointDamage [name, damage, useEffects, killer, instigator, breakRotor]` | **argument local, effect global** | replaces the hit point's damage, triggers Killed/MPKilled; "has no effect when allowDamage is set to false" |
| `getAllHitPointsDamage obj` | argument global | `[hitpointNames, selectionNames, damageValues]`, ordered by hit part index; `[]` for null or an Object without a shape |
| `setHit`, `getHit` | argument local / global | the same by model **selection** name (`name` in the config entry) |
| `setHitIndex`, `getHitIndex` | argument local / global | by hit part index |
| `alive` | argument global | `damage < 1` and not destroyed |
| `allowDamage`, `isDamageAllowed` | argument local / global | engines only, not terrain vegetation; "does not prevent scripted damage from `setDamage`, `setHit`, `setHitIndex` or `setHitPointDamage`", contradicting `setHitPointDamage`'s own note that it is ignored while `allowDamage` is false; `isDamageAllowed` is always false for a non-local Object |
| `forceHitPointsDamageSync` | — | forces a network sync of the hit point state (mutating commands do not always sync) |

`setHitPointDamage` and `Killed` are why damage is applied by the **local owner**: the command's
argument must be local, and the damage state a client may change is the state of an Object it
owns.

## 7. Destruction: effects, ruins and `destrType`

### 7.1 `class DestructionEffects` (high)

An entry of the hit point or of the type; the type-level ones run when the whole Object is
destroyed, the ones under a hit point when that hit point is destroyed. Each entry has
`simulation` and `type`:

| `simulation` | `type` names | meaning |
|---|---|---|
| `particles` | a `CfgCloudlets` class | particle bursts |
| `sound` | a `CfgSFX` class | sound |
| `light` | a `CfgLights` class | light flashes |
| `ruin` | **a model path** (`\A3\…\House_Big_01_V1_ruins_F.p3d`) | the ruin Object to create |
| `destroy` | a `CfgDestroyPos` class | delayed destruction phase |
| `damageAround` | a `CfgDamageAround` class | splash damage in the area |

Both the engine (`0x1410005f0`) and the wiki also gate on the entry: `Ruin1` etc. are ordinary
classes that may inherit, and may carry `position`, `intensity`, `interval`, `lifeTime`.

### 7.2 Ruins (high)

`FUN_140ec1540` (called from the destruction handler) creates the ruins of a destroyed Object:

1. walk the type's ruin list (`type+0x688`, count `type+0x690`) — the `simulation = "ruin"`
   entries of the type's `DestructionEffects`;
2. for each, take the `type` string (`entry+0x10`);
3. gate on it (`FUN_1410005f0(&DAT_1421d5a50, name)`), then resolve it to an Object type
   (`FUN_140ffed00(&DAT_1421d5a50, name, 2)`; failure prints `Failed ruin creation: no type for
   %s`). The registry is keyed by the model path, and BIS names the class
   `Land_` + the model's file stem — so `…\House_Big_01_V1_ruins_F.p3d` must exist as
   `Land_House_Big_01_V1_ruins_F` in `CfgVehicles`;
4. two more gates per entry (`FUN_140e8e290(entry, 1000)`, `FUN_140dcd150`), then create the
   Object (`FUN_141153d30(DAT_14220dc60, type, 1)`) at the destroyed Object's position.

A ruin is a normal Object of its own type, not a property of the destroyed one: the ruined house
is a new `Land_…_ruins_F` standing where the house was. Ruin classes derive from `Ruins_F`
(`Ruins_F` in turn from `HouseBase`/`Static`); `Wreck_Base` is the vehicle equivalent.

### 7.3 The damaged model: `replaceDamaged` (config, high; behaviour wiki)

A structure that survives damage can swap its model for a "damaged" class:

```
replaceDamaged = "Land_i_House_Big_01_V1_dam_F";
replaceDamagedLimit = 0.9;                                              // default
replaceDamagedHitpoints[] = {"Hitzone_1_hitpoint", "Hitzone_2_hitpoint"};
selectionDamage = "DamT_1";
```

When every hit point listed in `replaceDamagedHitpoints` is damaged past `replaceDamagedLimit`,
the Object is replaced by the `replaceDamaged` class (which must be a `CfgVehicles` class with the
damaged model); damage below the limit swaps back, so the two states are the same Object, not two
Objects. `selectionDamage` names the model selection the engine uses for the swap. Cars set
`replaceDamaged = ""` and an empty list, i.e. unused.

### 7.4 `destrType` (wiki, medium)

How a destroyed Object behaves. Census of the shipped `CfgVehicles` (419 `DestructNo`, 96
`DestructDefault`, 92 `DestructTree`, 56 `DestructBuilding`, 56 `DestructTent`, 44 `DestructWall`,
18 `DestructWreck`, 10 `DestructEngine`, 3 `DestructColumn`, plus a few lower-case/`0` entries):

| value | behaviour |
|---|---|
| `DestructDefault` | the Object stays and its `DestructionEffects` run |
| `DestructTree`, `DestructBush` | the model falls over around a model axis |
| `DestructWall` | falls forward or backward, away from the hit |
| `DestructTent` | collapses |
| `DestructBuilding` | ruins (the standard building path) |
| `DestructEngine` | burns |
| `DestructWreck` | becomes a wreck (vehicles) |
| `DestructColumn` | new in 2.22: leans 22.5° away from the source and falls |
| `DestructNo` | no destruction at all |

Trees and bushes are **not** in `CfgVehicles`: their types live in `CfgNonAIVehicles` (there is no
`Plants_F` model anywhere in `CfgVehicles`), which is why a type lookup has to search
`CfgNonAIVehicles` too.

## 8. Real configs (dumps of the shipped build)

`B_Soldier_F` (§3) on top of `Soldier_F`: type-level `armor = 2`,
`armorStructural = 4`, `explosionShielding = 0.4`, `minTotalDamageThreshold = 0.001`,
`impactDamageMultiplier = 0.5`.

`C_Offroad_01_F`: type-level `armor = 30`, `destrType = "DestructDefault"`,
`simulation = "carx"`; hit points (inherited from `Car_F` plus its own) `HitFuel` (armor 2,
radius 0.5, passThrough 0.1), `HitEngine` (armor 4, radius 0.25, passThrough 0.5), `HitBody`
(armor 1, passThrough 1), `HitGlass1..` (armor 0.1–0.25, explosionShielding 2), `HitHull`
(armor 1.5, passThrough 0.5, minimalHit 0.1), plus `HitLFWheel`/`HitRFWheel`-style entries with
**negative** armor (absolute values, §7.1 of `sim-ballistics.md`).

`Land_i_House_Big_01_V1_F`: type-level `armor = 2000`, `armorStructural = 1`,
`destrType = "DestructDefault"`, `selectionDamage = "DamT_1"`,
`replaceDamaged = "Land_i_House_Big_01_V1_dam_F"`,
`replaceDamagedHitpoints[] = {"Hitzone_1_hitpoint", "Hitzone_2_hitpoint"}`, and a type-level
`class DestructionEffects` with the ruin entry of §7.2 plus `sound`, `destroy` and `damageAround`
entries; the hit zone entries (`Hitzone_1_hitpoint`, …) carry armor 0.6, radius 0.4,
passThrough 0.4, minimalHit 0.02, explosionShielding 20 and their own `DestructionEffects`
(particles) and `convexComponent`.

## 9. Implementation notes for `a3-world` (deviation)

The Rust side (`crates/a3-world/src/damage.rs`, ADR 0006) keeps the observable behaviour and
leaves the model out:

- **Hit point indices** are the order of the `HitPoints` class entries in the merged config
  (own entries first, then inherited), because a3-world knows nothing about the model. The
  original indexes by the model's HitPoints LOD. Scripts only ever name hit points, and the
  ballistics side resolves names through the config, so the two orders agree on everything the
  engine exposes.
- **Ruins** are created as new Entities of a plain type named by the BIS rule
  (`Land_` + the ruin model's file stem) with the ruin model set, at the destroyed Object's
  transform. The original resolves the config class of the ruin (which needs a type registry
  that `World` does not own).
- **Fatal hit points** are a table on the parsed damage model: `Man` → `HitHead`/`HitBody` at the
  constant 1.0 (the tunables of §2.1 are untraced), everything else → `HitHull`/`HitFuel` at 1.0.
- **`depends`** is evaluated in config order after each change (a small expression parser, §3).
- **`HandleDamage`** is a synchronous seam on the `World` (`DamageHandler`): the handler sees the
  values the hit would apply and returns the values to apply instead. The SQF event handler
  registry that will drive it is a later issue, as is the network sync of §6.
- **Not implemented, and known to be missing**: `replaceDamaged` substitution (§7.3), the
  non-ruin `DestructionEffects` (particles, sound, `destroy`, `damageAround`), the hit
  point state of vehicles (§4 beyond `HitHull`/`HitFuel`), `DestructTree`-style model animation,
  `forceHitPointsDamageSync`.

## 10. Open points

- `DAT_14225db68 + 0x5c8`/`+0x5cc`: the Man fatal thresholds' defaults and what sets them (a
  difficulty setting?), and the equivalent rules of the vehicle classes (`HitHull` ≥ 0.9 or 1.0?).
- Whether the total damage ever derives from the hit points (other than through a `HandleDamage`
  handler): the wiki says it does not, and no code path from a hit point to `+0xf0` was found.
- The `passThrough`/`armorStructural` transfer path of §7.2 in `sim-ballistics.md`: the wiki
  describes it as imperfect and our RE found only part of it.
- How the network syncs damage and hit points when the owner changes (`forceHitPointsDamageSync`).
