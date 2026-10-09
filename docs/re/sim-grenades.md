# Thrown grenades: throw velocity, flight, bounce and fuse

`player throw` → detonation for the throwable `CfgAmmo` classes, read from `arma3_x64.exe` (RVAs)
with the tooling in `TOOLING.md`, plus the shipped config (`a3-tools config dump --resolved`).
Extends `sim-ballistics.md` (§2 simulation enum, §3 move `0x140e66ea0`, §4 ricochet/penetration).

**Confidence.** (high) = transcribed from code with its constants. (medium) = code clear, one input's
meaning inferred. (low) = guess. "not traced" means exactly that.

## 1. Which classes, which simulation (high)

`simulation` is the enum at `AmmoType+0x498` (`sim-ballistics.md` §2). `HandGrenade`/`MiniGrenade`
are **magazines**; the ammo classes are `GrenadeHand` and `mini_Grenade` (lower-case `m`) — there is
no `CfgAmmo >> HandGrenade` in build 2.22.

| `CfgAmmo` | `simulation` | enum | class | magazine | mag `initSpeed` | mag `mass` |
|---|---|---|---|---|---|---|
| `GrenadeHand` | `shotGrenade` | 1 | `GrenadeEPE` | `HandGrenade` | 18 | 10 |
| `GrenadeHand_stone` | `shotGrenade` | 1 | `GrenadeEPE` | `HandGrenade_stone` | 18 | 10 |
| `mini_Grenade` | `shotGrenade` | 1 | `GrenadeEPE` | `MiniGrenade` | 26 | 6 |
| `SmokeShell` + colours | `shotSmokeX` | 10 | `SmokeShellEPE` | `SmokeShell*` | 22 | 4 |
| `Chemlight_*` | `shotSmokeX` (via `SmokeShell`) | 10 | `SmokeShellEPE` | `Chemlight_*` | 14 | 2 |
| `SmokeShellArty` | `shotSmoke` | 9 | `SmokeShell` (a `ShotShell`) | — | — | — |

`GrenadeHand`: `explosionTime = 5`, `timeToLive = 6`, `fuseDistance = 0`, `typicalspeed = 18`;
`SmokeShell`: `explosionTime = 2`, `timeToLive = 60`, `explosive = 0`, `hit = 0`. `airFriction =
-0.0005` and `initTime = 0` come from `CfgAmmo >> Default`.

**Throwable test** — `0x141105260(MagazineType*)`, `0x1411051e0(weaponSlot)` (high):
```
throwable := AmmoType(mag+0x28).simulation(+0x498) in {0,1,2,3,8,9,10,19}
             and mag.initSpeed(+0x84) < 30.0        // strict; == 30 is not throwable
```
The muzzle side is `CfgWeapons >> Throw` (one muzzle per magazine: `HandGrenadeMuzzle`,
`SmokeShellMuzzle`, `ChemlightGreenMuzzle`, `IRGrenade`, …), all `cursorAim = "throw"` with a
decorative `initSpeed = 75` that is **not** used for throwables.

## 2. Throw velocity (high on the formula; medium on the frame)

The magnitude is **`CfgMagazines >> <mag> >> initSpeed`** — not `CfgAmmo` `thrust`/`initTime`, not
the muzzle's `initSpeed`. `0x140faf810(unit, MagazineType, weaponSlot)`:
```
speed = MagazineType.initSpeed(+0x84)                        // 18 for HandGrenade
if AmmoType.simulation in {6 shotBullet, 7 shotSpread}:       // 0x1410f2270, high
    w = WeaponType(+0x4f4)                                   // config key not identified
    if w > 0: speed = w * coef  else if w < 0: speed = |w| * initSpeed * coef
// coef = 1.0 by default (DAT_1420c9068), from an AI skill/aim source otherwise
```
Every grenade simulation skips the `{6,7}` branch, so `speed == mag.initSpeed`. `MagazineType` comes
from the loader `0x1410f5b60`:

