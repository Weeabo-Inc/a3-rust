# Weapons: config, firing, reload, Fired/HitPart events

How Arma 3 2.22 turns `CfgWeapons`/`CfgMagazines` into runtime types, fires one round from a
muzzle, gates the rate of fire, reloads magazines and raises the `Fired`/`FiredMan`/`HitPart`
events. Source: `arma3_x64.exe` (VAs below), read with the Ghidra tooling in `docs/re/TOOLING.md`.
Projectile flight, impact and damage are in `sim-ballistics.md`. This file does not repeat them.

**Confidence.** (high) = transcribed from decompiled code with its constants. (medium) = the
formula is certain but the meaning of an input is inferred. (low) = a guess.

**Names used below.**
- `WeaponType`: one `CfgWeapons` class.
- `MuzzleType`: one muzzle of a weapon. Size 0x618; the laser variant `MuzzleTypeLaser` is 0x650.
- `WeaponModeType`: one fire mode. Size 0xc0.
- `MagazineType`: one `CfgMagazines` class.
- `Magazine`: a magazine instance with its round count.
- `WeaponsState`: the weapon state of one turret or soldier.

A **weapon slot** is one `(mode, weapon, muzzle, MuzzleState)` record of 0x28 bytes. The slots
are stored in `WeaponsState+0x130`, and their count is at `+0x138`.

## 1. Config loaders

### 1.1 `WeaponModeType` (loader `0x1410fab80`, high)

`modes[]` of a muzzle is read in the muzzle loader. For each mode name there are two cases:
- If the name is `"this"`, the mode is loaded from the **muzzle class itself**.
- Otherwise the mode is loaded from the subclass of that name.

The loader also stores a name ID at `+0x94`. *Required* means the value is read with the
mandatory getter: the engine supplies no default, and the value comes from config inheritance
(the `Default`/`Mode_SemiAuto`… base classes).

| entry | offset | type | default |
|---|---|---|---|
| (config class) | +0x10 | entry ref | |
| `displayName`, `textureType` | +0x18, +0x20 | string | required |
| `multiplier` | +0x28 | int | required |
| `burst` | +0x2c | int | required |
| `burstRangeMax` | +0x30 | int | **−1** (absent) |
| `salvo` | +0x34 | bool | 0 |
| `soundTypeIndex` | +0x38 | int | 0 |
| `sounds[]` | +0x40 | array of sound sets, 0xb0 each; each name is read as a subclass | |
| `reloadTime` | +0x58 | float s | required |
| `requiredOpticType` | +0x5c | int | −1 |
| `recoil` | +0x60 | recoil ref | required |
| `recoilProne` | +0x68 | recoil ref | required; **if empty, it falls back to `recoil`** |
| `aiRateOfFire`, `aiRateOfFireDistance` | +0x70, +0x74 | float | required |
| `aiRateOfFireDispersion` | +0x78 | float | 0 |
| `soundContinuous`, `soundBurst` | +0x7c, +0x7d | bool | required |
| `autoFire` | +0x7e | bool | required |
| `useAction` | +0x7f | bool | required |
| `showToPlayer` | +0x80 | bool | required |
| `aiBurstTerminable` | +0x81 | bool | `!showToPlayer && burst > 3` |
| `useActionTitle` | +0x88 | string | |
| `artilleryCharge` | +0x90 | float | required |
| `minRange`, `minRangeProbab`, `midRange`, `midRangeProbab`, `maxRange`, `maxRangeProbab` | +0x98 … +0xac | float | required |
| derived slope, lower range | +0xb0 | `(midP − minP)/(mid − min)`; when `mid ≤ min` the reciprocal is 1e10 |
| derived slope, upper range | +0xb4 | `(midP − maxP)/(mid − max)`; when `max ≤ mid` the reciprocal is −1e10 |
| `dispersion` | +0xb8 | float rad | required |
| `artilleryDispersion` | +0xbc | float | required |

`aiDispersionCoefX/Y` belong to the muzzle (§1.2), not to the mode.

### 1.2 `MuzzleType` (loader `0x1410f75b0`; shape part `0x141101d00` → `0x1410fee20`; high)

Weapon-level entries (`WeaponType` loader `0x1410fbd10`):

| entry | offset | default | notes |
|---|---|---|---|
| `initSpeed` | WeaponType+0x4f4 | **−1** | §2.3 |
| `inertia` | +0x4f0 | 0 | |
| `aimTransitionSpeed` | +0x4ec | 1 | |
| `swayCoef` | +0x510 | | |
| `muzzles[]` | +0x138 | | Same `"this"` rule as modes. A muzzle class with `laser = 1` creates a `MuzzleTypeLaser`. |

