# AI formations, follower movement and waypoint execution

How the engine places units in formation, moves a group and works its waypoints; what a3-rust
implements is in section 8 and in `docs/re/ai.md`. Binary: `arma3_x64.exe`
2.22.0.154103, Ghidra project `a3`. Addresses are given as **RVA / VA** (VA = RVA + 0x140000000).
Functions renamed in the shared Ghidra project are named in `code` (for example
`AISubgroup_UpdateFormationPos`). All pseudo-code is a paraphrase in my own words.

Confidence tags: **[H]** read directly from code or config, **[M]** read from code but part of the
meaning is inferred, **[L]** inference or an unverified guess.

---

## 0. Enums (all [H], from the static enum-table initialisers)

| enum | values | initialiser |
|---|---|---|
| Formation (`AISubgroup+0xb8`) | 0 COLUMN, 1 STAG COLUMN, 2 WEDGE, 3 ECH LEFT, 4 ECH RIGHT, 5 VEE, 6 LINE, 7 DIAMOND, 8 FILE | RVA 0xcfa80, table VA 0x14221e830 |
| Behaviour | 0 UNCHANGED, 1 CARELESS, 2 SAFE, 3 AWARE, 4 COMBAT, 5 STEALTH | RVA 0x7c8e0, table 0x1421d1b80 |
| Speed mode (`AISubgroup+0xbc`) | 0 UNCHANGED, 1 LIMITED, 2 NORMAL, 3 FULL | RVA 0x7ed90 |
| Waypoint type (`wp+0x58`) | 0 UNDEF, 1 MOVE, 2 DESTROY, 3 GETIN, 4 SAD, 5 JOIN, 6 LEADER, 7 GETOUT, 8 CYCLE, 9 LOAD, 10 UNLOAD, 11 TR UNLOAD, 12 UNHOOK, 13 HOOK, 14 VEHICLEINVEHICLEGETIN, 15 …GETOUT, 16 …UNLOAD, 17 HOLD, 18 SENTRY, 19 GUARD, 20 TALK, 21 SCRIPTED, 22 SUPPORT, 23 GETIN NEAREST, 24 DISMISS, 25 LOITER, 26 FOLLOW, 27 AND, 28 OR | name strings at VA 0x141bb82e8.. (order of the strings = enum order; confirmed by the Turn dispatch in §3.2) |
| Command type (`Command+0x44`) | 0 NO CMD, 1 WAIT, 2 ATTACK, 3 Suppress, 4 HIDE, 5 MOVE, 6 HOOK CARGO, 7 UNHOOK CARGO, 8 VIV GETIN, 9 VIV GETOUT, 10 VIV UNLOAD, 11 HEAL, 12 REPAIR, 13 REFUEL, 14 REARM, 15 SUPPORT, 16 JOIN, 17 GET IN, 18 FIRE, 19 GET OUT, 20 STOP, 21 EXPECT, 22 ACTION, 23 SCRIPTED, 24 DISMISS, 25 HEAL SOLDIER, 26 PATCH SOLDIER, 27 FIRST AID, 28 HEAL SELF, 29 ATTACK AND FIRE, 30 CARRY SOLDIER, 31 DROP CARRIED, 32 TAKE BAG, 33 ASSEMBLE, 34 DISASSEMBLE, 35 DROP BAG, 36 OPEN BAG, 37 IRLASER ON, 38 IRLASER OFF, 39 GUN LIGHT ON, 40 GUN LIGHT OFF, 41 FIRE AT POSITION, 42 REPAIR VEHICLE, 43 OPEN PARA, 44–47 KEEP DEPTH (LEADER / UND SURF / ABV SURF / BOTTOM), 48 PUT IN, 49 UNLOAD FROM, 50 USE CONTAINER MAGAZINE, 51 ACTIVATE MINE, 52 DISABLE MINE | RVA 0xe3800, table 0x142220050 |
| AIUnit state (`AIBrain+0xd408`) | 0 WAIT, 1 INIT, 2 INITFAILED, 3 BUSY, 4 OK, 5 DELAY, 6 CARGO, 7 STOPPING, 8 REPLAN, 9 STOPPED, 10 PLANNING | RVA 0xe67d0 |
| Planning mode (`AIBrain+0x234`) | 0 DoNotPlan, 1 DoNotPlanFormation, 2 LEADER PLANNED, 3 LEADER DIRECT, 4 FORMATION PLANNED, 5 VEHICLE PLANNED | RVA 0xe5a50 |

---

## 1. Formation geometry

### 1.1 The data lives in config: `cfgFormations` [H]

There **is** a formation table in config: `class cfgFormations` (in the merged game config).
It has one class per side (`West`, `East: West`,
`Guer: West`, `Civ: West`; all four are identical in the shipped data). Each side class holds
nine formation classes. **The engine reads them by index, not by name**, in this order, which
is the Formation enum order:

| index | class | enum |
|---|---|---|
| 0 | `formColumnFixed` | COLUMN |
| 1 | `Staggered` | STAG COLUMN |
| 2 | `Wedge` | WEDGE |
| 3 | `EchelonLeft` | ECH LEFT |
| 4 | `EcholonRight` (sic) | ECH RIGHT |
| 5 | `Vee` | VEE |
| 6 | `Line` | LINE |
| 7 | `Diamond` | DIAMOND |
| 8 | `File` | FILE |

Loader: `AICenter_LoadCfgFormations` RVA 0x12b71b0 / VA 0x1412b71b0. It picks the side class
from `AICenter+0x20c` (0 West, 1 East, 2 Guer, 3 Civ; any other value falls back to East),
iterates the side's subclasses by index (at most 9), and stores per formation (stride 0x30)
at `AICenter+0x310 + f*0x30`:

| offset | type | content |
|---|---|---|
| +0x00 | ptr, int count, int cap | `Fixed[]` entries |
| +0x18 | ptr, int count, int cap | `Pattern[]` entries |

Each entry is 0x18 bytes, read from an array `FormationPositionInfoN[]`:

| entry offset | type | array element |
|---|---|---|
| +0x00 | int | [0] reference slot (see below) |
| +0x04 | float | [1] x (right +) in formation units |
| +0x08 | float | always 0 (y) |
| +0x0c | float | [2] z (forward +) in formation units |
| +0x10 | float | [3] watch-direction offset, radians |
| +0x14 | bool | [4] if present (5-element `Pattern` entries only), else true. Meaning unknown [L]. |

A non-array value in the class gives an all-zero entry. If a side class has fewer than nine
formations, the missing ones get a default: `Fixed = [{-1,0,0,0},{0,1,0,0}]`,
`Pattern = [{-2,-1,0,0},{-1,1,0,0}]` (that is a LINE).

### 1.2 Slot computation — `AISubgroup_UpdateFormationPos` RVA 0x131b740 / VA 0x14131b740 [H]

Run per subgroup when the formation or membership changes. It iterates **group unit slots**
`i = 0 .. group+0x178` (`AIGroup_UnitBySlot` RVA 0x12d95f0: `group+0x170` array, stride 0x38,
the unit pointer at +8). Slot index = the unit's position in the group's ID list, so empty
slots (dead/removed IDs) still occupy a formation place [M].

```
fixed, pattern = center.formations[subgroup.formation]
for i in 0..nSlots:
    if i < len(fixed):  info = fixed[i];  ref = info.ref                       # absolute slot
    else:               k = (i - len(fixed)) % len(pattern)
                        info = pattern[k]; ref = info.ref - k + i             # relative to block
    if ref < 0:  local[i] = (info.x, 0, info.z)                                # unscaled (leader: 0,0)
    else:
        (fxR, fzR) = formationX/Z of slot ref's vehicle type, or (1,1) if that slot is empty,
                     not in this subgroup, or has no vehicle
        (fxI, fzI) = same for slot i
        local[i].x = local[ref].x + info.x * (fxI + fxR) / 2
        local[i].z = local[ref].z + info.z * (fzI + fzR) / 2
    if unit i exists and belongs to this subgroup:
        unit.formAngle (AIUnit+0xd5b0) = info.angle
        (special: if exactly one unit of the subgroup is of a vehicle class whose type vfunc
         +0x228 returns true, that unit's angle is forced to 0 [L: meaning of +0x228])
        unit.formPos (AIUnit+0xd5b4..0xd5bc) = local[i]
```

`formationX`/`formationZ` are `EntityAIType+0x1558` / `+0x155c`, loaded in RVA 0xfbe020
(`formationTime` +0x1560, `formationTimeSwimming` +0x1568, `formationTimeDiving` +0x1570).
Config: `All` 10/20/time 5; `Man` 5/5; `CAManBase` inherits Man (5/5, `formationTime` 5,
swimming 3, diving 2). So for a group of men, one formation unit = **5 m** in both axes.

### 1.3 Resulting offset tables (formation units; ×5 m for infantry) [H — computed from the config with the algorithm above]

`x` is to the right of the formation direction, `z` forward; slot 0 is the leader's slot.

| formation | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| COLUMN | 0,0 | 0,-1 | 0,-2 | 0,-3 | 0,-4 | 0,-5 | 0,-6 | 0,-7 | 0,-8 | 0,-9 | 0,-10 | 0,-11 |
| STAG COLUMN | 0,0 | 1,-1 | 0,-2 | 1,-3 | 0,-4 | 1,-5 | 0,-6 | 1,-7 | 0,-8 | 1,-9 | 0,-10 | 1,-11 |
| WEDGE | 0,0 | 1,-1 | -1,-1 | 2,-2 | -2,-2 | 3,-3 | -3,-3 | 4,-4 | -4,-4 | 5,-5 | -5,-5 | 6,-6 |
| ECH LEFT | 0,0 | -1,-1 | -2,-2 | -3,-3 | … | | | | | | | -11,-11 |
| ECH RIGHT | 0,0 | 1,-1 | 2,-2 | 3,-3 | … | | | | | | | 11,-11 |
| VEE | 0,0 | 1,0 | -1,1 | 2,1 | -2,2 | 3,2 | -3,3 | 4,3 | -4,4 | 5,4 | -5,5 | 6,5 |
| LINE | 0,0 | 1,0 | -1,0 | 2,0 | -2,0 | 3,0 | -3,0 | 4,0 | -4,0 | 5,0 | -5,0 | 6,0 |
| DIAMOND | 0,0 | 0.5,-0.5 | -0.5,-0.5 | 0,-1 | 0.5,-1.5 | -0.5,-1.5 | 0,-2 | 0.5,-2.5 | -0.5,-2.5 | 0,-3 | 0.5,-3.5 | -0.5,-3.5 |
| FILE | 0,0 | 0,-0.5 | 0,-1 | 0,-1.5 | 0,-2 | 0,-2.5 | 0,-3 | 0,-3.5 | 0,-4 | 0,-4.5 | 0,-5 | 0,-5.5 |

Notes:
- VEE is asymmetric in the data (right arm starts level with the leader, left arm one unit
  ahead), and opens **forward** (+z). That is what the config says; it is not a transcription
  error.
- DIAMOND repeats a 4-unit diamond behind the previous one; FILE is a COLUMN at half spacing.
- Chain references: in COLUMN/ECH/DIAMOND/FILE each slot is placed relative to the previous
  slot, in WEDGE/VEE/LINE relative to the slot two back. Mixed vehicle types therefore space
  each link with the average of the two linked types' formationX/Z.
- Watch-direction offsets (`info.angle`, radians, added to the formation direction): COLUMN
  slot1 +π/4, slot2 −π/4, slot3 π (rear guard), pattern repeats `0, +π/4, −π/4, π`;
  WEDGE −π/4 left arm, +π/4 right arm; ECH LEFT −π/4 (every 4th −π/2); ECH RIGHT +π/4 (every
  4th +π/2); VEE leader −π/4, arms ∓π/4; LINE 0; DIAMOND +π/4, −π/4, 0; FILE −π/4, +π/4.
  The sign convention of the angle follows the engine's rotation helper (RVA 0x35c560), which
  `setFormDir` also feeds with `−deg·π/180` [M].

### 1.4 Formation direction [H for storage, M for updates]