| `CfgMagazines` key | offset | default |
|---|---|---|
| `ammo` | +0x28 (AmmoType*) | — |
| `mass` | +0xa4 | 0 |
| `initSpeed` | +0x84 | 0 |
| `initSpeedY` / `initSpeedZ` | +0x88 / +0x8c | 0 |
| `maxThrowHoldTime` | +0xa8 | **2.0** (`0x40000000`) |
| `minThrowIntensityCoef` | +0xac | **0.3** (`0x3e99999a`) |
| `maxThrowIntensityCoef` | +0xb0 | **1.5** (`0x3fc00000`) |

Every shipped throwable magazine sets `maxThrowHoldTime = 2.0`, `minThrowIntensityCoef = 0.3`,
`maxThrowIntensityCoef = 1.4`. The launch function `0x140fa7140` then applies a **one-shot charge
multiplier** held on the unit's weapon/inventory object (`Man+0xc50`), high:
```
speed = speed * m                     // m = *(float*)(Man + 0xc50 + 0x88) = Man+0xcd8
*(float*)(Man + 0xcd8) = 1.0          // consumed: reset to 1.0 right after
```
A full `HandGrenade` throw is `18 × [0.3, 1.4] = 5.4 … 25.2 m/s` (`MiniGrenade` 7.8…36.4,
`SmokeShell` 6.6…30.8, `Chemlight` 4.2…19.6). **Where `m` is written (the hold ramp) is not
traced**; the magazine fields above are that ramp's verified inputs.

**Frame.** Direction is model-space, rotated by the thrower's world matrix (Entity `vfunc +0x1a8`,
a `Frame`: rotation +8…+0x28, translation +0x2c):
```
dir_world = R · dir_model                      // then Shot vfunc +0x138 = SetDirection
v         = speed · forward(actor)             // shot's own axis, actor+0x20/+0x24/+0x28
FUN_140b62690(shot+0x30, v)                    // PhysX body: linear velocity
FUN_140b62420(shot+0x30, rand[0..10] per axis) // PhysX body: angular velocity (tumble)
```
Which `muzzlePos`/`muzzleEnd` memory point supplies the origin is **not traced** (medium).

## 3. Flight: `GrenadeEPE` runs PhysX, not the segment move (high)

`GrenadeEPE` overrides exactly one relevant vtable slot vs `ShotShell`:

| slot | offset | `ShotShell` | `ShotEPE` | `GrenadeEPE` |
|---|---|---|---|---|
| 374 | +0xbb0 | `0x140e65000` `ShotShell::Simulate` | `0x140d92060` | **`0x140d92040`** |

`0x140d92040` = `ShotEPE::Simulate` + `ShotBulletClose_Track` on the EPE item at `Shot+0x768`.
Sizes from the factory `0x140e5b720`: `GrenadeEPE` `0x7b8` (case 1, ctor `0x140d90dd0`),
`SmokeShellEPE` `0x820` (case 10), `ShotShell` `0x800` (case 9).

`0x140d92060` begins with
`if (!Entity_IsLocal(vfunc +0x50)) { ShotShell::Simulate(0x140e65000); return; }` — a grenade owned
by **another** machine uses the old segment/ricochet/penetration path; only the local one uses PhysX.
Per local step it copies the body pose (`actor = Shot+0xd0`, position `actor+0x2c`, velocity
`actor+0x54`), snaps the body to the terrain height if it is more than 1 m below it, runs the
water/buoyancy branch (water-plane fit, drag `v²·512·k`, upward accel `depth·1024`, sets the in-water
flag `Shot+0x650` while `fuseDistance` is unspent — medium), then the §5 timer block, then syncs the
pose back (`FUN_140e8cdd0`).

**Bounce and rolling are PhysX's** for the local grenade: it never enters `0x140e66ea0`, so §4 of
`sim-ballistics.md` is *not* what makes it bounce. Where the contact restitution/friction come from
is **not traced**.

## 4. The slow-shell / rolling branch of the ricochet (`0x140e65820`) (high)