Muzzle entries:

| entry | offset | default / notes |
|---|---|---|
| (first muzzle of the weapon) | +0x139 | bool. Accessory coefficients (§2.4) apply only to this muzzle. |
| (config class) | +0x158 | its name is the "muzzle" string in events |
| `reloadAction` | +0x168 | used when the magazine has none |
| `magazineReloadTime` | +0x170 | s, required |
| `magazineReloadSwitchPhase` | +0x174 | |
| `drySound`, `zeroingSound`, `changeFiremodeSound`, `reloadSound`, `reloadMagazineSound`, `soundBullet` | +0x178, +0x1a0, +0x1c8, +0x1f0, +0x218, +0x240 | |
| `aiDispersionCoefX`, `aiDispersionCoefY` | +0x260, +0x264 | §2.2 |
| `useExternalOptic` | +0x268 | |
| `maxZeroing`, `minZeroing` | +0x26c, +0x270 | optional |
| `fireSpreadAngle` | +0x274 | |
| `soundContinuous` | +0x278 | |
| `enableAttack`, `optics`, `showEmpty` | +0x279, +0x27a, +0x27b | |
| `autoReload` | +0x27c | §3.4 |
| `moveToInternal`, `keepInInventory` | +0x27d, +0x27e | optional |
| `backgroundReload` | +0x27f | |
| `forceOptics` | +0x280 | |
| `showAimCursorInternal` | +0x281 | |
| `showSwitchAction` | +0x282 | forced to 1 when `primary == 0` |
| `canLock`, `lockAcquire`, `ballisticsComputer`, `FCSMaxLeadSpeed`, `FCSZeroingDelay` | +0x284 … +0x294 | |
| `primary` | +0x298 | |
| `useAsBinocular` | +0x29c | |
| `modes[]` | +0x5b8 | array of `WeaponModeType` |
| `magazines[]` + `magazineWell[]` | +0x5d0 (count +0x5d8) | see below |
| default magazine | +0x5e8 | the first entry of the merged list |
| `irDistance`, `irDotIntensity`, `irDotSize` | +0x5fc, +0x600, +0x604 | |

**The magazine list.** `magazines[]` is read first, in config order. Names that are not in
`CfgMagazines` are skipped. Then each `magazineWell[]` entry is resolved to its `MagazineWell`
object, and that well's magazines are appended **when they are not already in the list**. The
list therefore keeps order: the muzzle's own magazines come first, then each well's magazines
in well order. The reload choice in §3.4 uses this order.

**Memory points** (`0x1410fee20`, high). The point names are config strings and the points are
looked up in the weapon model's **memory LOD**. All results are in **weapon-model space**.

| entry | stored | notes |
|---|---|---|
| `muzzlePos` (usually `"usti hlavne"`) | +0x538 | point |
| — | +0x544 | `muzzleDir = normalize(muzzlePos − muzzleEnd)`. If its length² is < 0.01, it becomes (0,0,1) and the engine logs "Bad muzzle direction". |
| `muzzleEnd` (usually `"konec hlavne"`) | +0x550 | point |
| `shotPos` / `shotEnd` (optional) | +0x55c / +0x574 | Default to `muzzlePos`/`muzzleEnd`. When both are given and found: +0x55c = shotPos, +0x574 = shotEnd. |
| — | +0x568 | `shotDir = normalize(shotPos − shotEnd)`; falls back to `muzzleDir` |
| `irLaserPos`, `irLaserEnd` | +0x580 pos, +0x58c dir | IR laser |
| `cartridgePos` | +0x598 pos; point index +0x5b0 | |
| `cartridgeVel` | +0x5a4 = `(cartridgeVel − cartridgePos)·50` m/s; point index +0x5b4 | |

`selectionFireAnim` is read by the weapon/vehicle model code (`0x140fb9a20`, `0x141103cc0`, …),
not by the muzzle loader. It was not traced.

### 1.3 `MagazineType` (loader `0x1410f5b60`, high)

