# Projectiles, ballistics, hits and damage

How Arma 3 2.22 flies projectiles (`CfgAmmo`), detects hits, ricochets and penetrates, and turns
hits into `damage` and hit-point damage. Source: `arma3_x64.exe` (RVAs below), read with the
Ghidra tooling in `docs/re/TOOLING.md`. The class and entity background is in
`world-object-model.md` and ADR 0006.

**Confidence.** Formulas marked (high) are transcribed from the decompiled code with their
constants. (medium) means the formula is certain but the meaning of an input is inferred.
(low) is a guess.

## 1. `CfgAmmo` → `AmmoType` (loader `0x141105920`, high)

| config entry | `AmmoType` offset | notes |
|---|---|---|
| `hit` | +0x35c | |
| `indirectHit` | +0x360 | |
| `indirectHitRange` | +0x364 | |
| `explosionEffectsRadius` | +0x368 | |
| `explosionForceCoef` | +0x370 | |
| `cost` | +0x3b8 | |
| `simulationStep` | +0x3c0 | per-entity step, see `world-object-model.md` |
| `explosive` | +0x3e0 | 0..1 |
| `caliber` | +0x3e4 | penetration power, §4 |
| `deflecting` | +0x3e8 | degrees in config, **stored in radians** |
| `deflectionSlowDown` | +0x3ec | |
| `timeToLive` | +0x3f0 | ≤ 0 → infinite |
| `minTimeToLive` | +0x3f4 | |
| `airFriction` | +0x3f8 | negative in config (e.g. −0.0012) |
| `waterFriction` | +0x3fc | |
| `coefGravity` | +0x400 | |
| `typicalSpeed` | +0x404 | damage reference speed, §5 |
| `hitOnWater` | +0x409 | |
| `deflectionDirDistribution` | +0x47c | |
| `penetrationDirDistribution` | +0x480 | |
| `thrust`, `thrustTime`, `initTime` | +0x3a8, +0x3a4, +0x3a0 | rockets/missiles |
| `sideAirFriction` | +0x3ac | missiles |
| `maneuvrability`, `trackOversteer`, `trackLead` | +0x37c, +0x380, +0x384 | missiles |
| `maxSpeed`, `explosionTime`, `fuseDistance` | +0x3bc, +0x3b0, +0x3b4 | `explosionTime` ≤ 0 → never |
| `simulation` | +0x498 | enum, §2 |

## 2. Simulation kinds (high)

`simulation` is hashed case-insensitively (FNV-1a 64; `0x1410eb620`). The resulting enum
(`AmmoType+0x498`) selects the C++ class in `0x140e5b720`:

| value | `simulation` | class |
|---|---|---|
| 0 | `shotShell` | `ShotShell` |
| 1 | `shotGrenade` | `GrenadeEPE` (PhysX body) |
| 2 | `shotSubmunitions` | `ShotSubmunitions` |
| 3 | `shotDeploy` | `ShotDeploy` |
| 4 | `shotMissile` | `Missile` |
| 5 | `shotRocket` | stored as 4 (`Missile`) |
| 6 | `shotBullet` | `ShotBullet` |
| 7 | `shotSpread` | `ShotSpread` (derived from `ShotBullet`) |
| 8 | `shotIlluminating` | `Flare` |
| 9 | `shotSmoke` | `SmokeShell` (a `ShotShell`) |
| 10 | `shotSmokeX` | `SmokeShellEPE` (PhysX) |
| 11 | `shotTimeBomb` | error "Time bomb no longer supported" |
| 12 | `shotDirectionalBomb` | `DirectionalBomb` |
| 13, 14 | `shotPipeBomb`, `shotMine` | `Mine` |
| 15 | `shotBoundingMine` | `BoundingMine` |
| 16 | unknown name | |
| 17 | `laserDesignate` | |
| 18 | `shotCM` | `CounterMessure` |
| 19 | `shotNVGMarker` | `MarkerLightShell` |
| 20 | `shotLaser` | `ShotLaser` |
| 21 | empty or unknown | default |

`ShotBullet` → `ShotShell` → `Shot` → `Entity`. Projectiles live in the world's fast-vehicle list
(see `world-object-model.md`).

## 3. Flight (high)

`ShotShell::Simulate` is `0x140e65000`; `ShotBullet` (`0x140e64d00`) adds tracer/sound
bookkeeping. Each step `dt`:

- Timers count down:
  - the arming delay at `+0x644`;
  - `timeToLive` at `+0x5e8`: at ≤ 0 the shot is deleted;
  - the explosion timer at `+0x648`: at ≤ 0 the shot explodes.
- The move is `0x140e66ea0`:
  1. If the friction coefficient is below −0.99, `dt` is limited to `4/|v|`.
  2. Segment test from `p` to `p + v·dt` against terrain (`0x1412236c0`) and objects
     (`0x141224ea0`). The object test uses the LOD that the shot asks for via `vfunc +0x1b0`,
     i.e. **Fire Geometry**. The filter ignores the shot itself and its shooter, and also the
     shooter's vehicle when the shooter is inside one. The nearest hit wins.
  3. With no hit, the shot moves to `p + v·dt`, then velocity is updated (`0x140e6c3d0`):

```
k   = airFriction            // waterFriction (+0x63c) while underwater
a   = k·|v|·v − (0, 9.8066·coefGravity, 0)          // y is up
dv  = a·dt
if |dv|² ≥ |v|²:  dv = dv · |v|/|dv|                  // never reverses the velocity
v  += dv
```

This is explicit Euler: position first with the old velocity, then velocity. Velocity is only
updated when `0 < factor ≤ caliber·1000`; the plain move passes `factor = 1`, so a shot with
`caliber < 0.001` never changes speed. When `|v| = |dv| = 0` (a shot at rest without gravity) the
clamp divides 0 by 0 and the original gets NaN; we leave the velocity at zero.

`0x140e6c3d0(shot, dt, to, factor)` is the one place a shot moves (high):
- it subtracts the distance moved from the remaining arming distance (`fuseDistance`, `+0x64c`)
  while that is positive;
- it puts the shot at `to`;
- it steps the velocity with `dt` as above (when `factor` allows);
- for ammo with the flag at `AmmoType+0x3c7` it turns the shot to face its velocity.

**The move in detail** (`0x140e66ea0(shot, dt, continuing, …)`, high):
1. `dt ≤ 0` returns. With `k < −0.99`, `dt = min(dt, 4/|v|)`; the rest of the frame step is
   simply not flown.
2. `end = p + v·dt`. If `|v|·dt ≤ 0` the shot only steps its velocity.
3. `continuing` is set for the rest of a step after a ricochet or a penetration: the object test
   then starts 0.1 m along `v̂` from `p` (so it does not meet the surface it left), and a terrain
   hit closer than 0.1 m is ignored.
4. Terrain test from `p` to `end`; object test (Fire Geometry, the filter above) from the start
   to `end`. An object hit counts only when it is nearer than the terrain hit (distances measured
   from their own start points). Intersection records on a surface with `thickness > 0` count
   only with their flag bit0 set (the entry face).
5. **Object hit** at distance `d` (+0.1 when continuing): `t = min(d/|v|, dt)`;
   `0x140e6c3d0(t, hitPoint, 1)` — **the velocity steps for `t` before the impact**, so `v_in` is
   the velocity at the hit point, not the muzzle velocity. Then ricochet (§4.1, with the rest
   `dt − t`), else penetration (§4.2), else stop (§4.3).
6. **Terrain hit** (no object hit): the same move to the hit point, then ricochet with the
   ground's `deflection` and `objLimit = 1`, else stop. The terrain is never penetrated. Water
   (sea surface) hits take a separate branch (a splash; the shot switches to `waterFriction`)
   that we do not model yet.
7. No hit: `0x140e6c3d0(dt, end, 1)`.

**Missiles** (`Missile::Simulate` `0x140e63790`, medium) work in model space. Lateral drag per
axis is `((|u|·u + u)·10 + u³·0.0005)·sideAirFriction`, and axial drag is
`(|w|·w·0.01 + w³·1e-5 + 2w)·airFriction`, both scaled by a mass-like factor. Gravity is
`9.8066·coefGravity`. Thrust and guidance (`thrust`, `thrustTime`, `maneuvrability`, the
lock types) are applied in helpers that have not been decoded yet.

A second pass over the same function (2026-10-09) confirms the constants and pins the shape of
the step; the *inputs* stay (medium) because the fields they read are not yet named:

- The three drag terms are computed from three floats of the shot's block
  (`+0x60`, `+0x64`, `+0x68`; with `pfVar10 = block + 0x2c` being the position these read like a
  **model-space velocity**, medium). Lateral (`+0x60`, `+0x64`) is scaled by `sideAirFriction`
  (`AmmoType+0x3ac`), axial (`+0x68`) by `airFriction` (`AmmoType+0x3f8`); all three are then
  multiplied by `k·0.1`, so the implemented scale is **`0.1·dt`** if `k` is the step.
- `k = FUN_140e85230(shot)`: the shot's `+0x88` read as an int when `vfunc +0x1d0` (the PhysX
  body test) answers yes, else `shooter(+0xc8) + 0x5c8`, else 0. Unnamed; it also scales the
  gravity term (`− k·9.8066`), which is what makes "k is the step" (medium) plausible.
- The acceleration is rotated from model to world space by a 3×3 matrix at `block + 8 …
  block + 0x28` (the missile's orientation), then handed to `FUN_140e59ab0(shot, &accel, &vel,
  arg, k)`, which is where the shooter's velocity (`+0x54`) and the shot's `+0x740` enter.
- `FUN_140e59d50(shot, &accel, &vel, lockType == 0x40)` is thrust and guidance. It is called only
  when the shot's lock state (`+0xea`) is not 1 **and** the axial speed is at least 30 m/s.
- `FUN_1410f5830(AmmoType)` returns the lock type. `0x40` takes a different drag branch
  (`−0.03·|u|·u` lateral, `−0.005·w − 0.00033·|w|·w` axial, both scaled by `k`), the "advanced"
  model some missiles use.

Open: what `+0x60/+0x64/+0x68` hold exactly, what `k` is, the thrust and guidance law inside
`0x140e59d50` (and its `maneuvrability`, `trackOversteer`, `trackLead` inputs), the lock types of
`0x1410f5830`, and `initTime`/`thrustTime`.

## 4. Impact: ricochet → penetration → stop (high)

When the segment hits something, the shot advances to the hit point and then tries the three
outcomes in order. `n` is the surface normal and `v̂ = v/|v|`.

### 4.1 Ricochet (`0x140e65820`)

```
maxSin = max(sin(deflecting · surfDeflect), 0)        // deflecting in radians
if objLimit > 0 or maxSin > 0:
  n'   = normalize(n + R(−d, 0, d) per axis, x then y then z)   // d = deflectionDirDistribution
  sinG = −n'·v̂                                          // sine of the grazing angle
  if 0 ≤ sinG < maxSin and sinG < objLimit and |v|² > 25:
    v' = v − 2(v·n')n'
    if v'·n > 0:                                         // the unrandomized normal
        k  = min(max(1 − (sinG/maxSin)², 0), deflectionSlowDown) · R(0.6, 0.9, 1.0)
        v' = v'·k
        apply hit damage with v_in = v, v_out = v'      // §5 (the hit handler may scale v')
        v  = v'
        move(remaining dt, continuing = true)           // recursive 0x140e66ea0
```
`R(min, mid, max)` is `Rand_MinMidMax` (`0x14030e020`, high): four draws of the global 31-bit LCG
`x = (x·0xC1C64E6D + 0x3039) & 0x7fffffff` (state `0x142165668`) are averaged to `f` in 0..1 (a
bell around 0.5), then `f < 0.5 → min + (mid − min)·2f`, else `mid + (max − mid)·(2f − 1)`. So a
ricochet keeps 60–100 % of its speed with a median of 90 %; the earlier reading "U(0.6, 0.9)"
was wrong (the third argument, 1.0, is passed in XMM3).
- `surfDeflect` is the surface's deflection coefficient (the hit surface record `+0x48`; for
  terrain, the ground surface). The bisurf/CfgSurfaces entry `deflection` is loaded in
  `0x1410dfd80`. Medium confidence on which of these two fields is used here.
- `objLimit` comes from the hit object (`vfunc +0x350`, not decoded; `a3-world` uses 1 for every
  Object, so the surface alone decides). For the terrain it is 1.
- A slow-moving shell (normal speed < 2 m/s) with a fuse, or one already rolling, instead loses
  its normal velocity and slides. This is grenade/shell rolling.

### 4.2 Penetration (`0x140e69b00`)

Surface data (bisurf / `CfgSurfaces`, loader `0x1410dfd80`):
- `R = 1e6 / bulletPenetrability`. `bulletPenetrabilityWithThickness` takes precedence if present.
- `thickness` is in mm, stored ×0.001 as metres (default −1).

```
requires a hit Object (not the terrain), a surface, R > 0 and (explosive < 0.7 or R ≤ 100)
L = length of the ray inside the hit component        // from the intersection record
if thickness > 0:
    if record flag bit0: L = |thickness / (n·v̂)|  (L_geom instead if flag bit1 and L_geom ≤ L)
    else:                R = 0.01    // unreachable from the move: such records are skipped
                                     // as the nearest hit (step 4)