- `AISubgroup+0xd0..0xd8` = formation direction (unit Vector3).
  `AISubgroup+0xe8` = time it was set explicitly; `+0xdc` "directionMove", serialised as
  `direction`/`directionMove`/`directionChanged` (`AISubgroup` serialiser RVA 0x1317340).
- `AISubgroup_SetFormation` RVA 0x1319250: stores the formation, sends radio message 7
  (RadioMessageFormation) to the group, **resets the direction to the leader vehicle's current
  facing**, and clears a flag bit 2 at `vehicle+0x5e4` on every member that is not in this
  subgroup's formation.
- `setFormDir` (SQF RVA 0x191d00) → `AISubgroup_SetDirection` RVA 0x1319220: direction =
  rotation of the given azimuth; stamps `+0xe8` with the current time (ms).
- `formationDirection` (SQF RVA 0x192bf0) → `AIUnit_GetFormationDirection` RVA 0x1349750:
  the subgroup direction rotated by the unit's own `formAngle` (the leader's when he is in a
  vehicle) — so `formationDirection unit` is that unit's **watch** direction, in degrees
  `atan2(x,z)` wrapped to [0,360).
- How the direction follows movement was **not** traced. Commands store a normalised
  `dest − leaderPos` direction at `Command+0x7c` when issued (RVA 0x1314bf0), which is the
  likely source of `directionMove` [L].

### 1.5 A unit's absolute slot position [H]

`AIUnit_GetFormationAbsolute` RVA 0x1348cb0 (used by SQF `formationPosition` RVA 0x56ae60 /
`expectedDestination` etc.) → `AIUnit_ComputeFormationPos` RVA 0x1348ea0:

```
leader  = subgroup.leader   (AISubgroup+0x90)
local   = unit.formPos - leader.formPos + unit.extraOffset (AIUnit+0xd58c..0xd594)
M       = orientation(dir = subgroup.direction, up = world up)
base    = leaderVehicle.stateAt(t).position          # t = caller time; <0 → vehicle vfunc +0x1b8
slot    = base + M * local                           # x along M's aside (right), z along dir
y       = height of the *unit's own* vehicle state (not the leader's)
if unit uses road-column logic (see §2.3, never for men because Man vfunc +0x1690 = false): …
slot   += leadSeconds * referenceUnit.velocity       # 3rd argument, 0 in nearly every caller [M]
if vehicle type +0x1748 == 2 (air) and not ‘raw’: y = max(y, surfaceY + 25)
```

Without a subgroup or leader it returns the unit's own position (and logs "No subgroup" /
"No leader"). `formationPosition` returns ASL→ATL converted coordinates.

---

## 2. Follower movement

### 2.1 Leader speed control — `AISubgroup_SetLeaderSpeed` RVA 0x131ad20 [H]

Runs per subgroup; writes the leader's **wanted speed cap** to `leaderVehicle+0x648` (m/s).

```
v = leaderType.maxSpeed / 3.6         (EntityAIType+0x1764, km/h → m/s; Man: 24 → 6.67 m/s)
    (× a global factor when the vehicle is a Person in a special state, vfuncs +0xd8/+0xc90)
v = max(v, 0.1)
best_v, best_lag = v, 0
for each member ≠ leader, in this subgroup's formation, NOT in trail-follow mode (§2.3):
    lag = dot(slotPos − unitPos, subgroup.direction) * 0.1      # metres behind the slot / 10
    vf  = member maxSpeed / 3.6
    if lag / vf > best_lag / best_v: best_v, best_lag = vf, lag
slack  = 0.5 if leader behaviour < COMBAT else 1.0
target = (best_v − (best_lag − slack) / 3) / v                # ratio of leader top speed
coef   = formationCoef (AISubgroup+0xc8, default 1.0)
coef  += clamp(target − coef, −0.1·dt, +0.1·dt)               # dt seconds since +0xcc; 0.1/s slew
coef   = clamp(coef, 0.1, 1.5);  store coef and now (+0xcc)
speed  = coef * v
switch speedMode:
  LIMITED: if leader behaviour ≠ COMBAT: speed = min(speed, max(0.1, v * limitedSpeedCoef))
           leader.speedCap = speed
  NORMAL:  leader.speedCap = speed
  FULL:    leader.speedCap = 1.5 * v          # leader never waits for the group
  other:   unchanged
```

`limitedSpeedCoef` is `EntityAIType+0x176c` (config `All` 0.22, `Car` 0.5, Man inherits
0.22 → LIMITED infantry cap ≈ 1.47 m/s, a walk). This is the "formationCoef" the save game
stores. So: the leader slows down (to 0.1×) when followers lag, speeds up to 1.5× when they
are ahead, and LIMITED additionally caps him; COMBAT ignores the LIMITED cap.

### 2.2 Script speed limits [H]

- `forceSpeed` (RVA 0x569ed0): `AIBrain+0x23c` = speed in m/s; any negative value → −1
  (= no forced speed).
- `limitSpeed` (RVA 0x536610): `vehicle+0x64c` = number × (1/3.6) (km/h → m/s); a bool or
  anything else → `maxSpeed / 3.6 × 2` (= effectively unlimited).
- `setSpeedMode` (RVA 0x192270) writes `AISubgroup+0xbc`; NORMAL is the serialiser default.

How the man's movement code turns `+0x648`, `+0x64c`, `+0x23c` into walk/run/sprint was **not
traced** [open]. The vehicle steering component has a column speed rule (RVA 0x423350): with
`s` = spacing (custom or the average formationZ of it and the vehicle ahead) and `d` = distance
to the vehicle ahead, `k = clamp((d − 1.5 s)·0.4/s, −1, 1)`; when `k > 0.3` it sets a flag and
scales a speed term by `clamp(sqrt(1.4 s / d), 0, 1)` [M: which term is scaled].

### 2.3 Two follow modes: slot vs. trail [M]

`EntityAIFull` vfunc +0xeb8 (Man: RVA 0x3ffba0) decides whether a unit **follows the trail of
the unit ahead** instead of steering to its geometric slot:

- returns true when the unit's behaviour is CARELESS or SAFE;
- for AWARE/COMBAT/STEALTH (vfunc +0xea8, RVA 0x690210 → behaviour in 3..5) it returns true only
  if the leader or a lower-numbered member stands on a road (`FUN_14134c440` road-level ≥ 2,
  RVA 0x34c440 queries the road net) [M].