| entry | offset | default | notes |
|---|---|---|---|
| `ammo` | +0x28 | | `AmmoType`. Errors: "No class %s", "No ammo class %s". |
| (config class) | +0x30 | | its name is the "magazine" string in events |
| `scope` | +0x38 | | |
| `type` | +0x78 | | |
| `count` | +0x7c | required | clamped to ≥ 0 |
| `maxLeadSpeed` | +0x80 | required | |
| `initSpeed` | +0x84 | required | m/s |
| `initSpeedY`, `initSpeedZ` | +0x88, +0x8c | 0 | |
| `quickReload` | +0x90 | 0 | §3.3 |
| `modelSpecialIsProxy`, `deleteIfEmpty` | +0x91, +0x92 | | `deleteIfEmpty` is a tri-state: −1 = absent |
| `value`, `weight`/`mass` | +0xa0, +0xa4 | | |
| `maxThrowHoldTime`, `minThrowIntensityCoef`, `maxThrowIntensityCoef` | +0xa8, +0xac, +0xb0 | | throwing |
| `reloadAction` | +0xb8 | | |
| `tracersEvery` | +0x380 | **0** | §2.5 |
| `lastRoundsTracer` | +0x384 | **0** | §2.5 |
| `muzzleImpulseFactor` | +0x390 (angular), +0x394 (linear) | see note | §2.6 |

**`muzzleImpulseFactor` default.** It is (1, 1) when the ammo `simulation` is shotShell,
shotGrenade, shotSubmunitions, shotDeploy or shotIlluminating, and (0, 0) otherwise. A scalar
value `v` gives `(v, 1)`. A 2-element array `[a, b]` gives `(a, b)`.

`CfgMagazines` has **no `magazineReloadTime`**. The string is read only by the muzzle loader.

### 1.4 Accessory coefficients (`ItemInfo`, loader `0x14168b230`, high)

The muzzle attachment of the first muzzle supplies three coefficient blocks. When there is no
accessory, every coefficient is 1.0:

- **`MagazineCoef`**: `initSpeed` (+0xc8 of the item info).
- **`AmmoCoef`** (`0x14168b890`; default block `0x1420c9070`): [1] `visibleFire`,
  [2] `visibleFireTime`, [3] `audibleFireTime`, [5] `typicalSpeed`, [6] `airFriction`,
  [7] `audibleFire`. Slots [0] and [4] were not identified.
- **`MuzzleCoef`** (`0x14168bcb0`; default block `0x1420c9090`): [0] `dispersionCoef`,
  [1] `artilleryDispersionCoef`, [2] `fireLightCoef`, [3] `recoilCoef`, [4] `recoilProneCoef`,
  [5..10] the min/mid/max range and probability coefficients.

The getters are `0x140faf6c0` (AmmoCoef), `0x140fb2160` (MuzzleCoef) and the `MagazineCoef`
read inside `0x140faf810`.

## 2. The fire path

### 2.1 Call chain (high)

```
EntityAI vfunc +0x1780  FireWeapon(ctx, slot, target, ...)   Man: 0x1407862e0
  ├ checks (§3.1); if not ready → dry sound (0x140fd44f0), return false
  ├ vfunc +0x17b8  create the shot by ammo simulation          Man: 0x14078ba60
  │                                                            vehicles: 0x140d21630 (air),
  │                                                            0x140da56b0 (heli),
  │                                                            0x140ee0f60 (Car/Tank/Ship…),
  │                                                            0x140e16670 (Motorcycle)
  │     sim 6,7 (bullet, spread)            → FireBullet   0x140fa62b0
  │     sim 0,1,2,3,8 (+9,10,0x13 for Man)  → FireShell    0x140fa7140
  │     sim 4 (missile/rocket)              → FireMissile  0x140fa6990
  │     sim 0x14 (shotLaser)                → 0x140fa6090
  │     0x11 laserDesignate, 0x12 shotCM: special paths
  └ on success: PostFire 0x140fd5600 (ammo, burst, round reload, Fired EH, auto reload)
```

`ctx` is a turret context: `[0]` turret, `[1]` turret type, `[2]` gunner, `[3]` `WeaponsState`.

**Thrown items (Man).** In `FireWeapon` for ammo of the shell family, the magazine
`initSpeed` decides the path. With `initSpeed ≥ 30` the shell is fired at once, like a bullet.
With `initSpeed < 30` (thrown grenades) the engine plays the throw action (action 0xb) instead,
and the shot is created later from the animation. A thrown shell's speed is multiplied by the
throw-hold intensity `WeaponsState+0x88`, which is then reset to 1.

_Implementation note (`a3-world`)._ The shell family is
`AmmoType::is_shell_family` (sim 0, 1, 2, 3, 8, 9, 10, 0x13), the threshold is
`THROW_SPEED_LIMIT = 30`, and the intensity is `FireRequest::throw_intensity` — there is no throw
animation yet, so the caller supplies the release point and direction and the intensity. The
shipped data agrees: `GrenadeHand`'s `HandGrenade` magazine throws at 18 m/s with a 5 s fuse, and
the `Throw` weapon class carries the magazine.