loss = (R / caliber) · L                               // m/s
if loss < |v|:
    exit = hit + v̂·L
    v̂'   = normalize(v̂ + (R(−d,0,d) drawn for z, then y, then x)·loss/|v|)
                                     // d = penetrationDirDistribution
    v'   = v̂' · (|v| − loss)
    t    = min(L/|v|, remaining dt)
    0x140e6c3d0(t, exit, R)          // velocity steps only if R ≤ caliber·1000; overwritten below
    apply hit damage with v_in = v, v_out = v'         // §5 (the hit handler may scale v')
    v = v'
    move(remaining dt − t, continuing = true)
else: stop (fall through to 4.3)
```
In other words, `bulletPenetrability` is the depth in mm that a caliber-1 projectile goes
through at 1000 m/s, and `caliber` scales that depth linearly.

### 4.3 Stop

If the shot neither ricochets nor penetrates:
- when it is armed — no explosion timer running (`+0x648` ≥ FLT_MAX or ≤ 0), no arming distance
  left (`+0x64c` ≤ 0) and the flag at `+0x650` clear, or forced by `+0x5ee` — an explosive shot
  (`explosive > 0`) that has not exploded yet explodes (`0x140e5bf10` for the effects, and the
  world explosion for the damage), and the hit handler deals the direct hit with `v_out = 0`;
- in every case the shot is then stopped and deleted (`0x140e583d0`). An unarmed fused shell
  (inside its `fuseDistance`) is a dud.

## 5. Direct hit value (high)

The hit handler (`ShotShell` `vfunc +0xd08`, `0x140e664e0`) computes an energy factor for
projectiles with `explosive < 1`:

```
e = min((|v_in| − |v_out|) / typicalSpeed · shotCoef, 2) · (1 − explosive)
if the shot stopped (v_out ≈ 0) and it has no fuse: e += explosive
```
`shotCoef` is `Shot+0x638`, normally 1 (medium).

The world then applies `hit · e` to the object (`0x14121bcd0` → `0x141028440`):
- The hit-point "radius" for a direct hit is `rd = 0.27·sqrt(hit·e)`. It is stored negated to
  mark the hit as direct.
- The value passed on is `D = hit·e / armor`, where `armor` is the vehicle `armor` and
  `1/armor` is `type+0x150c`.

So a bullet that passes through and keeps most of its speed does little damage, and a stopped
bullet deals up to 2× `hit` at typical speed.

**Implementation note (`a3-world`).** `R` is read as the model's ODOL
`ModelInfo::bounding_sphere` (`shape+0x7c`, medium on the identification;
`a3_physics::LayerShape::model_bounding_sphere`). An MLOD model stores none; for a direct hit
its layer's extent stands in, an explosion treats it as a point.

## 6. Indirect hits / explosions (high on formulas)

The world explosion call gets `indirectHit · factor` and the radius `r = indirectHitRange`. It
searches objects within **4·r** (`0x14121bcd0` → `0x14121cd30`). For each object, with `R` its
bounding radius (`shape+0x7c`) and `d` the distance from the explosion to the object centre:

```
f(x) = x² ≤ r² ? 1 : r⁴ / x⁴                            // inverse 4th power outside r
fn = f(max(d − R, 0)),  fc = f(d),  ff = f(d + R)
total += 0.33·(fn + fc + ff) · D_ind · typeExplosionCoef  // only if D_ind·fn > 0 (or > 0.001)
```
`typeExplosionCoef` is `+0x2a8` of the object's type component (`vfunc +0x640`). If the
accumulated total is below `+0x2a4` of that component, the total is zeroed.

**Total damage from a direct hit:** `total += (rd/R)² · D` (EntityAI `vfunc +0x7c0`,
`0x1410285b0`). The result is capped at 2000 before the per-hitpoint step.

## 7. Hit points (`CfgVehicles >> HitPoints`)

### 7.1 Config (loader `0x140e70c20`, high)

| entry | offset | default | notes |
|---|---|---|---|
| `armor` | +0xc | | `armor ≥ 0` → `armor × vehicle armor`; **negative → absolute value** |
| — | +0x14 | | `1/armor` |
| `radius` | +0x10 | −1 | |
| `passThrough` | +0x18 | | |
| `explosionShielding` | +0x1c | 1.0 | |
| `minimalHit` | +0x20 | 0.01 | |
| `visual`, `name` (HitPoints-LOD selection), `convexComponent`, `armorComponent`, `simulation` | | | also loaded |

### 7.2 Distribution (`0x140e8c130`, high)

This runs for every hit (direct and indirect) against the **HitPoints LOD**. The input value is
`H = D·armor` (back to raw `hit` units). The radius is:
- direct: `r' = 0.25·rd·g`, where `g` is a global tunable from `0x1406ecc00`, not identified;
- indirect: `r' = indirectHitRange`.