"Unit ahead" (RVA 0x13146f0): among the subgroup's other trail-following members, the one with
the highest formation number (`AIUnit+0xd588`) below mine; otherwise the leader. Trail mode
(`EntityAIFull` vfunc +0x1520, RVA 0xfa8c60, shared by Man) walks back along a 10-point ring
buffer of the ahead unit's past positions (`vehicle+0xa1c`, 12-byte points, head index
`+0xa9c`; seeded by RVA 0xfb9780 at spacing `formationZ × 0.1` behind it) by a look-back
distance of `formationTime × aheadSpeed × 0.6` (0 when the ahead unit is on a road), floored at
`1.5 × r + 2.5` when the ahead unit is slow (< 9 m/s) or flagged (`+0x5e7`) — `r` from vfunc
+0x6f0 [L: a size/radius]. If that trail point is within `1.25 × (fzAhead + fzSelf)` of the
unit and behind it (or the unit is nearly stopped), the unit holds instead of overtaking.

Consequence: SAFE/CARELESS infantry walk in single file behind the man in front (slot geometry
mostly unused), AWARE+ infantry off-road steer to their geometric slot.

### 2.4 "Too far from formation" [H]

`EntityAIFull_IsFarFromFormation` (vfunc +0xd80, RVA 0xfbc090):
`limit = clamp(12 × max(formationX, formationZ), 150, 500) × scale` — infantry 150 m × scale;
true when the slot (or, in trail mode, the unit ahead) is further than `limit` (3-D for slot
mode, 2-D for trail mode). Callers / the `scale` values were not traced.

### 2.5 Behaviour and the per-unit Formation FSM [H for config, M for effects]

`CAManBase` has `fsmFormation = "Formation"`, `fsmDanger = "-"`. `CfgFSMs >> Formation` is a
config FSM of coded functions (resolved in RVA 0x775c30 → AIUnit methods; `vehicle+0xb38` is
the brain):

```
Init      [formationInit]                 → Start
Start     vehicle → Excluded; behaviourCombat → Combat; else → Excluded
Excluded  [formationExcluded]             → Search_path__No_1
Search_path__No_1 [searchPath(0,0)]       coverReached → Provide_cover__Out; else → Start
Combat    formationIsLeader → Leader;      else → Search_path__Covering
Leader    [formationLeader]               → Search_path__No [searchPath(0,0)]
                                              coverReached → Provide_cover__Out; else → Test_reload
Search_path__Covering [searchPath(10,5)]  coverReached → Provide_cover__Out; else → Start
Test_reload  reloadNeeded → Drop_to_ground [setUnitPosToDown] → Reload [reload] → Start
Provide_cover__Out [formationProvideCover] / Next_target__Out [formationNextTarget] /
Hide_in_cover__Hidden [formationHideInCover]:
     vehicle or not combat → Clean_up [formationCleanUp] → Start
     reloadNeeded → Drop_to_ground_1 → Reload__Hiden_ → Provide_cover__Out
     formationCanLeaveCover → Clean_up
     randomDelay → Hide_or_Out_ (threshold-1 'true' link → Next_target__Out, else Hide_in_cover)
```

So formation bounding/covering exists only in COMBAT ("behaviourCombat"); other behaviours
sit in `Excluded` (plain formation following). Effects read so far:
- `formationExcluded` (RVA 0x1343bc0) / `formationLeader` (RVA 0x1343ce0): person vfunc
  +0x1c60 (unit-pos request) is called; `vehicle+0xa14` = 2 when behaviour ≥ COMBAT, else 0
  [L: a stance/cover mode]; `formationExcluded` also sets `vehicle+0xaa0 = 1`.
- `formationNextTarget` (RVA 0x1343d40): stamps `AIUnit+0xd618` = now; with 30 % chance
  (random > 0.7) requests unit-pos 2 (down) when the current unit-pos is 1 [M].
- `setUnitPosToDown` (RVA 0x1344bb0): person vfunc +0x1c60 with 2.
- `formationHideInCover` (RVA 0x1343c70): next-target logic, then `vehicle+0xa14 = 0`.

The per-behaviour stance choice for SAFE/AWARE outside this FSM (SAFE standing/lowered weapon,
etc.) was not traced [open].

---

## 3. Waypoint execution: the group Mission FSM

### 3.1 Structure [H]

`AIGroup_CreateMissionFSM` RVA 0x129cb80: an `FSMTyped<AIGroupContext>` (vtable
`0x141caadc8`) over a static state table at VA **0x1420c2760**, 0x52 = 82 states, entries of
0x20 bytes `{name, enter, check, exit}` (a group without a mission gets the 1-state table at
0x1420c2740, "Wait"). Context: `+0x00` mission data (its `+0x30..0x38` = target position,
`+0x3c` = completion radius, `+0x20` = target link), `+0x08` the AIGroup, `+0x10` the FSM.
FSM int vars (vfunc +0x50): 0 = current waypoint index, 1 = waypoint type, 2/3 = copies of
`wp+0x48`/`wp+0x40`, 4 = "waiting" flag, 5 = retry/brown counter. Timer var 0 (vfunc +0x60).
`SetState` = FSM vfunc +0x20.

Waypoint (0x138 bytes, `AIGroup+0x2f0`) fields used here: `+0x18` synchronisation id, `+0x1c`
position, `+0x30` name, **`+0x3c` completionRadius (default 0)**, `+0x40`, `+0x48` attached
object/house link, `+0x4c` house position index (default −1), `+0x58` type, `+0x70/+0x74/+0x78`
timeout min/mid/max (default 0), `+0x88` condition (text) / `+0x98` compiled, `+0x90`
statement, `+0xb0` script, `+0xd8` effects. Defaults from RVA 0xce06d0 / 0xcea6a0.

States (index: name — enter / check):