### 2.2 Position, direction, dispersion (high unless noted)

**Muzzle position (Man, `0x14072dc30`).**
- The weapon proxy of the current weapon is looked up in the man's memory LOD. Its transform
  includes skeleton animation (`0x14122ef80`).
- Start point = proxy transform × the muzzle's shot point (model space of the man):
  - **bullets (sim 6, 7): `shotEnd`/`muzzleEnd`** (+0x574). This is the barrel's rear point,
    usually `konec hlavne`, not its tip (medium on the reason);
  - every other ammo: `shotPos`/`muzzlePos` (+0x55c).
- The point is then transformed to world space by the soldier's frame. When the soldier is
  inside a vehicle, the vehicle's proxy transform is used instead (vehicle `vfunc +0x9d8`).

**Fire direction (Man, `0x14072e3e0`, called with `useBarrel = 1`).**
- The direction is the muzzle's `shotDir` (+0x568), rotated by the same animated proxy
  transform.
- Zeroing is not applied here: the soldier aims through the optic, so zeroing already tilts
  the barrel.
- For a locally controlled player, `0x140707a80` adjusts the world direction before firing.
  It is not decoded (free aim or crosshair correction, low).
- Vehicles take position and direction from their turret's gun memory points
  (`0x140f32150`/`0x140f73920`/`0x140f73730`). Not decoded.

**Per-shot seed.** Both FireBullet and FireShell seed a private RNG:

```
h    = FNV-1a-64 over bytes: i32 LE roundsInMagazine   // count BEFORE this shot is removed
                             then f32 LE dir.x, dir.y, dir.z   // world fire direction
seed = low 32 bits of h
```

The RNG is the ANSI C LCG:
```
s = (s·1103515245 + 12345) & 0x7fffffff
U() = s · 2^-31                 // uniform [0,1)
```
The engine's global RNG `0x142165668` uses the same LCG. Its helpers are `U()` (`0x14030e310`)
and `U(mid, spread) = mid − spread + 2·spread·U()` (`0x14030e240`).

**Dispersion (`0x140fb05a0`).**
```
sx = sy = 0
repeat 4 times: sx += U(); sy += U()            // x and y draws interleave
D  = mode.dispersion · MuzzleCoef.dispersionCoef // radians
dx = (sx·0.5 − 1)·D                             // Irwin-Hall(4): range [−D, D), σ ≈ 0.2887·D
dy = (sy·0.5 − 1)·D
if gunner is AI (not player-controlled) and the muzzle is known:
    s  = brain ? (1 − skill(aimingAccuracy))·2.3 : 1       // skill index 1, medium on the name
    dx *= 1 + s·muzzle.aiDispersionCoefX
    dy *= 1 + s·muzzle.aiDispersionCoefY
local = (dx, dy, 1)
```
The distribution is approximately Gaussian, but **bounded** at ±D on each axis. Each axis is
independent, so the dispersion pattern is a square, not a disc.

The shot is first oriented along the fire direction, with `(0,1,0)` as the up reference.
`local` is expressed in that frame: x is the side axis, y is the up axis, z is the fire
direction. The rotated vector becomes the new shot direction:
```
d' = aside·dx + up·dy + dir·1
```

**Initial velocity.**
```
bullet (FireBullet):  v0 = initSpeed · d'          + v_shooter   // d' not renormalised:
                                                                 // |v0| is larger by sqrt(1+dx²+dy²)
shell  (FireShell):   v0 = initSpeed · normalize(d') + v_shooter
```
- `v_shooter` is the shooter entity's world velocity (visual state +0x54).
- **Artillery** (an `AmmoType` byte at +0x3c7 is set; probably `artilleryLock`, low):
  ```
  v = artilleryCharge · initSpeed · normalize(d')
  v.x += (Ux − 0.5)·A
  v.z += (Uz − 0.5)·A
  A   = mode.artilleryDispersion · MuzzleCoef.artilleryDispersionCoef
  Ux, Uz from Rand_MinMidMax(seed RNG, 0, 0.5, 1)
  ```
  The noise is added along **world** x and z.

**Shot setup.**
- The shot is created by the ammo factory `0x140e5b720` (sim-ballistics §2). Its creation
  record holds: `AmmoType`, the shooter, the gunner, the weapon, the muzzle, the target, the
  tracer flag (§2.5) and an `AmmoType` table value selected by `MuzzleState+0x40`.
- The shot is placed at the start point, given `v0`, and added to the world's fast-vehicle
  list (`World_AddFastVehicle`).
