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
updated when `0 < fraction ≤ caliber·1000`; in practice this is always true.

**Missiles** (`Missile::Simulate` `0x140e63790`, medium) work in model space. Lateral drag per
axis is `((|u|·u + u)·10 + u³·0.0005)·sideAirFriction`, and axial drag is
`(|w|·w·0.01 + w³·1e-5 + 2w)·airFriction`, both scaled by a mass-like factor. Gravity is
`9.8066·coefGravity`. Thrust and guidance (`thrust`, `thrustTime`, `maneuvrability`, the
lock types) are applied in helpers that have not been decoded yet.

## 4. Impact: ricochet → penetration → stop (high)

When the segment hits something, the shot advances to the hit point and then tries the three
outcomes in order. `n` is the surface normal and `v̂ = v/|v|`.

### 4.1 Ricochet (`0x140e65820`)

```
maxSin = max(sin(deflecting · surfDeflect), 0)        // deflecting in radians
n'     = normalize(n + U(±deflectionDirDistribution) per axis)
sinG   = −n'·v̂                                         // sine of the grazing angle
if 0 ≤ sinG < maxSin and sinG < objLimit and |v| > 5 m/s:
    v' = reflect(v, n')
    if v'·n > 0:
        k  = min(max(1 − (sinG/maxSin)², 0), deflectionSlowDown) · U(0.6, 0.9)
        v' = v'·k
        apply hit damage with v_in = v, v_out = v'      // §5
        continue the remaining dt with v'
```
- `surfDeflect` is the surface's deflection coefficient (the hit surface record `+0x48`; for
  terrain, the ground surface). The bisurf/CfgSurfaces entry `deflection` is loaded in
  `0x1410dfd80`. Medium confidence on which of these two fields is used here.
- `objLimit` comes from the hit object (`vfunc +0x350`).
- A slow-moving shell (normal speed < 2 m/s) with a fuse, or one already rolling, instead loses
  its normal velocity and slides. This is grenade/shell rolling.

### 4.2 Penetration (`0x140e69b00`)

Surface data (bisurf / `CfgSurfaces`, loader `0x1410dfd80`):
- `R = 1e6 / bulletPenetrability`. `bulletPenetrabilityWithThickness` takes precedence if present.
- `thickness` is in mm, stored ×0.001 as metres (default −1).

```
requires R > 0 and (explosive < 0.7 or R ≤ 100)
L = length of the ray inside the hit component        // from the intersection record
if thickness > 0:
    if component flag bit0: L = thickness / |n·v̂| (clamped to the segment if flag bit1)
    else:                   R = 0.01                   // literal; meaning unclear (low)
loss = (R / caliber) · L                               // m/s
if loss < |v|:
    exit = hit + v̂·L
    v̂'   = normalize(v̂ + U(±penetrationDirDistribution)·loss/|v|)
    v'   = v̂' · (|v| − loss)
    apply hit damage with v_in = v, v_out = v'         // §5
    continue the remaining dt from exit
else: stop (fall through to 4.3)
```
In other words, `bulletPenetrability` is the depth in mm that a caliber-1 projectile goes
through at 1000 m/s, and `caliber` scales that depth linearly.

### 4.3 Stop

If the shot neither ricochets nor penetrates:
- an explosive shot (`explosive > 0`) explodes (`0x140e5bf10` for the effects, and the world
  explosion for the damage);
- otherwise it deals its direct hit and is deleted.

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
- Hit-point dependencies: total damage from hit points, `depends`, the fatal Man hit points.
- `g` in §7.2, `shotCoef`, and the type component behind `vfunc +0x640` (`+0x2a4`, `+0x2a8`).
- Which surface field feeds the ricochet coefficient; the meaning of the penetration component
  flags.
- Simulation enum value 16.
