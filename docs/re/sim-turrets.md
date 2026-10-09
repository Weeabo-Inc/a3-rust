# Turrets and recoil
How Arma 3 2.22 loads `CfgVehicles >> Turrets`, aims and rotates a turret each frame, produces the
muzzle transform a shot needs, and what "recoil" is. Source: `arma3_x64.exe`, read with the Ghidra
tooling in `docs/re/TOOLING.md`. Entity/type background: `world-object-model.md`; projectiles:
`sim-ballistics.md`.

**Confidence.** (high) = transcribed from decompiled code with its constants. (medium) = code path
certain, meaning of an input inferred. (low) = hypothesis. **Not traced** = looked for and not
found in this pass. **Addresses are RVAs** (VA = RVA + `0x140000000`); names are the shared-project
labels, so `a3re.py decompile Turret_Simulate` works.

## 1. `CfgVehicles >> Turrets` → `TurretType` (high)
`CfgVehicles >> <vehicle> >> Turrets >> <Class> >> Turrets >> …`, one `TurretType` per turret
class, nested to any depth.

| What | Where | Notes |
|---|---|---|
| `Turrets` array | `TransportType_Load` `0xf40c20` | reads `Turrets`, calls the ctor per entry |
| one turret class | `TurretType_ctor_LoadTree` `0xf6db60` | allocates `0x7d0` bytes, calls `TurretType_Load`, then recurses on its own `Turrets` |
| the config entries | `TurretType_Load` `0xf77880` | ~12 kB; every offset below comes from here unless stated |
| memory points, bones, muzzle | `TurretType_InitShape` `0xf74760` | on model load; from `TransportType_InitShape` `0xf34c40` and recursively |
| `weapons[]`, `magazines[]` | `FUN_140fc4db0` `0xfc4db0` | turret's own config class; also `linkedItems`. Optics: `FUN_141109960` / `FUN_14110bb00` |

`param_1` in `TurretType_Load` is already `+0x10` into the object (`TurretType` has two vftables,
at `+0x0` and `+0x10`), so the offsets below are absolute from the start of `TurretType`.

### 1.1 Fields (high unless noted)

| entry | offset | notes |
|---|---|---|
| `minElev` / `maxElev` / `initElev` | `+0x28` / `+0x2c` / `+0x30` | degrees × `0.017453292` → radians |
| `minCamElev` / `maxCamElev` / `initCamElev` | `+0x34` / `+0x38` / `+0x3c` | defaults `−π/2` (`0xbfc90fdb`), `+π/2`, 0 |
| `maxVerticalRotSpeed` / `maxHorizontalRotSpeed` | `+0x40` / `+0x44` | **degrees/s, stored unchanged** |
| `minTurn` / `maxTurn` / `initTurn` | `+0x48` / `+0x4c` / `+0x50` | degrees → radians |
| `primaryGunner` | `+0xd4` | byte, set last in the tree ctor |
| `elevationMode` | `+0xd8` | int, default 0 |
| `stabilizedInAxes` | `+0xe4` | byte |
| `primaryObserver` / `hasGunner` / `commanding` | `+0x6a1` / `+0x6a2` / `+0x6a4` | byte / byte / int |
| `animationSourceBody` / `Gun` / `Hatch` | `+0x6b0` / `+0x6b8` / `+0x6c0` | strings |
| `animationSourceStickX` / `StickY` | `+0x6c8` / `+0x6d0` | strings |
| `animationSourceCamElev` / `Elevation` | `+0x6d8` / `+0x6e0` | strings; the latter also cached at `+0x700` as a float |
| `body` / `gun` | *(not stored)* | read only in `InitShape`, §1.3 |
| `discreteDistance` | ptr `+0x630`, count `+0x634`, cap `+0x638` | float array; `discreteDistanceInitIndex` `+0x648` (0). No `discreteElevationStep` exists in the binary |
| `reliability` / `reliabilityControl` | `+0x64c` / `+0x650` | default 1.0 |
| `OpticsIn`+`ViewOptics` / `OpticsOut`+`ViewGunner` | `+0x5c8` / `+0x5b0` | via `FUN_14110bb00` |
| `TurretSpec` | `+0x5e0` | §1.2 |
| `turretInfoType` | `+0x728` | array |
| sub-turrets | array ptr `+0x30`, count `+0x3c`, cap `+0x38` | **medium** on the field order — appended through an `AutoArray` helper |

`elevationMode` is read with the *number* accessor (int), so the shipped names never reach it as
text; whether `locked`/`manual`/`powered` are hashed to 0/1/2 upstream was **not traced**. Turret
code treats it as an int: `mode == 2` clamps the wanted angles back inside the limits, `mode == 1`
follows them (`FUN_140f76c20`; the same block is inlined in `Turret_MoveWeapons` and
`FUN_140f81e50`).