For each hit point with a selection:
- `shield` = 1 for direct hits, `explosionShielding` for indirect hits.
- `vfunc +0xb70` adds per-hitpoint extra armour `ea` and a pass-through multiplier `pm`. This is
  where wearable armour (vests, helmets) plugs in (medium).

Hit point with `radius ≥ 0` (point cloud):
```
for each vertex v of the selection: d² = |v − hitPos|²
    f_v = d² ≤ radius² ? 1 : d² ≤ r'² ? 1 : r'⁴/d⁴
f   = max_v f_v
cov = min(N_vertices · (radius/r')³ · f · 0.0596831, 1)     // 3/(16π)
dmg = f · H · shield / (armor + ea)   // the division applies when hit-info flag +0x6c is set
                                       // (normal case); without it dmg = f·H
```

Hit point with `radius < 0` (convex component):
- `f` is the mean of `r'⁴/d⁴` (or 1) at the selection's minimum, mean and maximum vertex
  distances.
- Coverage uses the selection's bounding radius, at least 5 % of the object size:
  `cov = min((ρ/r')³·f·0.0597, 1)`.

Then:
- **`minimalHit`.** If `minimalHit ≥ 0`, the damage applies only when `dmg ≥ minimalHit`. If it
  is negative, the damage applies only when `dmg > −minimalHit`, and is reduced by
  `−minimalHit`.
- **`passThrough`.** It scales the **total** damage:
  `factor = min over hitpoints of (pm·passThrough·cov + (1 − cov))`. The total from §5/§6 is
  multiplied by this factor. A hit point with `passThrough = 0` that fully covers the hit absorbs
  it from the total.

### 7.3 Applying (`0x140e74a00`, `0x14102cb00`, high)

- The new total and the per-hitpoint deltas first go through the `HandleDamage` event (event id
  30; invoker `vfunc +0xb00`, `0x140fb6590`). The returned value replaces the delta.
- They are then stored. Total `damage` is kept at `Object+0xf0`, capped at 1000 when stored and
  read back clamped to 1.
- **Destroyed** means `damage ≥ 1.0` (`0x14013df60`). `EntityAI` is also destroyed when flag
  `+0x5e4 & 1` is set (`0x14013df30`).
- `vfunc +0xb60`, `+0xb88` and `+0xb80` then update dependent state (destruction effects, fuel,
  …). How hit-point damage feeds back into total damage — `depends`, fatal head/hull hits — has
  not been traced yet.
- Damage values are stored behind anti-tamper wrappers (XOR plus a checked function pointer).
  This does not affect the formulas.

## 8. Open points

- Missile thrust and guidance; submunitions and deploy timing.
- Hit-point dependencies: total damage from hit points, `depends`, the fatal Man hit points —
  now traced, see `sim-damage.md`.
- `g` in §7.2, `shotCoef`, and the type component behind `vfunc +0x640` (`+0x2a4`, `+0x2a8`).
- Which surface field feeds the ricochet coefficient; the meaning of the penetration component
  flags.
- Simulation enum value 16.