| # | state | # | state | # | state |
|---|---|---|---|---|---|
| 0 | Init | 28 | GetOut Move | 56 | Gravon Wait |
| 1 | Turn | 29 | GetOut GetOut | 57 | Gravon Attack |
| 2 | Move Move | 30 | Load Move | 58 | Gravon Overlook |
| 3 | Talk Move | 31 | Load GetIn | 59 | Gravon Move |
| 4 | Talk GetOut | 32 | Unload Move | 60 | Support Move |
| 5 | Talk Walk | 33 | Unload GetOut | 61 | Support Wait |
| 6 | Dismiss Move | 34 | TransportUnload Move | 62 | Support Transport |
| 7 | Dismiss Dismiss | 35 | TransportUnload GetOut | 63 | Support Supply |
| 8 | Destroy Move | 36–38 | UnhookCargo Move/FlyDown/Unhook | 64–69 | Vehicle in vehicle Move/GetIn/GetOut-Move/GetOut/Unload-Move/Unload |
| 9 | Destroy Brown | 39–42 | HookCargo Move/FlyDown/Hook/Retry | 70 | Scripted |
| 10 | Destroy Attack | 43 | Hold Move | 71 | Logic (AND/OR) |
| 11–13 | GetIn Move/Sync/GetIn | 44 | Hold Wait | 72 | **Sync** |
| 14–16 | GetIn Nearest Move/Sync/GetIn | 45 | Hold Overlook | 73 | **Countdown** |
| 17 | SAD Move | 46 | Sentry Move | 74 | **Next** |
| 18 | SAD Check | 47 | Sentry Wait | 75 | **Unlock** |
| 19 | SAD Wait | 48 | Sentry Overlook | 76 | Flee |
| 20 | SAD Overlook | 49 | Sentry Brown | 77 | Loiter Move |
| 21 | SAD Brown | 50 | Guard Move | 78 | Loiter |
| 22–24 | Join Move/Sync/Join | 51 | Guard Wait | 79 | Follow |
| 25–27 | Leader Move/Sync/Join | 52–55 | Guard Attack/Overlook/Brown/BrownTarget | 80 | Succeed |
| | | | | 81 | Failed |

("Brown" = Brownian wandering around a point; "Overlook" = go and look at a contact.)

### 3.2 The waypoint loop [H]

```
Init.enter:   wpIndex = 1 (index 0 is the implicit start waypoint); waiting = 0  → Turn
Turn.enter (AIGroupFSM_Turn_Enter RVA 0x12969e0):
    if wpIndex >= count: → Succeed (80)
    while wp.type == CYCLE:
        j = waypoint index the CYCLE's position refers to (RVA 0x12d7020, searched among 0..wpIndex-1)
        if j < 0: log "Cycle as first waypoint has no sense" → Succeed
        reset the 'done' state of waypoints j..wpIndex-1 (RVA 0x12c20f0 with flag 1)
        run the CYCLE waypoint's activation (AIGroup_ActivateWaypoint)
        wpIndex = j
    mission.target = wp.position; mission.radius = wp.completionRadius; copy wp+0x40/+0x48
Turn.check (AIGroupFSM_Turn_Check RVA 0x129c1a0):
    apply the waypoint's settings (RVA 0x129f1a0; behaviour/speed/formation/combat [M])
    dispatch on type: MOVE→Move Move, DESTROY→Destroy Move, GETIN→GetIn Move, SAD→SAD Move,
      JOIN→Join Move, LEADER→Leader Move, GETOUT→GetOut Move, LOAD/UNLOAD/TR UNLOAD/HOOK/UNHOOK/
      VIV*→their Move states, HOLD→Hold Move, SENTRY→Sentry Move, GUARD→Guard Move,
      TALK→Talk Move, SCRIPTED→Scripted, SUPPORT→Support Move, GETIN NEAREST→GetIn Nearest Move,
      DISMISS→Dismiss Move, FOLLOW→Follow, AND/OR→Logic,
      LOITER→ Loiter Move if it is the last waypoint, else treated as Move Move,
      UNDEF→ stays in Turn
… type-specific states … → Sync (72)
Sync.enter:  run the waypoint 'arrival' hook (RVA 0x12c20f0, flag 0); if the condition/sync
             still blocks: waiting = 1 and keep the group there (AIGroup_HoldAtWaypoint)
Sync.check (RVA 0x129bac0): blocked = AIGroup_WaypointWaitCondition(wp)
             not blocked → waiting = 0 → Countdown; blocked → HoldAtWaypoint again
Countdown.enter: deadline = now + Rand_MinMidMax(timeout min, mid, max)
Countdown.check: now >= deadline → Next
Next.check:  → Unlock; AIGroup_ActivateWaypoint(wpIndex)   # statements, script, effects run HERE
Unlock.check: when AIGroup+0x200 (lock counter, lockWP) < 1: wpIndex += 1 → Turn
```

- **Condition** (`AIGroup_WaypointWaitCondition` RVA 0x129e240): if the condition text is not
  literally `true`, it is evaluated with `this` = leader person and `thisList` = all group
  units; a false result keeps the group waiting. Then waypoint synchronisation (`wp+0x18`) is
  checked (RVA 0x12bafa0). The condition is therefore evaluated **only after arrival**, every
  FSM tick while in Sync/Logic.
- **onActivation / script / effects** run in `Next`, i.e. after the condition became true
  **and** the timeout elapsed (`AIGroup_ActivateWaypoint` RVA 0x12923c0 reads `wp+0x90`,
  `+0xb0`, `+0xd8`).
- **Timeout distribution** (`Rand_MinMidMax` RVA 0x30e020): `u` = mean of four draws of the
  31-bit LCG (`x = x·1103515245 + 12345 mod 2^31`), roughly bell-shaped on [0,1];
  `u < 0.5 → min + (mid−min)·2u`, else `mid + (max−mid)·(2u−1)`. Median ≈ mid.
- **HoldAtWaypoint** (RVA 0x129f720), AI leader only: if the leader is more than
  `1.1 × max(10, 5·precision, completionRadius)` (2-D) from the waypoint, re-issue a Move;
  otherwise, when the subgroup is idle, issue a WAIT command for 600 s.
- **Logic** (AND/OR) state: leader vehicle vfunc +0x460 to the waypoint position, run the
  arrival hook, waiting = 1; check like Sync but jumps straight to Countdown.

### 3.3 Arrival test for MOVE-like waypoints — `AIGroupFSM_CheckMoveCompleted` RVA 0x129a1b0 [H]