Called from the move `0x140e66ea0` as
`0x140e65820(shot, hitRecordOrNull, n, hitPos, distToContact, dtRemaining, objLimit, surfDeflect)`
where `n` = surface normal, `distToContact` = distance from the step's start to the contact (metres),
`dtRemaining = dt − distToContact/|v|`, `objLimit` = the hit object's `vfunc +0x350` (1.0 with no
object) and `surfDeflect` = `hitObject+0x48`, or `(surfaceRecord+0x68)+0x48` for the terrain/exit case
— i.e. the **surface's `deflection`** (answers the open question in `sim-ballistics.md` §8). It first
computes `maxSin = max(sin(deflecting_rad · surfDeflect), 0)` (`0x1419be240`), the jittered normal
`n'`, `sinG = −n'·v̂` and `vn = max(−(v·n), 0)`.

**Slow-shell / rolling branch** — a *live* shot about to touch something:

```
if ( (explosionTimer(+0x648) < FLT_MAX and explosionTimer > 0)      // running fuse
     or fuseDistance(+0x64c) > 0 or inWater(+0x650) )
   and ( flag5ee(+0x5ee) == 0 and vn < 2.0 and distToContact < 0.1 ):
        if distToContact > 0.01: Shot vfunc +0xd18(shot, 1)         // "rolling" notification
        v -= (v·n)·n                                               // lose the normal component
        f = clamp01(-100 · cos(v, n))                               // 0 at a 0.57° graze, 1 beyond
        dv_i = −(0.1·v_i + 3·sign(v_i)) · f                         // per axis, opposing motion
        v += dv_i · dt_remaining                                   // clamped: v never reverses
        return 0x140e66ea0(shot, dt_remaining, ...)                 // continue the move, sliding
```

`cos(v, n)` is the normalised cosine `FUN_14035a2b0` returns (`v·n / (|v||n|)`), so `f` is 1 for
anything but a shot skimming the surface within half a degree; `vn = max(−(v·n), 0)` is the
**speed** along the normal and is what the `< 2.0` gate tests.

So a **fused** shell that touches a surface with less than 2 m/s along the normal **slides**: the
normal component is removed and a friction of `3 m/s²` Coulomb plus `0.1·|v| /s` viscous opposes it.
Shots without a fuse (bullets) skip this. `distToContact < 0.1 m` is the contact test; the
`> 0.01 m` guard only gates the notification.

**Fast ricochet branch** — `sim-ballistics.md` §4.1 is confirmed verbatim; three additions: the speed
test is `|v|² > 25` (`|v| > 5 m/s`), the reflected velocity is further scaled by the value the hit
handler `vfunc +0xd08` returns in its `&outScale` argument, and the surface coefficient is the one
named above. Shots without a fuse (bullets) never take the sliding branch. Neither branch reverses
the velocity, and a shot whose `v·n > 0` after reflection falls through to the penetration path
`0x140e69b00`.

## 5. Fuse (high)

`ShotShell`'s ctor `0x140e513d0` copies the ammo's timers into the shot **once, at creation** — the
fuse starts at the throw, not at the pin pull:

| shot offset | from `AmmoType` | meaning | `GrenadeHand` |
|---|---|---|---|
| +0x5e8 | +0x3f0 `timeToLive` | hard lifetime (`Shot` ctor `0x140e50ee0`) | 6 s |
| +0x644 | +0x3a0 `initTime` | delay before the timers tick | 0 s (`Default`) |
| +0x648 | +0x3b0 `explosionTime` | **the fuse** | 5 s |
| +0x64c | +0x3b4 `fuseDistance` | distance budget | 0 m |
| +0x640 / +0x63c | +0x3f8 / +0x3fc | air / water friction | −0.0005 |
| +0x650, +0x651, +0x5ec, +0x5ee, +0x760, +0x761 | — | in-water, water-hit, dead, force-explode, landed, exploded | 0 |

Each step (`0x140d92060`; the same block is in `0x140e65000`):