- **There is no partial sub-step at creation.** The shot's first move happens in its own
  simulate.
- FireBullet also writes two coefficients into the shot with `vfunc +0xcd0`: index 6 =
  `AmmoCoef.airFriction` and index 5 = `AmmoCoef.typicalSpeed`. Suppressors use them.
- Physics shells (PhysX shots, `vfunc +0x1d0`) also get a random angular velocity of
  `U()·10` rad/s on each axis.

### 2.3 Initial speed (`0x140faf810`, high)

```
mag   = magazine.initSpeed
W     = weapon.initSpeed                        // default −1
k     = accessory MagazineCoef.initSpeed        // 1 without accessory
if ammo.simulation ∉ {shotBullet, shotSpread}:  initSpeed = mag      // weapon value ignored
elif W > 0:   initSpeed = W · k
elif W < 0:   initSpeed = |W| · mag · k        // negative = multiplier of the magazine speed
else (W = 0): initSpeed = mag                  // k not applied
```

### 2.4 Rounds consumed and `multiplier` (`0x140fe0ac0`, high)

`RemoveAmmoAfterShot` takes away `n = min(mode.multiplier, rounds)` rounds.
- `multiplier` **only consumes ammo**. FireBullet creates exactly one shot per trigger event.
  The extra pellets of shotgun-like ammo come from `shotSpread`/submunitions.
- The count is stored obfuscated in the magazine as two 64-bit halves at +0x8c and +0x94:
  `rounds = ((a + b) ^ 0xd0867141) as i32`. A Rust port can store a plain `i32`.
- On a soldier, when a magazine reaches 0 it is removed from the inventory in two cases:
  `deleteIfEmpty == 1`, or `count == 1` with `deleteIfEmpty` absent. This is `0x140748690` /
  `0x140e2f660`.

### 2.5 Tracers (high)

At shot creation, with `r` = rounds in the magazine **before** this shot:
```
tracer = r ≤ lastRoundsTracer  or  (tracersEvery ≥ 1 and (r − lastRoundsTracer) % tracersEvery == 0)
```
With both values at 0 (the defaults), a shot is never a tracer, because `r` is always ≥ 1.

### 2.6 Vehicle recoil impulse (`0x140f47fe0`, high)

Vehicle fire paths (for example `0x140d21630`) apply an impulse after each shot:
```
J   = −(ammo.hit · initSpeed / 52) · d          // d = fire direction, world
r   = firePos − shape.centerOfMass (shape+0x28c), rotated into model space
linear  impulse = J · muzzleImpulseFactor[1]   (+0x394)
angular impulse = (r × J) · muzzleImpulseFactor[0]   (+0x390)
```
Nothing is applied when both factors are 0, as for bullets by default. Soldier recoil (the
`recoil` classes and `recoilCoef`) is an animation and camera effect. It was not traced here.

## 3. Rate of fire and reload state

### 3.1 State per magazine and per slot (high)

| field | meaning |
|---|---|
| `Magazine+0x70` | **round reload phase**: 1 just after a shot, falls to 0 |
| `Magazine+0x74` | round reload duration factor |
| `Magazine+0x78` | **magazine reload, seconds remaining** |
| `Magazine+0x7c` | magazine reload, total seconds |
| `Magazine+0x88` | pylon index; −1 for normal magazines (medium) |
| `MuzzleState+0x44` | rounds left in the current burst |
| `MuzzleState+0x40` | per-slot ammo table index, reset on load from `AmmoType+0xe28` (meaning low) |
| `WeaponsState+0x2c` | selected slot |
| `WeaponsState+0x30` | pending fire request slot (`fire` command), −1 = none |
| `WeaponsState+0x8c` | time of the last shot |

**Ready to fire** (`0x140fb5be0`):
```
magazine present
and round phase ≤ 0
and magazine reload ≤ 0
and the unit is not busy (vfunc +0xfc8(2): an action such as a weapon switch)
and rounds > 0
```
Weapons without a magazine, or with ammo `simulation` 0x15 (none), are always ready.

**FireWeapon (Man) when not ready.**
- If there is no magazine, or the magazine is empty, reloading, or the unit is busy, it plays
  `drySound` and returns false.
- It never queues the shot.
- It does not start a reload by itself; auto-reload happens after the last round (§3.4).

### 3.2 After a successful shot (PostFire `0x140fd5600`, high)