Used (with `minRadius = 0`) by Move, Dismiss, SAD, Hold, Sentry, Join, Leader, Support, Guard
"Move" states (target state passed in).

```
leader must exist and not be busy (leader+0x128 == 0 or RVA 0x134d080)
if leader is the local player (or the player commands his vehicle):
    done = ANY group unit within max(10, 5 × precision, completionRadius) (3-D)
           (air units: also |dy| ≤ 5 × <vehicle height tolerance>)        (RVA 0x129c900)
elif side is LOGIC (center+0x20c == 7): done
elif main subgroup has no pending command (its Move command finished): done   (RVA 0x12d8070)
elif completionRadius <= 0 or group flag +0x324 == 0:
    r = max(completionRadius, minRadius, leader precision)
    done = |leaderPos − wpPos| (3-D) <= r  (air: also a height check)
on done (non-player): subgroup bookkeeping (RVAs 0x13115a0, 0x1312fa0, 0x13116c0) → next state
```

- `precision` is vfunc +0x1030 (`Man_GetPrecision` RVA 0x72ac60) = `EntityAIType+0x1554`
  (`precision`; `All` 5, `Man` 1), or 1.0 in one Man special case.
- **Default completion radius = 0**, so the AI leader must be within `precision` (1 m for a
  man, 3-D) — or, much more often, the subgroup's MOVE command ends first:
- The group's Move state issues a `Command` MOVE (type 5) to the main subgroup with
  destination = waypoint (or the house position `wp+0x4c` of the house in `wp+0x48`),
  context 6, precision `Command+0xc0` = completionRadius (enter RVA 0x1294e10). If the leader
  is > 200 m away, every subgroup unit gets a "far" flag (`AIBrain+0x245`, RVA 0x1351cf0).
- **Subgroup MOVE command** (state table 0x1420c3cf0: Init/Move/Succeed/Failed; Move
  enter RVA 0x13290f0, check RVA 0x1323020): sets the subgroup's wanted position
  (`AISubgroup+0xa8`, mode `+0xa0 = 1`) and the leader's destination with planning mode
  LEADER PLANNED and the given precision (RVA 0x1314bf0 → `AIBrain` +0x220 pos, +0x230
  precision, +0x234 mode). It succeeds when the AI leader's unit state becomes **OK (4)**
  (path finished), or for a player leader when he is within **2.5 m** (on foot) / **10 m**
  (in a vehicle), 2-D.
- "Group flag +0x324" was not identified [open].

### 3.4 Per-type behaviour [H unless marked]

- **MOVE**: Move Move → (arrival) → Sync → Countdown → Next → Unlock.
- **DESTROY**: enter sets retry counter = 5; attacks at once if the target is known. Move check:
  player leader → Attack; subgroup finished → Brown. *Attack*: target dead (`+0x5e4 & 1`) or
  fully damaged (≥ 1.0) → **Sync (complete)**; group knows target → keep updating the move point
  to its last known position; else → Brown. *Brown*: wander (radius 20 m, 0 if the target
  object has flag `+0x168 & 8`); target found → Attack; each finished brown move decrements
  the counter, at 0 → Sync (complete). So DESTROY completes when the target is destroyed or
  after 5 fruitless searches.
- **SAD**: Move → SAD Check (enter: `group+0x1ec` = now + 15 s, counter = 0) → SAD Wait.
  SAD Wait, AI leader: best target (RVA 0x129d620: nearest identified enemy, else nearest
  state-4 contact) none/dead/unknown → if counter < 5 → SAD Brown, else **Sync (complete)**;
  state-4 contact and `(dist − 400)/1200 < r` → Overlook (move to it); otherwise if no unit
  can already engage it and the leader has no attack/fire/action command → issue ATTACK (type 2)
  → SAD Check. Player leader: complete 15 s after Check unless a target is known.
  `r` = `AIGroup_RandomAggression60s` RVA 0x12e8ee0: a uniform random number re-rolled
  every 60 s (`group+0x328`, next roll `+0x32c`).
- **HOLD**: Hold Move → Hold Wait; **never completes**. In Wait: a state-4 contact with
  `(dist − 400)·0.0025 < r` → Overlook (move to it); if the leader drifts beyond
  `max(5, 3.5·precision, 1.5·completionRadius)` and no unit is within the player/AI arrival
  radius (factor 5 / 3.5) → back to Hold Move. Overlook continues while `(dist − 600)·0.0025 < r`.
- **SENTRY**: Sentry Move → Sentry Wait. In Wait: no known target and leader further than
  `max(10, 3.5·precision)` → Sentry Move; best target is an identified enemy (state ≠ 4) →
  **Sync (complete)**; a state-4 contact with `(dist − 400)/1200 < r` → Overlook (move to it).
  Overlook: when the contact is lost → Sentry Brown around the leader position (`group+0x2c8`)
  with offsets random(0, 4·v) (v = leader maxSpeed m/s) [M], up to 5 moves → Sentry Move.
  So SENTRY completes when an **identified** enemy is known to the leader; unidentified
  contacts are investigated first.
- **GUARD**: Guard Move → on arrival Guard Wait if the waypoint has guarded points/triggers
  (RVA 0x12b66b0 count > 0) else Gravon Wait (guard-by-trigger logic); a timer of
  `Rand(450, 150)` s [M: argument order] is stored. Guard states cycle Wait/Attack/Overlook/
  Brown/BrownTarget/Move and **never complete**.
- **DISMISS**: Dismiss Move → Dismiss Dismiss (enter issues the DISMISS command, type 24, to
  the units). Completes (→ Sync) when the leader is the player or **any unit's behaviour
  becomes COMBAT or STEALTH**.
- **LOITER**: only the last waypoint loiters (Loiter Move → Loiter); a non-last LOITER behaves
  as MOVE.
- **CYCLE**: handled inside Turn (above).
- **AND/OR**: Logic state (above).
- **SCRIPTED**: state 70 (enter RVA 0x12952f0, 663 lines, not read).
- **JOIN / LEADER**: Move → Sync (their own, 23/26) → Join (24/27). **SUPPORT**: Move → Wait
  (61) → Transport/Supply. GETIN/GETOUT/LOAD/UNLOAD/TR UNLOAD use Sync/GetIn/GetOut states
  (not read).