### 1.2 `TurretSpec` (high)
A small class with two byte flags, loaded by `FUN_14110a7a0` into `TurretType+0x5e0`:
`showHeadPhones` → `+0x0`, `showBackpack` → `+0x1`. Shipped MBT: `TurretSpec { showHeadPhones = 0; }`.

There is **no `turretAxis` config key in this build** (the string is absent from the binary). The
turret axis is implicit: the body rotates about the model's vertical axis at the body bone.

### 1.3 `body` / `gun` are bones, not animation sources (high)
`TurretType_InitShape` resolves each against the model (`FUN_141202250` → bone/memory-point index)
and logs `"Error: %s: Turret body %s not found while initializing the model %s"` (resp. `Turret
gun`) when the index is negative — a skeleton mismatch is only a log line. So `body = "mainTurret"`
names the bone the turret rotates and `animationSourceBody` names the matching animation source.
`TurretType+0xec` / `+0xf0` are the *proxy* indices of the two, used by `FUN_140f735c0` to fetch a
proxy's current memory point.

### 1.4 Muzzle geometry from `gunBeg` / `gunEnd` (high)

| quantity | offset | expression |
|---|---|---|
| barrel midpoint (the muzzle point) | `+0xf8`, `+0xfc`, `+0x100` | `(gunBeg + gunEnd) / 2` |
| barrel direction, **unnormalised** | `+0x104`, `+0x108`, `+0x10c` | `gunBeg − gunEnd` |
| copy of the direction | `+0x11c`, `+0x120`, `+0x124` | |
| `memoryPointGun` | `+0x110`, `+0x114`, `+0x118` | single point; `memoryPointGun[]` builds an array at `+0x128` |

If neither name resolves, the midpoint is zero, the direction `(0,0,0)` and `+0x10c` becomes `1.0`.
Otherwise the direction's **first component is zeroed when `|x| > 1e-6`** — the barrel is flattened
into the turret's own vertical plane. `missileBeg`/`missileEnd` are the missile equivalent;
`selectionFireAnim` names the firing selection.

## 2. Per-frame turret update
`Turret_Simulate` `0xf7f8a0` (callers: `FUN_140f5e720`, and itself for nested turrets):

```
Turret_Simulate(turret, visualState, dt, ?, parentMatrix)
    type = turret+0x50                                  // carrier is turret+0x1d0's carrier
    Turret_AimDirToWanted(turret, vs, &aimDirWanted)    // sets the WANTED angles
    ... state machine, weapons, optics ...
    Turret_MoveWeapons(turret, vs, aiBrain, dt, parentMatrix)   // integrates the CURRENT angles
    per nested turret of vs: FUN_140f81b60(childTurret, …, parentMatrix)
```
| per-turret state (`TurretVisualState`) | offset | meaning |
|---|---|---|
| `Turn` / `Elevation` | `+0x10` / `+0x14` | current azimuth / gun elevation, radians |
| `yRotWanted` / `xRotWanted` | `+0x18` / `+0x1c` | commanded azimuth / elevation |
| — | `+0x20` | reload/servo progress, `−= dt·0.0033333334`, clamped `[0,1]` (≈300 s scale) |
| — | `+0x40` / `+0x44` | rates fed to the vehicle's servo sound |

(high on the offsets — they are the only fields `Turret_Simulate` and `Turret_MoveWeapons` read
from their state argument; medium on the field names, which are the animation-source names the
game exposes for the same rig.)

`Turret_AimDirToWanted` `0xf702e0`: `yRotWanted = atan2(d.x, d.z)` with the hull heading removed,
`xRotWanted = atan2(d.y, |d.xz|)`; skipped while the "wanted angles valid" flag `Turret+0x334` is
clear.

### 2.1 Speed clamping — `Turret_MoveWeapons` `0xf7ad30` (high; units certain, `k` medium)

```
skill   = AIBrain_GetSkill(aiBrain, 0)          // 1.0 when the shooter is the local player
maxTurn = skill · type.maxHorizontalRotSpeed    // +0x44, deg/s   (Turret+0x2c0 = turn rate)
maxElev = skill · type.maxVerticalRotSpeed      // +0x40, deg/s   (Turret+0x2c4 = elev rate)
rate  += clamp(k·(wanted − current) − rate, −3·dt, +3·dt)   // k = 4.0; 8.0 on the azimuth axis
rate   = clamp(rate, −max, +max)
angle += rate·dt                                            // Turret+0x2c0 / +0x2c4
angle  = clamp(angle, minTurn/minElev, maxTurn/maxElev)
```