```
lastShotTime = now
if burstLeft ≤ 0:                                    // start a new trigger pull
    n = mode.burst
    if mode.burstRangeMax ≥ 0:
        n = clamp(burst + floor(U()·(burstRangeMax − burst)), burst, burstRangeMax − 1)
    if mode.multiplier > 0 and magazine is not on a pylon:
        n = min(n, rounds / multiplier)
    burstLeft = n
factor = 1
if not mode.autoFire and (burstLeft == 1 or mode.burst == 0):   // last round of this pull
    k = 5.0                                   // no AI brain
    k = 1.0                                   // player-controlled (medium)
    k = 2 − skill(reloadSpeed)                // AI, skill index 7 (medium on the name)
    factor = U(1.0 ± 0.1) · k
magazine.roundPhase  = 1.0
magazine.roundFactor = factor
burstLeft -= 1
RemoveAmmoAfterShot (§2.4); network broadcast; Fired EH (§4)
```
`burstRangeMax` is therefore an **exclusive** upper bound.

### 3.3 Per-step update (weapons simulate `0x140f90e40`, high)

Each step `dt`, every distinct magazine that is loaded, has rounds > 0 and is not on a pylon
is updated:

```
if magReload > 0:
    if |magReload − magReloadTotal| ≤ 1e-4: play muzzle.reloadMagazineSound   // first step
    magReload = max(magReload − dt, 0)
elif roundPhase > 0:
    if roundPhase == 1.0: play muzzle.reloadSound                              // e.g. bolt cycling
    roundPhase = mode.reloadTime > 0
                 ? max(roundPhase − dt / (mode.reloadTime · roundFactor), 0)
                 : 0
```

So the time between shots is `reloadTime · roundFactor`:
- **inside a burst and in `autoFire` modes it is exactly `reloadTime`;**
- after the last round of a semi-auto pull it is randomised by ±10 % and, for AI, scaled by
  the reload skill.
- The magazine reload and the round reload never count down together.

**Bursts and requests.** In the same simulate:
- A selected slot (or the secondary slot `+0x34`) with `burstLeft > 0` fires again through
  `FireWeapon`, aimed at the last fire target (`entity+0x958`). Burst rounds 2…n are fired by
  the engine, one each time the round reload completes.
- A pending `fire` request (`WeaponsState+0x30`) fires only when it names the **selected**
  slot, the weapon is ready, and either the aim check returns ≥ 0.7 (`vfunc +0x16c0`) or
  `vfunc +0x1688` allows firing.
  - A successful fire clears the request. A failed fire keeps it.
  - A mode with `useAction = 1` runs its UI action instead of firing.

**`autoFire` (held trigger).** The fire input path was not traced. Holding the trigger calls
`FireWeapon` again; the readiness gate above limits it to one round per `reloadTime` (medium).

### 3.4 Magazine reload

**Auto reload** happens after the shot that empties the magazine (`0x140f95940`):
1. Set the round phase to 0.
2. Reload when the unit's auto-reload flag (`entity+0xaa0`) is set and either
   `muzzle.autoReload == 1` or the unit is not player-controlled. In practice AI always
   reloads, and a player reloads only with `autoReload = 1` (medium on the flag).
3. The reload calls `vfunc +0x18b0` (`0x140fdfeb0`).

**Choosing the magazine** (`0x140fa51b0`, high). Only magazines that are not loaded in another
weapon are candidates, and only magazines with rounds > 0:
1. A magazine of the **same `MagazineType`** as the empty one: the one with the **most
   rounds**. On a tie, the first one found wins.
2. Otherwise, walk the muzzle's merged magazine list (§1.2) in order, skipping the old type.
   For the first type that has any candidate, take the candidate with the most rounds.
3. Otherwise no reload happens.

**Soldier reload start** (`0x140748120`):
- Plays the magazine's `reloadAction` (or the muzzle's) as a weapon action, then attaches the
  magazine.
- In vehicles and for `autoReload` muzzles the magazine is attached directly.
- Raises EH 0x50 (80) and the muzzle config event, both with the args of `0x140fd9970`.

**Attaching a magazine** (`0x140fe0010`, high):
```
reject magazines the muzzle cannot use ("Cannot use magazine %s in muzzle %s")
raise EH 0 ("Reloaded" type, args built by 0x140fd9970: unit, weapon, muzzle, new mag, old mag)
k = player 1.0 | AI (2 − skill(reloadSpeed)) | no brain 1.8
magReload = magReloadTotal = instant ? 0 : muzzle.magazineReloadTime · U(1.0 ± 0.2) · k
roundPhase = 0
roundFactor = magazine.quickReload ? 1 : U(1.0 ± 0.1) · k
every slot whose muzzle is this muzzle now points at the new magazine
```
- An empty old magazine that is not shared is removed from the soldier's inventory.
- `magazineReloadTime` is the muzzle's value. The magazine has none.