- "Contact state 4" is `Target+0x78`; read as "side not yet identified" from how
  RVA 0x129d620 prefers non-4 enemies [L].

---

## 4. The subgroup Command FSM [H]

`AISubgroup_CreateCommandFSM` RVA 0x1325b10 builds an `AISubgroupFSM`
(`FSMTyped<AISubgroupContext>`) per `Command`, from `Command+0x44` (state tables at
VA 0x1420c3748..0x1420c45xx, same 0x20-byte layout). Context +0x98 non-null with value
0x66/0x67 picks the mine tables.

| command | table VA | states |
|---|---|---|
| none / default | 0x1420c3748 | Wait |
| WAIT | 0x1420c3770 | Init, Wait, Succeed, Failed |
| ATTACK, ATTACK AND FIRE | 0x1420c37f0 | Init, FarMove, Move, Attack, Wait, RunAway, WaitAfterExplo, Succeed, Failed |
| Suppress | 0x1420c3ab0 | Init, Fire, Succeed, Failed |
| HIDE | 0x1420c3910 | Init, Move, Failed |
| MOVE | 0x1420c3cf0 | Init, Move, Succeed, Failed |
| FIRE | 0x1420c3970 | Init, Fire, Succeed, Failed |
| FIRE AT POSITION | 0x1420c39f0 | Init, Reload, Fire, FireEnd, Succeed, Failed |
| STOP | 0x1420c3d70 | Init, Stop, Succeed, Failed |
| EXPECT | 0x1420c3e10 | Init, Expect, Succeed, Failed |
| JOIN | 0x1420c4030 | Init, Succeed, Failed |
| GET IN | 0x1420c4090 | Init, Move, GetOut, Walk, Direct, GetIn, Succeed, Failed |
| GET OUT | 0x1420c4190 | Init, GetOut, Move, Succeed, Failed |
| SUPPORT | 0x1420c4230 | 4 states |
| DISMISS | 0x1420c3e90 | Init, SelectTask, Move, Relax, CleanUp, Succeed |
| HEAL, REPAIR, REFUEL, REARM, ACTION, HEAL SOLDIER, PATCH SOLDIER, FIRST AID, HEAL SELF, TAKE BAG, ASSEMBLE, DISASSEMBLE, REPAIR VEHICLE, USE CONTAINER MAGAZINE | 0x1420c3f50 | Init, Alloc, Move, Direct, Supply, Succeed, Failed |
| HOOK/UNHOOK/VIV* | 0x1420c4490 / 4570 / 42b0 / 4350 / 43f0 | not listed |
| OPEN PARA | 0x1420c3df0 | OpenParachute |
| ACTIVATE MINE (or ctx 0x66) | 0x1420c3c50 | Init, Move, Disable, Succeed, Failed |
| DISABLE MINE (or ctx 0x67) | 0x1420c3b30 | Init, MoveFar, Move, Down, MoveSlow, Disable, MoveToSafety, Succeed, Failed |
| SCRIPTED | — | `AISubgroupFSMScripted` loaded by name from `Command+0xa8` ("FSM %s not found") |

Command layout seen: `+0x44` type, `+0x48` target link, `+0x70` destination, `+0x7c` direction,
`+0xa0` house position index, `+0xb4`, `+0xb8` (2 → a flag passed to the destination setter),
`+0xbc` context (6 = issued by the mission FSM), `+0xc0` precision.
Subgroup command stack: `AISubgroup+0x48` (16-byte entries), count `+0x50`.

---

## 5. Unit-level orders and queries

- **`moveTo`** (RVA 0x47de90): if the unit state is OK (4) it is reset first (RVA 0x135fc30);
  then `AIBrain` destination = pos, precision 0, planning mode **LEADER PLANNED (2)**,
  force flag 1 (RVA 0x13601f0: writes `+0x220` pos, `+0x22c` flag, `+0x230` precision,
  `+0x234` mode, `+0x238` force, then RVA 0x135ff90 unless the unit already plans in a
  vehicle he does not command).
- **`moveToCompleted`** (RVA 0x47de00) = `AIBrain+0xd408 == OK (4)`; true for a non-AI object.
- **`moveToFailed`** (RVA 0x47de70) **always returns false** in 2.22.
- **`unitReady`** (RVA 0x569170): false if any given unit is the leader of his subgroup **and**
  that subgroup has a pending command (stack count > 0); otherwise true (also true for an
  empty input). Followers are always "ready".
- **`doMove` / `commandMove`** (RVA 0x5690d0 / 0x568340) are thin wrappers into a shared
  command builder (RVA 0x17d590, not followed).
- **`setUnitPos`** (RVA 0x53fc20): parses the mode (RVA 0x46bbf0) and calls person vfunc
  +0x1c58 via the brain (RVA 0x135fe30). The formation FSM uses the sibling vfunc +0x1c60 for
  its "weak" requests (`setUnitPosToDown` passes 2). [H] The Man keeps three requests:
  `+0x228c` (vfunc +0x1c50, not traced to a caller), `+0x2290` (`setUnitPos`, vfunc +0x1c58)
  and `+0x2294` (weak, vfunc +0x1c60); AUTO is 3. **`unitPos`** (RVA 0x533390 → vfunc +0x1c48
  = RVA 0x72c650) returns the first of the three that is not AUTO, so it reports the formation
  FSM's weak stance while the script has set none. The stance and the `disableAI` bits live in
  the unit's `ObjectState` (`docs/re/sqf-object-state.md`); the AI reads them there.
- **Behaviour of a unit** (`AIUnit_GetBehaviour` RVA 0x1347be0): `AIUnit+0x14c`; CARELESS (1)
  is sticky; otherwise raised to the group's behaviour (`AIGroup+0x1f4`) when a flag
  (RVA 0x134ba30) allows, then clamped to the min/max from vfunc +0x190.