```
if dead(+0x5ec): return
initTime(+0x644) -= dt
if initTime <= 0:
    if timeToLive(+0x5e8) < FLT_MAX: timeToLive -= dt;  if timeToLive < 0: dead = 1; return
    if explosionTime(+0x648) < FLT_MAX: explosionTime -= dt
    if exploded(+0x761) == 0 and ( explosionTime <= 0
                                   or (flag452 & 1 and explosionTime >= FLT_MAX)
                                   or (y <= waterY and landed(+0x760) and explosionTime >= FLT_MAX)
                                   or forceExplode(+0x5ee) != 0 ):
        exploded = 1
        FUN_140e5bf10(shot, actor+0x2c /*position*/, actor+0x54 /*velocity*/)   // effects
        Shot vfunc +0xd08(shot, 0, 0, position, zero, zero, 1)                  // world explosion
fuseDistance(+0x64c) -= |Δposition|            // floored at 0, every step
```

- The explosion happens **at the projectile's own position with its current velocity** — not at the
  ground, not snapped to a surface.
- `timeToLive` expiring **deletes** the shot silently (no explosion): 6 s is only a backstop behind
  the 5 s fuse; `SmokeShell`'s 60 s is what eventually removes the shell.
- `fuseDistance` is a *distance* spent by path length travelled and is **not** a gate in the
  explosion condition; its only reader is the water branch above. With `fuseDistance = 0` (all
  grenades) it never applies.
- **The fuse is independent of impacts**: the timer block runs every step whether the grenade flies,
  bounces or rests, so a bounced grenade's charge keeps burning down.

## 6. Cook-off / held fuse (medium — inference, not traced)

The fuse lives on the `Shot` created by the launch function `0x140fa7140`, and nothing else
decrements `AmmoType+0x3b0`, so an exploding-in-hand grenade **cannot** be modelled by the fuse code
as traced: a grenade is harmless while held. The only hold mechanics found are `maxThrowHoldTime`
(`MagazineType+0xa8`, 2.0 s) and the intensity coefficients — throw-charge data, not a fuse. Whether
the throw is forced past `maxThrowHoldTime`, and where that ramp lives, is **not traced**;
`CfgMagazines >> canThrowAway` and the `UAThrow`/`CycleThrownItems` actions were not decoded.

## 7. `GrenadeEPE` mass and geometry (mostly not traced)

- Shape = the ammo's model: the `Shot` ctor passes `AmmoType+0x2e8` (the loaded `model` shape) and
  logs `"No shape for ammo type %s"` when null (`GrenadeHand`: `\A3\Weapons_f\ammo\Handgrenade_throw`).
  The body is an `EPEItem` made by the `GrenadeEPE` ctor at `Shot+0x768` (`0x140900430`, kind 10) —
  the handle `Simulate` drives; `Physx3Grenade` (RTTI vtable `0x1b44b30`) is the PhysX-side object.
- **Not traced:** which LOD becomes the PhysX convex (Geometry / Fire Geometry / `physx` components),
  the body mass and inertia, whether grenade-vs-world contacts use Fire Geometry, and the
  restitution/friction pair (see also `physics-collision.md`). `CfgAmmo >> GrenadeHand` has **no
  `mass` entry** (checked in the resolved dump); the magazine's `mass = 10` is inventory loadout
  mass. Bounce heights cannot be reproduced from this document yet.

## 8. Open points

- The writer of the throw charge at `Man+0xcd8` (`maxThrowHoldTime` / intensity ramp) and the exact
  release-time frame where `m = 1.0`.
- Which muzzle memory point supplies the throw origin; whether the thrower's own velocity is added.
- PhysX geometry LOD, mass, restitution and friction for `GrenadeEPE` (§7); `WeaponType+0x4f4`'s
  config key and the AI coef `pfVar7` in `0x140faf810`.
- The `Shot` vfuncs `+0xd00`, `+0xd18`, `+0x1d0` and the flags `+0x452 & 1`, `+0x5ee`, `+0x760`.
- Grenades thrown from a moving platform, and the AI path (`RadioMessageThrowingGrenade`, `UAThrow`).