**`weaponState`** (`0x1405342b0`) returns
`[weapon, muzzle, mode, magazine, rounds, roundPhase, magPhase]`:
- `roundPhase` is −1 when there is no magazine.
- `magPhase` is computed by `0x1410f51e0`:
  `x = min(magReload/magReloadTotal + MuzzleState+0x58, 1)`, and `magPhase = x > 0 ? 1 − x : 0`.
  The meaning of `MuzzleState+0x58` is unclear (low).
- `weaponReloadingTime` (`0x140fb5fd0`) returns the raw round phase (`Magazine+0x70`).

## 4. Events

Event IDs, from the name registration at `0x1400a2220`: `Deleted` 2, `Hit` 3, `FiredMan` 14,
`Fired` 15, `HandleDamage` 30, `HitPart` 31.

### 4.1 `Fired` / `FiredMan` (`0x140fd5df0`, high)

PostFire builds the record
`{target, weapon, muzzle, mode, magazine, shot, person, gunner}` and calls the dispatcher.
Normal magazines take the path through `vfunc +0x17c0`, which also plays the fire sound,
muzzle flash and cartridge. Pylon magazines call the dispatcher directly.

**Strings and objects.** The weapon, muzzle, mode and magazine strings are their config class
names. The ammo string is the `CfgAmmo` class name.

**Fired.** Raised when the entity has a `Fired` (15) handler, **or** the muzzle class has
`EventHandlers`, **or** the ammo class has `EventHandlers`. Arguments:
```
[unit, weapon, muzzle, mode, ammo, magazine, projectile, gunner]
```
- `unit` is the firing entity: the vehicle, or the soldier.
- `mode` is `""` when there is no mode.
- `magazine` is `""` when there is no magazine.
- `gunner` is the turret context's gunner.

**FiredMan.** Raised when the *person* who fired has a `FiredMan` (14) handler. Arguments:
```
[unit, weapon, muzzle, mode, ammo, magazine, projectile, vehicle]
```
`vehicle` is the firing entity when it is a vehicle type, otherwise `objNull`.

**Delivery.** Both events are queued as **postponed events** (`IPostponedEvent`, world queue
`0x14070b440`) and run later in the frame. When the script runs, the round has already been
removed and the projectile exists. The muzzle and ammo config handlers use the same argument
array (`MuzzleEventVoidCall` / `AmmoEventVoidCall`).

**Side effects of the dispatcher.**
- Stores the shooter's audibility and visibility of fire:
  - `entity+0x948 = ammo.audibleFire (+0x4c8) · AmmoCoef.audibleFire`
  - `+0x94c = ammo.visibleFire (+0x4cc) · AmmoCoef.visibleFire`
  - `+0x950 = ammo.visibleFireTime (+0x4d4) · AmmoCoef.visibleFireTime`
- Notifies AI within 50 m.
- For remote units the engine raises `Fired` only within `max(visibleFire, audibleFire)` of
  the camera (the wiki says so; the code computes this value in `FireWeapon`, medium).

### 4.2 `HitPart` (direct `0x140e5ce10`; explosion variant `0x140e5d600`; high on sources)

Raised on the **target** when the target has a `HitPart` (31) handler. It is dispatched
**immediately**, not postponed (`0x140fcf800`). The argument is an array of hit records, one per
hit component:
```
[target, shooter, projectile, position, velocity, selection, ammo, vector, radius, surfaceType, isDirect, instigator]
```
- `shooter` is shot+0x5b8 and `instigator` is shot+0x5c8.
- `position` is the hit point.
- `velocity` is the shot's velocity at the hit (visual state +0x54).
- `selection` lists the fire-geometry selection names of the hit component (`0x1415bb880`).
- `vector` is the surface normal.
- `radius` and `surfaceType` come from the component; `radius` defaults to 1.0.
- `isDirect` is true in `0x140e5ce10`; the explosion path builds the indirect hits.
- The `ammo` sub-array was not decoded.

The projectile's own `HitPart` handler (projectile EH index 7) gets a 10-element variant.

### 4.3 `Hit` (id 3)

The wiki signature is `[unit, source, damage, instigator]`. Its builder was not located: it is
not one of the direct callers of the handler-exists check `0x140fbc5b0`.

## 5. Answers to open points of `sim-ballistics.md` (high)