- **AIUnit offsets**: `+0xc8` person link, `+0xd0` vehicle link, `+0xd568` subgroup link,
  `+0xd588` formation number, `+0xd58c` extra offset, `+0xd598` follow-reference unit,
  `+0xd5b0` formAngle, `+0xd5b4` formPos, `+0xd618` next-target time.
  **AISubgroup**: `+0x78/+0x80` units, `+0x90` leader, `+0x98` group, `+0xa0` mode, `+0xa4`
  refresh time, `+0xa8` wantedPosition, `+0xb4` cover flag, `+0xb8` formation, `+0xbc`
  speed mode, `+0xc4` avoidRefresh, `+0xc8` formationCoef, `+0xcc` coef time, `+0xd0`
  direction, `+0xdc` directionMove, `+0xe8` direction time.
  **AIGroup**: `+0x90` main subgroup, `+0x98` leader, `+0xa0` center, `+0xa8` command queue,
  `+0x170/+0x178` unit slots, `+0x1ec` SAD deadline, `+0x200` lock count, `+0x2b8` last target,
  `+0x2c8` brown centre, `+0x2f0/+0x2f8` waypoints, `+0x328/+0x32c` random aggression.

---

## 6. Path following (§4 of the request) — mostly open

Verified only:
- Config per type (`EntityAIType`): `steerAheadSimul` +0x1578, `steerAheadPlan` +0x157c,
  `predictTurnSimul` +0x1588, `predictTurnPlan` +0x158c, `precision` +0x1554,
  `brakeDistance` +0x1550 (RVA 0xfbe020). `All`: 0.3 / 0.4 / 1.2 / 1.0 / precision 5 /
  brake 5. `Man`: steerAhead 0.1 / 0.1 (diving 0.5 / 0.5), precision 1, brake 1. These are
  the look-ahead parameters; how they become metres was not traced (likely seconds × speed) [L].
- A destination is reached when the brain's state becomes OK; the leader's precision is the
  command precision (= completion radius) but the waypoint test also uses `type.precision`.
- The search side is in `docs/re/navigation.md`.

Not traced: per-path-point acceptance radius, replanning cadence (`refreshTime`,
`waitWithPlan`, `attemptPlan`, `lastPlan`, `noPath`, `updatePath` are the serialised
`AIBrain`/planner fields at `+0xd538..` / `+0x248..0x24c`), the man's walk/run/sprint choice from
the speed cap.

---

## 7. What to change in our implementation (suggestions)

1. Replace `FORMATION_SPACING` and the hand-made offsets with the `cfgFormations` algorithm
   (§1.2) using each type's `formationX/Z`; read the table from config by index.
2. Slots are relative to the leader's slot, rotated by the subgroup direction (reset to the
   leader's facing on `setFormation`, set by `setFormDir`), not by the leader's heading
   every frame.
3. SAFE/CARELESS: single file on the trail of the unit ahead; AWARE+ off road: geometric slots.
4. Leader speed = `formationCoef × maxSpeed`, coef slewed at 0.1/s in [0.1, 1.5]; LIMITED caps
   at `maxSpeed × limitedSpeedCoef` (not in COMBAT); FULL = 1.5 × maxSpeed.
5. Waypoint completion: AI leader within `max(completionRadius, precision)` 3-D **or** the
   subgroup's Move command finished (leader path state OK); player leader: any unit within
   `max(10, 5·precision, radius)`. Default radius 0.
6. Run condition after arrival, then the min/mid/max timeout, then statements; HOLD and GUARD
   never complete; SENTRY completes on an identified enemy; DISMISS on any unit in COMBAT;
   DESTROY on target destroyed or 5 failed searches; SAD after 5 empty brown searches.
7. `moveToFailed` is always false; `unitReady` is false only for a subgroup leader with a
   pending command.

---

## 8. What a3-rust implements (`crates/a3-world/src/ai/{formation,movement}.rs`)

| item | status |
|---|---|
| `cfgFormations` by position, fixed/pattern entries, reference slots, average `formationX/Z` per link, empty slot = 1 unit (§1.1-1.3) | done: `FormationTable::{from_config, shipped, slots}`; the shipped table equals every side's config (real-data test) |
| slot = leader's position + slot offset relative to the leader's slot, turned by the formation direction (§1.5) | done; height is the leader's, not the unit's own |
| formation direction: reset to the leader's facing by `setFormation`, set by `setFormDir`, `formationDirection` | done |
| direction while moving (§1.4, not traced) | ours (low): set towards the waypoint when the group turns to it, kept until the next one |
| leader speed control: `formationCoef` slewed 0.1/s in 0.1..1.5 towards the slowest follower, LIMITED cap (not in COMBAT), FULL 1.5x (§2.1) | done |
| `forceSpeed` (§2.2) | done (caps the pace); `limitSpeed` not registered |
| walk/run choice from the speed cap (not traced) | ours (low): below 0.3 m/s stand, below 3 m/s walk, else run; CARELESS/SAFE always walk; a follower more than two formation units behind his slot runs |
| SAFE/CARELESS trail following (§2.3) | done in outline: 10-point trail at `formationZ x 0.1` spacing, look-back `formationTime x speed x 0.6` floored at `1.5r + 2.5` (r = 0.5 m, ours), hold when the point is close and behind; the road case is not modelled |
| "too far from formation" (§2.4) | not done |
| waypoint loop: Turn (modes, direction, leader's path), arrival = leader path finished or within `max(completionRadius, precision)` 3-D, default radius 0, then the type's wait, then a `Rand_MinMidMax` countdown, then Next (§3.2-3.3) | done; the waypoint condition and statements are not run (the World has no VM; they are stored), synchronisation is not modelled |
| per type (§3.4) | MOVE and the others: arrival; HOLD, GUARD, SUPPORT and vehicle types: never; SENTRY: an identified enemy (knowledge 1.5); SAD/DESTROY: no contacts left (the five brown searches are not modelled); DISMISS: group in COMBAT |
| unit commands (§5) | `unitReady`, `moveToCompleted`, `moveToFailed` (always false), `setUnitPos`, `setUnitPosWeak`, `unitPos`, `formationPosition`, `formationLeader`, `isFormationLeader` |
| path following (§6, not traced) | ours: the leader and a unit with `doMove` plan on the `a3-nav` navigator when the World has one (straight line otherwise) and walk point to point, replanning when the goal moves by more than `precision`; followers steer straight at their slots |