`AIBrain_GetSkill` `0x1346b70` returns `1.0` when the acting entity is the local player, else a
value shaped by AI skill (`0.8·skill − 0.2` … `0.8 + 0.2·skill`, clamped to `[0.01,1]`).
`maxHorizontalRotSpeed = 1.2` (shipped `MainTurret` of `B_MBT_01_cannon_F`) therefore caps the turn
rate at 1.2 °/s, and the axis is additionally slew-limited to `3·dt` per step. The function returns
`true` while the turret is still moving; the caller uses that for the servo sound and the "aimed"
flag `Turret+0x335`.

### 2.2 Where the `body` / `gun` animation phases come from (high)
The model's `animationSourceBody` / `animationSourceGun` are ordinary animation sources; their
getters are `AnimationSourceTurretBody_GetValue` `0xf730b0` and
`AnimationSourceTurretGun_GetValue` `0xf731d0`. Each hashes the requested source name against
`Turret+0x20`, looks the source up on the vehicle (`FUN_140f31120`) and returns a float at `+0x14`
of the entry — the current axis value (medium on the exact mapping, high that the getters exist and
return an animation value). The `body`/`gun` names themselves never feed a phase.

## 3. Muzzle direction when firing
Model-space chain (`FUN_140f81b60` `0xf81b60` composes a child turret's 3×3 — `Turret+0x310…+0x330`
— with the parent matrix passed in):

```
p_veh   = type.gunMid                              // TurretType+0xf8/0xfc/0x100
dir_veh = (type.gunBeg − type.gunEnd), x zeroed, normalised      // +0x104…
M_turret = rotation(Turn)        // azimuth about the model's vertical axis
M_gun    = rotation(Elevation)   // about the turret's lateral axis
p_world  = M_entity · (M_turret · M_gun · p_veh)
dir_world= normalize(M_entity · M_turret · M_gun · dir_veh)
```

The shot spawner `FUN_140e661a0` `0xe661a0` takes a **vehicle-space** offset vector, transforms it
with the entity's own 3×3 and position (`entity+0x1a` → the composed matrix) and then

```
ammo   = AmmoType of the magazine (FUN_140e5b720)
spread = (U(0,1)·2 − 1) · ammo.dispersion           // ammo+0xdf0
dir'   = rotateY(dir, spread)                       // FUN_14035c5d0, about world Y
shot.SetPosition(p_world)      // vfunc +0x138
shot.SetVelocity(param_6)      // vfunc +0xa88 — param_6 is the direction
shot.<set dir>(dir')           // vfunc +0xc88
World_AddFastVehicle(shot)
```

`FUN_140e56b30` `0xe56b30` is the multi-pellet (`ShotSpread`) variant: one start point and direction
per pellet from `AmmoType+0xddc…+0xe40`, then the same sequence.

**Shooter velocity: not traced.** Neither spawn path adds a recognisable velocity term; the speed
comes from the shot's own setter, and `initSpeed` (`CfgMagazines`, e.g. `1670` for
`24Rnd_120mm_APFSDS_shells_Tracer_Red`) is read by the `AmmoType` loader. The two are joined inside
the callee, which was not traced.

## 4. Recoil

### 4.1 Config path (high)

| level | entries | loader | result |
|---|---|---|---|
| `CfgWeapons >> <w> >> modes >> <mode>` | `recoil` | `FUN_1410f75b0` `0x1410f75b0` | `RecoilFunction*` at mode `+0x610` |
| `CfgWeapons >> <w> >> modes >> <mode>` | `recoilProne` | `FUN_1410fab80` `0x1410fab80` | `RecoilFunction*` at weapon-type `+0x68` |
| `CfgRecoils >> <name>` | `muzzleOuter[]`, `kickBack[]`, `permanent`, `temporary` | `FUN_140e39f40` `0xe39f40` | one `Recoil::WeaponParams` (`0x38` bytes) per name; cache `FUN_140e39b30` `0xe39b30` |
| `CfgWeaponHandling >> Recoil` | `kickVisual`, `impulseCoef` | `FUN_1406edde0` `0x6edde0` (from `FUN_1406f0240` `0x6f0240`) | game constants |
| `CfgWeaponHandling` | `resting*`, `deployed*`, `upperBodyRadius`, `weaponRadius`, `deployTime`, `undeployTime`, `deployBipodTime`, `undeployBipodTime`, `groundLimits`, `objectLimits` | `FUN_1406ed650` `0x6ed650` | the unit's handling block |