- **`R` in the explosion falloff (§6, `shape+0x7c`).** It is half the diagonal of the
  **Geometry LOD's** bounding box: `0.5·|bboxMax − bboxMin|`, LOD index `shape+0x30a`. When the
  model has no Geometry LOD, it is `shape+0x78` (the whole-model bounding sphere). The box
  centre is stored at `shape+0x280`. Source: `0x1415bcb70`.
- **Special LOD index bytes**, from `0x1415d0580`, matched by resolution:

  | resolution | offset | LOD |
  |---|---|---|
  | 1e13 | +0x30a | Geometry |
  | 2e13 | +0x30b | |
  | 3e13 / 4e13 | +0x30c | PhysX |
  | 1e15 | +0x309 | Memory |
  | 2e15 | +0x315 | Land contact |
  | 3e15 | +0x316 | Roadway |
  | 4e15 | +0x317 | Paths |
  | 5e15 | +0x318 | Hit-points |
  | 6e15 | +0x30e | View geometry |
  | 7e15 | +0x30d | Fire geometry |
  | 1.3e16 … 2e16 | +0x30f … +0x314, +0x31d | |
  | 10000–11000 | +0x319 / +0x31a | shadow volume (first index / count) |
  | 11000–12000 | +0x31b / +0x31c | |

- **`surfDeflect` for ricochet:** not resolved in this pass.

## 6. SQF commands (handlers from `docs/re/sqf-commands.tsv`)

| command | handler | behaviour |
|---|---|---|
| `unit fire muzzle` / `[muzzle, mode, magazine]` | `0x1405281a0` / `0x140528330` | Runs `FireTurretWeapon` on every turret. It finds the slot whose muzzle name matches (and the mode, if given), loads the named magazine first if given, then sets `WeaponsState+0x30` to that slot. This is a **request**: it fires in a later simulate only when that slot is the **selected** weapon and is ready and aimed (§3.3). Returns nothing. No request is made when the slot's magazine is empty, unless its type has `count < 1`. (high) |
| `setAmmo [muzzle, n]` | `0x140553250` → `0x1411176b0` | `n` is rounded. Every slot whose **muzzle** name matches and that has a magazine gets the value. **`n < 0` or `n > count` sets the magazine to full `count`.** Works for soldiers and, through the turret, for a soldier's vehicle. (high) |
| `currentMagazine unit` | `0x14084c990` | The magazine class name of the selected slot, `""` when there is none. (high) |
| `magazines unit` | `0x14083dfa0` → `0x14082cec0` | Iterates the unit's magazine storage. **Empty magazines are skipped** (except pylon ones). Loaded magazines are added only by the variants called with the "include loaded" flag (`magazinesAmmoFull`-style: primary, handgun, secondary, binocular slots). Order follows the containers (medium). |
| `addWeapon w` / `[w, autoSelect]` | `0x14083bc10` → `0x140831dd0` | `autoSelect` defaults to true. It is passed to the add routine `0x140f93210` as −1 (select) or −3. Whether a magazine from the inventory is loaded automatically was not traced; the wiki says a magazine must be added first. |
| `weaponReloadingTime` | `0x1404b8270` | the raw round phase, −1 when not applicable |
| `weaponState` | `0x1405342b0` | see §3.4 |

Not decompiled: `ammo`, `selectWeapon`, `reload`, `forceWeaponFire`, `removeWeapon`,
`addMagazine(s)`, `primaryWeapon`, `currentWeapon`.

## 7. Open points

- The player trigger and `autoFire` input path. Which input calls `FireWeapon` each frame was
  not traced.
- Vehicle gun position and direction (`0x140f32150`, `0x140f73920/0x140f73730`, turret
  `memoryPointGun`), and the player-only direction correction `0x140707a80`.
- Skill index names. 1 → `aimingAccuracy` and 7 → `reloadSpeed` are inferred from the name
  table at `0x141c64600` and from how the values are used (medium).
- `Magazine+0x88`. Pylon index is the most likely meaning: normal magazines take the
  full-effects path and the round-reload countdown; pylon ones skip both.
- `MuzzleState+0x40` and `AmmoType+0xe28/+0xe10` (a per-shot value passed to the shot), shot
  `+0x758` (set from `muzzle+0x608`), and `AmmoType+0x3c7` (artillery flag).
- The `ammo` element of `HitPart`, and where `Hit` (3) is raised.
- `magPhase` in `weaponState`: the role of `MuzzleState+0x58`.
- Soldier recoil (`recoil`/`recoilProne` classes, `recoilCoef`), weapon sway, and
  `fireSpreadAngle`.
- The ricochet surface coefficient (sim-ballistics §4.1).