`CfgMagazines` has **no** `recoil` entry in this build (whole tree scanned: 0 hits). `CfgAmmo` has
2, both `recoil = "Empty"`, inert for this mechanism.

`CfgRecoils >> <name>` is an **array**, not a class, of `(t, x, y)` triples. The shipped 120 mm
cannon uses `recoil = "recoil_single_primary_3outof10"`:

```
recoil_single_primary_3outof10[] = { 0,0,0,  0.03,0.0110829,0.043044,  0.03,0.0159085,0.0170136,
                                     0.03,0.0138285,0.0116128,  0.06,0.0066492,0.004788, … 0.06,0,0 }
```

`RecoilFunction::RecoilFunction` `0xe38f80` builds a keyframe array from it (stride `0x10` bytes,
four floats per key = `{A, B, C, −0.33·B}`). It walks the flat array in steps of 3 — the first
element accumulates into `A`, the next two become `B` and `C` — and then normalises the curve by
subtracting the last key's `A`, so the array is `(Δt, horizontal, vertical)` per key and the curve
returns to zero at the end. The `−0.33·B` component is a third rotation axis (low confidence which
one).

| `CfgRecoils` entry | offset: `RecoilFunction` (`0x30` B) / `Recoil::WeaponParams` (`0x38` B) | notes |
|---|---|---|
| vtable / refcount | `+0x0` / `+0x8` in each | `WeaponParams` pointer is at `RecoilFunction+0x28` |
| the key array | `RecoilFunction`: ptr `+0x10`, count `+0x18`, cap `+0x1c` | |
| `kickBack[]` | `WeaponParams+0x18`, max at `+0x1c` | must have exactly 2 elements |
| `muzzleOuter[]` | `WeaponParams+0x20`, `+0x24`, `+0x28`, `+0x2c` | `{x, y, a, b}`; loaded only when the array has exactly **4** elements (`FUN_140e39550`) |
| `permanent` / `temporary` | `WeaponParams+0x30` / `+0x34` | ints |

`FUN_140e39f40` `0xe39f40` builds the pair; validation is `0 ≤ muzzleOuter.x` and
`muzzleOuter.y ≤ muzzleOuter.x`, else `"Invalid recoil definition: %s"` and neither object is
created.

### 4.2 Application (partly traced)
Verified:

- The **man** side is an animation-source mechanism, not a rigid-body impulse: the recoil state is
  published as the animation source `"Recoil"` (plus `restingRecoil`, `restingRecoilPersistent`,
  `deployedRecoil`, `deployedRecoilPersistent`) and consumed by the Man rig. `FUN_1406edde0` copies
  `CfgWeaponHandling` — including `kickVisual` (camera-only share) and `impulseCoef` (common scale)
  — into the unit's handling object; `FUN_1406ed650` fills its resting/deployed block.
- For a **vehicle**, the shot is the only thing the fire path visibly creates; no force or torque
  on the vehicle body was found along the shot-spawn path.
- The vehicle's *visual* recoil is a model animation: `B_MBT_01_cannon_F` declares
  `AnimationSources >> class recoil_source { source = "reload"; weapon = "cannon_120mm"; }` and the
  skeleton has a `recoil` animation (`animate[] = { …, {"recoil", 0}, … }`). Its phase is the
  weapon's reload progress, i.e. driven by the reload state machine, not by physics (medium: the
  `source = "reload"` → phase mapping is the documented meaning; that state machine was not read).

**Not traced:** any impulse on the vehicle's PhysX body (`−muzzleDir·k`, a torque, mass scaling),
and the per-shot evaluation site of the `WeaponParams` curve. Its only decoded consumers are the
loader and two virtuals (dtor `+0x8`, a probe `+0x10`), so the evaluation either reads the fields
directly (no vtable dispatch) or was not recognised.

## 5. Open points
- Whether `elevationMode` is a hash of `locked`/`manual`/`powered`, and what each value does.
- `Turret+0x3b8` / `+0x3ba` / `+0x3bc`: `0x3b8` selects an external (remote/optics) transform for
  the aim direction, `0x3bc & 0xf` disables it. What sets them, not traced.
- The origin of the commanded aim direction fed to `Turret_AimDirToWanted` (AI target vs. mouse):
  `Turret_Simulate`'s caller `FUN_140f5e720` was not traced.
- Whether a shot inherits the shooter's velocity, and where `initSpeed` meets the shot's speed.
- Per-shot recoil application on vehicles; the `WeaponParams` curve evaluation.
- `memoryPointGun[]` (`TurretType+0x128`): how many muzzles one turret can have and how the active
  one is chosen.
