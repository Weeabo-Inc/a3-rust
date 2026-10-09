# Aircraft: the basic helicopter model and the `airplanex` aero model

How Arma 3 2.22 flies helicopters (`helicopterrtd`, the engine's own **basic** flight model, the
default when the advanced flight model is off) and planes (`airplanex`). Source:
`arma3_x64.exe`, read with the Ghidra tooling in `docs/re/TOOLING.md`; RVAs below are virtual
addresses. Background: `sim-vehicles.md` §1, §3, §4; `world-object-model.md`.

**Confidence.** (high) = transcribed from the decompiled code with its constants. (medium) = the
formula is certain, the meaning of an input is inferred. (low) = a guess.

**Frames.** All formulas below are written in the engine's model space: X right, Y up, Z forward.
A raw P3D faces the other way (front at −Z, left at +X, `p3d-odol.md`): the model space here is
the raw one turned half a turn about Y, `(x, y, z) → (−x, y, −z)`. Evidence: the formulas put the
tail rotor at `−0.95·R` on Z and thrust along +Z, and the Hummingbird's `inspect_hitvrotor*`
memory points are at raw `z ≈ +4`; flown with raw coordinates, every plane's nose wheel trails
the main gear and no jet rotates before 450 km/h, flown turned they lift off at their
`landingSpeed` (§6) (high).
"Model speed" `s = (sx, sy, sz)` is the world velocity rotated into model space (the frame's
`+0x60`; `sz` is forward speed). Torques are about the centre of mass; positive about X is nose
down, positive about Z is roll left (right wing up), positive about Y is yaw right. Formulas are
transcribed component by component; the usual `cross(a, b)` formula gives the same signs.

## 1. Classes and step (high)

| class | `Simulate` (vtable slot 374) | model |
|---|---|---|
| `HelicopterRTD` | `0x140dc5b30` | RotorLib when the advanced model is on (`+0x1630` RotorLib object non-null and the difficulty/option bits at `GWorld+0x2e70` allow it); otherwise calls `HelicopterAutoEPE::Simulate` |
| `HelicopterAutoEPE` | `0x140db4a30` | **the basic model**, forces handed to a PhysX rigid body |
| `HelicopterAuto` / `Helicopter` | `0x140daa1f0` / `0x140da87f0` | legacy non-PhysX versions (no shipped class) |
| `AirplaneAutoEPE` | `0x140690e40` | the plane model, forces handed to a PhysX rigid body plus PhysX vehicle wheels |
| `AirplaneAuto` / `Airplane` | `0x140d2a190` / `0x140d25570` | legacy |

Step length: `Transport::Simulate` (`0x140f5e720`) sets the simulation precision of a PhysX
vehicle (`vtable+0x1d0`) to **1/15 s** when local and 0.1 s when remote (high).

Every aircraft force step has the same shape:

1. read the PhysX body: world velocity → frame `+0x54`, model speed → `+0x60`, acceleration
   `(v − v_prev)/dt` → `+0x6c`; angular velocity → entity `+0x2c0`; angular momentum
   `L = R·I·Rᵀ·ω` → entity `+0x2cc` (`I` = the shape's inertia tensor, shape `+0x2bc`);
2. control smoothing, rotor or engine spool (§2.3, §3.3);
3. accumulate, in model space, a **force** `F`, a **torque** `T`, a **friction force** `Ff` and
   an **angular friction** `Tf`; transform all four to world space;
4. integrate them with the friction integrator (§4) and hand the velocity change to PhysX.

Mass is `vtable+0x618` (the shape's mass, shape `+0x5c8`, unless `setMass` overrides it).
`R` below is the shape's **bounding-sphere radius**, shape `+0x78` (`sizeOf` returns `2·R`,
`0x140564680`) (high). Gravity is PhysX's; plane lift uses `9.8066`.

## 2. Helicopter, basic model

### 2.1 `HelicopterType` fields (loader `0x140da39c0`, high)

| config | offset | default | notes |
|---|---|---|---|
| `gearMinAlt` | +0x2be4 | 0.5 | |
| `gearDownTime`, `gearUpTime` | +0x2bdc, +0x2be0 | | stored as `1/t` (0 when `t ≤ 0`) |
| `mainRotorSpeed`, `backRotorSpeed` | +0x2c08, +0x2c0c | | animation only |
| `startDuration` | +0x2c10 | 20 | stored as `1/t` |
| `mainBladeRadius`, `tailBladeRadius` | +0x2c14, +0x2c18 | 0 | effects |
| `tailBladeVertical` | +0x2c1c | 1 | stored inverted |
| `liftForceCoef` | +0x2c20 | 1 | |
| `cyclicAsideForceCoef` | +0x2c24 | 1 | |
| `cyclicForwardForceCoef` | +0x2c28 | 1 | |
| `backRotorForceCoef` | +0x2c2c | 1 | |
| `bodyFrictionCoef` | +0x2c30 | 1 | |
| `altFullForce`, `altNoForce` | +0x2be8, +0x2bec | 1000, 3000 | metres ASL |
| `min/max/neutralMainRotorDive` | +0x2c34, +0x2c3c, +0x2c44 | | degrees in config, stored in radians |
| `min/max/neutralBackRotorDive` | +0x2c38, +0x2c40, +0x2c48 | | radians |
| `envelope[]` | +0x2c50 (count +0x2c58) | | |
| `washDownStrength`, `washDownDiameter` | +0x2c84, +0x2c88 | –, 40 | effects |
| hit point indices `HitHRotor`, `HitVRotor` | +0x2bf0, +0x2bf4 | | `HitEngine` is `+0x2120` (shared with every transport) |

### 2.2 State

Frame (`entity+0xd0`): `+0x1f0` cyclic forward, `+0x1f4` cyclic aside, `+0x1f8` collective
lever (animation), `+0x1fc` main rotor angle, `+0x200` main rotor angle step, `+0x204` tail rotor
angle, `+0x208` main rotor dive, `+0x20c` gear (1 = retracted), `+0x210` **rotor speed** 0..1,
`+0x214` **collective** (thrust) −0.2..rotor speed.

Entity: `+0x1348` engine throttle wanted (0 or 1: `EngineOff` `0x140d9df80`, `EngineOn`
`0x140d9dfb0`), `+0x134c` pedal (rudder) state, `+0x1350` rudder wanted, `+0x1354` collective
wanted, `+0x1358` cyclic forward wanted, `+0x135c` cyclic aside wanted, `+0x1360` rotor dive
wanted, `+0x14d4` wanted climb rate (keyboard collective), `+0x5e4` bit 0 = destroyed,
`+0x452` bit 1 = touching land, bit 2 = touching water.

### 2.3 Player input (`0x140da2c50`, called from `0x140db4620`) (high)

`a(UA)` is the analogue value of a user action (`0x141093600`). Action ids (static table
`0x1400b3dc0`): `HeliUp 221, HeliDown 222, HeliLeft 223, HeliRight 224, AirBankLeft 225,
AirBankRight 226, HeliRudderLeft 227, HeliRudderRight 228, HeliForward 229, HeliBack 230,
HeliFastForward 231, HeliThrottlePos 236, HeliThrottleNeg 237, AirPlaneBrake 239,
HeliCyclicForward 240, HeliCyclicBack 241, HeliCyclicLeft 242, HeliCyclicRight 243,
HeliCollectiveRaise 244, HeliCollectiveLower 245, HeliCollectiveRaiseCont 246,
HeliCollectiveLowerCont 247`.

```text
digital = a(CollectiveRaise) − a(CollectiveLower)
analog  = a(CollectiveRaiseCont) − a(CollectiveLowerCont)
```

Whichever of the two changed by more than 0.1 most recently is in charge (`+0x150a` = digital).
Either one above 0 starts the engine (`vtable+0x1168`) when it is off.

- **analog**: collective wanted `+0x1354 = analog`.
- **digital**: wanted climb rate `+0x14d4 = 10·digital` m/s, and in `0x140db5810`:
  `wanted = (climb − (0.5·acc.y + vel.y))·0.1 + collective`, times an up-side-down factor
  `f(up.y)` (1 for `up.y > 0`, `10·(up.y + 0.2) − 1` for −0.2..0, −1 below), clamped to
  −1..1. World `acc`, `vel`. **Releasing the keys holds the vertical speed at 0.**

```text
side   = a(HeliLeft) − a(HeliRight)
k      = max(|min(aside.y·6, 1)|·[aside.y·6·side·3 ≤ 0], clamp((|sz| − 4)·0.04, 0, 1))
rudder wanted   +0x1350 = clamp((1 − k)·3·side + a(RudderLeft) − a(RudderRight), −1, 1)
cyclic aside    +0x135c = 3·(a(CyclicLeft) − a(CyclicRight)) + k·3·side
cyclic forward  +0x1358 = 2·mouseY + 3·(a(CyclicForward) − a(CyclicBack))
```

(`aside` is the model X axis in world space; `mouseY` is `0x141093b80(input, 1)`, the mouse
axis.) So `A`/`D` yaw at low speed and bank above 4 m/s (high).

Overrides in `0x140db5810` (high): with no live pilot the wanted controls rest at
(rudder −0.1, collective 0.1, cyclic forward 0.1, cyclic aside −0.1) in the air and at 0 on
land; while the angular velocity exceeds 10 rad/s (`|ω|² ≥ 100`) they are forced to collective
0.3 and everything else 0. With "mouse controls" off
(`+0x150e = 0`) the cyclic is direct as above; the mouse-flight branch (`+0x150b`) is an
autopilot on bank/dive and is not decoded here.

### 2.4 Control smoothing and rotor (`0x140db7990`, `0x140db75c0`) (high)

Rates per second, `dt` = step:

```text
cyclicAside   (+0x1f4) → clamp(+0x135c, −2, 2) at ±4/s, clamp ±1
cyclicForward (+0x1f0) → +0x1358 at ±10/s, clamp ±1
rotorDive     (+0x208) → clamp(+0x1360, ±1) at ±0.15/s, clamp ±1
pedal         (+0x134c) → +0x1350 at ±10/s, clamp ±1
```

Rotor speed (`+0x210`):

```text
throttle = min(+0x1348, 1 − max(dmg(HitHRotor), dmg(HitEngine)))
if destroyed: rotor −= 0.2·dt
else:
  d = throttle − rotor
  if dmg(HitHRotor) > 0.95 (and the main-rotor-broken flag): throttle = rotor = 0
  autorot = clamp(−0.125·sy − 0.25, 0, 1)            // descent spins the rotor
  if autorot > 0 and not on land and throttle ≤ 0.1:
      d = (autorot − rotor)·0.04·dt
  c = max(collective·0.2, 0)
  d = clamp(d, (−0.025 − 0.37·c)·dt, dt/startDuration)
  rotor += d
rotor = clamp(rotor, 0, 1)
collective (+0x214) → min(+0x1354, rotor) at ±0.25/s, then clamp(−0.2, rotor)
```

The engine spins the rotor up in `startDuration` seconds (20 s default) and down at about
0.025/s; autorotation keeps it turning in a descent.

### 2.5 Forces and torques (`0x140db4a30` and helpers) (high)

`m` mass, `ρ` = rotor speed, `c` = collective, `R` bounding radius, `y` = model origin height
ASL, `h` = height above the surface below (`0x141656280`).

**Altitude factor** `alt(y) = 1` below `altFullForce`, linear to 0 at `altNoForce`.

**Main rotor lift** (`0x140db29c0`, `0x140da6b00`). With `ρ' = ρ·(destroyed ? 0.1 : 1)`, lift is
0 when `ρ' ≤ 0.01`. Otherwise:

```text
ge   = clamp(1.2 − h/(1.5·R), 0, 1)²·0.25                  // ground effect
a    = max(sy + 3 − ρ'·c·18·(ge + 1), −5)
e    = envelope sampled at |s_xz|, spanning 0..1.4·maxSpeed:
       i = (n−1)·(1/1.4)·|s_xz|/maxSpeed (m/s); lerp; beyond the table the
       second-to-last entry (the original indexes `n−2` there)
L    = max(e·4000 − (|a|·a·400 + a·6000), −5000)·alt(y)
F   += (0, L, 0)·liftForceCoef·ρ²·M/3000
```

`M` is the mass of the helicopter plus every sling-loaded object (`0x140db3f20`). When
`|rotorDive| > 0.001` the lift vector is rotated about X by `rotorDive` radians:
`(0, L·cos, L·sin)`. At hover (`sy = 0`, `e(0) = 0`, no ground effect) lift equals weight when
`400·b² + 6000·b = 9.8066·3000/liftForceCoef` with `b = 18c − 3`: the Hummingbird
(`liftForceCoef = 1.5`) hovers at `c ≈ 0.32`.

**Cyclic** (the lift acts off the centre of mass):

```text
Lc = alt(y)·m·ρ²·2.11
p  = (0.6·R·cyclicAsideForceCoef·cyclicAside, 0, −1.6·R·cyclicForwardForceCoef·cyclicForward)
T += cross(p, (0, Lc, 0))  =  (1.6·R·cyclicForwardForceCoef·cyclicForward·Lc, 0, 0.6·R·cyclicAsideForceCoef·cyclicAside·Lc)
```

**No rotor**: when not touching land (`+0x452` bit 1 clear) and `ρ < 0.1`:
`T += (−1.3·m, 0, 0.5·m)` — a helicopter falling without its rotor pitches up and rolls.

**Tail rotor** (`0x140db3c50`), with `d = min(1, 1.3·dmg(HitVRotor))⁴`,
`b = aside.y/|dir_xz|` (bank), `v = |sz|`:

```text
yaw = ((1−d)·backRotorForceCoef·pedal·8 − max(c + 0.1, 0.05)·ρ·d·20)·(1 − min(1, v·0.0125))
    + b·(b²·(b·(b·v·0.00147 + b·v²·2.52e−5) + v·0.0032667 + v²·5.6e−5) + v²·0.000336 + v·0.0196)
Fy = yaw·ρ²·m·R·0.0791667
T += cross((0, 0.0076·R, −0.95·R), (Fy, 0, 0)) = (0, −0.95·R·Fy, −0.0076·R·Fy)
```

A destroyed tail rotor (`d → 1`) leaves the main rotor's reaction (`−20·(c+0.1)·ρ`) unopposed.

**Weathervane and pitch damping** (in `Simulate`), `k = min(1, (1/30)/dt)`, `v = |sz|`:

```text
Fx = m·k·(−sx·(sz²·4.8e−6 + v·2.8e−4) − |sx|·sx·(sz²·4.8e−6 + v·2.8e−4))
Fy = m·k·(−sy·(sz²·2.4e−6 + v·1.4e−4) − |sy|·sy·(v·8.4e−6 + sz²·1.44e−7))
T += (0.95·R·Fy, −0.95·R·Fx, 0)          // a side force at the tail, (0, 0, −0.95·R)
```

**Body friction** (`0x140db2c40`, `0x140d9aa60`). `u = s − Rᵀ·(wind + gust)`, gust a random
vector in −1..1 per axis renewed every 2 s per helicopter; `q = ρ²·0.6 + 0.4`; `sgn(x)` is −1, 0
or 1:

```text
fx = (ux·|ux|·3 + ux·50 + 2·sgn(ux))·q
no rotor dive (maxMainRotorDive ≤ 0.01):
  fy = (uy·|uy|·3 + uy·500 + 2·sgn(uy))·q
  fz =  uz·|uz|·2.05 + uz·50 + 2·sgn(uz)
rotor dive:
  fy = (uy·|uy|·4 + uy·500 + 15·sgn(uy))·q
  fz =  uz³·0.05 + (|uz| + 100)·uz + 6·sgn(uz)
Ff = bodyFrictionCoef·(fx, fy, fz)       // newtons, not scaled by mass
```

**Angular friction**: `Tf = L·(|sz|·0.014 + sz²·0.00024 + 2.5)·(ρ + 0.2)` (world space).

### 2.6 Ground contact

`HelicopterAutoEPE` calls the legacy contact routine `0x140daba00` with its "apply forces" flag
off: PhysX resolves the contacts. The legacy routine's own model (used by `HelicopterAuto`, and by
us in place of PhysX) for each Geometry-LOD point below the surface (`0x14121f920`, up to `n`
contacts with `n = min(count, 5)`), depth `d`, surface normal `N = normalize(−∂h/∂x, 1, −∂h/∂z)`,
contact point `r` relative to the centre of mass (high):

```text
land:  d' = min(d, 0.1)          water: d' = min(d, 3)·0.001
F  += N·m·40·(3/n)·d'                 T += cross(r, that)
v  = velocity (see below)
f  = (v.x·5000 + 10000·sgn(v.x),
      v.y·|v.y|·1000 + v.y·8000 + 10000·sgn(v.y),
      v.z·|v.z|·150 + v.z·250 + 5000·sgn(v.z))
f  = m·(3/n)·0.0001·R·f                 // R: model → world
Ff += f
T  −= cross(r, f'),  f' = f with each axis zeroed where |v| < 1 on that axis
Tf += L·15·d                            // d unclamped
```

The legacy code feeds the **world** velocity into the friction formula and then treats the
result as model-space (an inconsistency of the legacy path); the EPE helicopter never runs it.

### 2.7 Animation state (high)

```text
+0x200 = dt·ρ·20                         // main rotor angle step, rad
+0x204 += (1 − d)·ρ·dt·20                // tail rotor angle
+0x1fc = wrap(+0x1fc + +0x200, 60π)      // main rotor angle
+0x1f8 → (collective wanted + 1)/2 at 0.5·Δ + 0.2 per second   (lever, digital collective)
```

Model sources the shipped models use: `rotorh`, `rotorv` (rotation 0..2π over value 0..1),
`cyclicforward`, `cyclicaside`, `collectivertd`, `rudderrtd`, `rpm`, `horizonbank`,
`horizondive`, `altbaro`, `vertspeed`, `speed`, `direction`. The engine resolves source names
without storing them as plain strings (none of `rotorh`/`rotorv` appear in the image); the value
of `rotorh` is assumed to be the main rotor angle in revolutions (`+0x1fc/2π`) and `rotorv` the
tail angle (low: inferred from the `0..2π` ranges in the models).

## 3. Plane, `airplanex`

### 3.1 `AirplaneType` fields (loader `0x140d1df50`, high)

| config | offset | default | notes |
|---|---|---|---|
| `altFullForce`, `altNoForce` | +0x27b4, +0x27b8 | 5000, 13000 | |
| `stallWarningTreshold` | +0x28c8 | | |
| `envelope[]` | +0x2730 | | §3.4 |
| `thrustCoef[]` | +0x2748 | | |
| `elevatorCoef[]`, `aileronCoef[]`, `rudderCoef[]` | +0x2760, +0x2778, +0x2790 | | |
| `elevator/aileron/rudderControlsSensitivityCoef` | +0x27a8, +0x27ac, +0x27b0 | | rates of the control surfaces, 1/s |
| `aileronSensitivity`, `elevatorSensitivity`, `wheelSteeringSensitivity` | +0x2830, +0x2834, +0x2838 | | |
| `landingSpeed` | +0x2824 | | km/h → m/s; if < 1: `max(0.33·maxSpeed, 33.33)` m/s |
| `stallSpeedForced` | +0x282c | | if missing or < 0: `landingSpeed·0.87` (≤ 21 m/s) else `·0.65` |
| (derived) | +0x2828 | | `min(landingSpeed, 65)` m/s |
| `flapsFrictionCoef`, `gearsUpFrictionCoef`, `airBrakeFrictionCoef` | +0x2840, +0x2844, +0x2848 | –, 0.5, 3 | |
| `airFrictionCoefs0/1/2[]` | +0x284c, +0x2858, +0x2864 | (0,0,0), (0.1, 0.05, 0.006), (0.001, 0.0005, 6e−5) | per axis X, Y, Z |
| `flaps`, `airBrake` | +0x2819, +0x281a | | bools |
| `landingAoa` | +0x283c | | |
| `rudderInfluence` | +0x2874 | 0.99619 (cos 5°) | |
| `VTOLYaw/Pitch/RollInfluence` | +0x2878.. | 2 | |
| `angleOfIndicence` | +0x2884 | 0.05236 (3°) | radians |
| `draconicForceX/Y/ZCoef` | +0x2888, +0x288c, +0x2890 | 7.5, 1, 1 | |
| `draconicTorqueXCoef[]`, `draconicTorqueYCoef[]` | +0x2898, +0x28b0 | 0.15 | number or array |
| `throttleToThrustLogFactor` | +0x29f8 | 1 | |
| `gearRetracting` | +0x2808 | | gear rates `+0x281c` (down), `+0x2820` (up) |
| `VTOL` | +0x2814 | | 0 = conventional |
| hit points: engine `+0x2120`, `+0x28e8`; ailerons `+0x28e0/+0x28e4`; elevators `+0x28d0/+0x28d4`; rudders `+0x28d8/+0x28dc` | | | indices |

**Coefficient arrays** (`0x140d197c0`): sampled by forward speed `sz` over **0..1.5·maxSpeed**:
`i = (n−1)·sz/(1.5·maxSpeed)`, linear between entries, the last entry beyond, the first below
0 (0 when `maxSpeed ≤ 0`); an empty array returns a default (1, or for `thrustCoef` a curve: 1
below `0.66·vref`, linear to 0 at `1.15·vref`, `vref = maxSpeed`).

### 3.2 Player input (`0x140d1c5f0`) (high)

```text
rudder wanted   +0x13bc = a(HeliRudderLeft) − a(HeliRudderRight) + (a(HeliLeft) − a(HeliRight))
aileron wanted  +0x13c0 = 0.5·clamp(a(AirBankLeft) − a(AirBankRight), ±1) + 0.25·(a(HeliLeft) − a(HeliRight))
elevator wanted +0x13b4 = 4.5·clamp(a(HeliFastForward) + a(HeliForward) − a(HeliBack), ±1)
digital = clamp(a(HeliUp) − a(HeliDown), ±1)
analog  = a(HeliThrottlePos), brakeAxis = a(HeliThrottleNeg)
```

Throttle wanted `+0x13ac`: the analog axis when it changed last (`clamp 0..1`); otherwise, while
`|digital| > 0.0001`:
`t = clamp((t^f + clamp(digital, ±dt/2))^(1/f), 0, 1)`, `f = throttleToThrustLogFactor` — the
keys move the lever 0.5 per second. Brake `+0x1380 = +0x1384 = clamp(a(AirPlaneBrake) +
brakeAxis, 0, 1)` (wheel brake and air brake). Any thrust input starts the engine.

### 3.3 Control smoothing (in `Simulate`) (high)

```text
engine (+0x214)    → (engine on ? 1 : 0) at ±0.1/s
thrust (+0x13a8)   → throttle wanted at +0.4/s, −0.7/s, clamp 0..1
rpm    (+0x13b0)   = engine·10·(thrust + 0.4)
rudder (+0x1f8)    → +0x13bc at ±rudderControlsSensitivityCoef/s, clamp ±1
aileron (+0x204)   → +0x13c0 at ±aileronControlsSensitivityCoef/s, clamp ±1
elevator (+0x1f4)  → +0x13b4 at ±elevatorControlsSensitivityCoef/s, clamp ±1
flaps (+0x1e8)     → 0.5·flapsPosition (0, 1, 2) at ±0.33/s
gear  (+0x1ec, 1 = up) → (gear down ? 0 : 1) at gearDown/UpTime rates (only with gearRetracting)
airbrake (+0x208)  → +0x1384 (0 if destroyed) at ±1/s
lever  (+0x200)    = throttle wanted ^ throttleToThrustLogFactor
```

### 3.4 Forces and torques (`0x140690e40` and helpers) (high)

`vmax` = `maxSpeed` in m/s, `vl` = landing speed (m/s), `sz` forward speed, `alt` the altitude
factor, `ctrl = clamp(|sz|/(0.65·vl) − 0.5, 0, 1)·alt` (control authority), `R` bounding radius,
`scale` the object scale (1).

**Angle of attack** (`0x14068f510`): `aoa = angleOfIndicence − sy/|s|` (0 if `|s|² ≤ 1e−6`).

**Thrust**: `Fz += m·sqrt(vmax)·engine·min(thrust, engineHealth)·0.30174154·alt·thrustCoef(sz)`,
`engineHealth = 1 − mean damage of the engine hit points` (`0x140d19b70`).

**Lift** (`0x140d2da40`), `x = 0.8·sz/vmax` (the envelope spans 0..1.25·maxSpeed):

```text
flapAoa = x < 0.22 ? 4° : x > 0.4 ? 0 : 4° − (x − 0.22)·0.38785
α = aoa + 2° + (flapAoa + 1°)·flaps
if α > 18°: α = max(36° − α, 0)                  // stall: lift falls off past 18°
i = (n−1)·x
lift = i < 0 ? 0 : i ≥ n−1 ? envelope[n−1] : lerp(envelope, i)·max(α, −10°)/18°
if lift ≥ 0: lift ·= 1 + ge,  ge = clamp(1.5 − h/(1.5·R), 0, 1)²·0.1
Fy += m·9.8066·lift·alt        (0 when sz ≤ 0)
```

So `envelope` is lift in g at 18° effective angle of attack.

**Ground steering**: on land, `|sz| < 25`, upright: `k = sz < 6.67 ? sz·0.14993 : (25 − sz)·0.054555`
(≤ 40), `Ty += k·wheelSteeringSensitivity·(−30)·m·rudder`.

**Ailerons** (`0x14068dbb0`): each half `aL, aR = 0.5·aileron` (hit-point jitter when damaged
> 0.25, `0x140d19110`); `A = R·scale·ctrl·m·aileronSensitivity·aileronCoef(sz)·6.537733`;
`Tz += A·(aL + aR)`. Opposite halves (damage) add `Fy += sgn(aL)·1.96132·min(|aL|,|aR|)·m·lift`.

**Elevator**: halves `eL, eR = 0.5·(elevator + trim)`;
`E = m·ctrl·elevatorCoef(sz)·5.88396·elevatorSensitivity·5`; `Tx += E·(eL + eR)`,
`Tz += m·(eR − eL)·9.8066`.

**Rudder** (`0x14068f260`), only when `|s| > 1`: halves `rL, rR = 0.5·rudder`;
sideslip factor `g = 1`, or when `sx·rudder > 0`:
`g = clamp((|sz|/sqrt(sz² + sx²) − rudderInfluence)/(1 − rudderInfluence), 0, 1)`;
`D = m·ctrl·g·rudderCoef(sz)`; `Ty += −0.76·R·scale·D·(rL + rR)`; `Tz += 0.25·D·(rL + rR)`.

**Draconic (weathervane) forces and torques** (`0x14068de10`), only when `|s| > 1`, with
`σ` = stall factor (below), `w = |s|`, `ŝ = s/w`:

```text
fx = (w²·0.005 + w·0.02)·|ŝx|,   fy = (w²·0.02 + w·0.2)·|ŝy|
Fx −= alt·m·draconicForceXCoef·fx·ŝx
Fy −= alt·m·draconicForceYCoef·fy·ŝy
Fz += (−0.05·fy − 0.15·fx)·m·alt·draconicForceZCoef·σ·ŝz
Ty += (0.3·σ + 0.04)·sx·4·draconicTorqueXCoef(sz)·m
Tx += m·draconicTorqueYCoef(sz)·(0.04 + 0.3·σ)·sy·(−4)
```

**Stall** (`0x140d164d0`), airborne only: with `vs` = stall speed,
`σ = clamp((|aoa| − 10°)/15°, 0, 1)·min(1, max(0, 1.5 − sz/vs) + 0.3)` (0 on land or if
`vs ≤ 0.1`). Below `0.65·vl` in the air (or destroyed, with `σ = 1`): `Tx += 2.5·m·σ`,
`Tz += 1.5·m·σ` — the nose and a wing drop.

**Auto trim**: when `|s| > 1`, the pilot is a player and `|elevator| < 0.001`:
`trim −= Tx·dt/m·0.1`; then `trim = clamp(trim, 0, 0.1·clamp((sz − vl)/(0.5·vl), 0, 1))`.

**Air friction** (`0x14068e220`): per axis `f = c2·|s|·s + c1·s + c0·sgn(s)` with
`airFrictionCoefs2/1/0`; `fx, fy ·= 0.1`; `fz ·= 1 + |1 − gear|·gearsUpFrictionCoef
+ flaps·flapsFrictionCoef + brake·airBrakeFrictionCoef`, where `brake` is the airbrake state with
`airBrake = 1`, else `(1 − engine·thrust)·min(2·sz/vmax, 2)` (an automatic speed brake at low
throttle); `Ff = m·alt·f`.

**Angular friction**: `Tf = 2·L`.

### 3.5 Ground contact

The plane rolls on **PhysX vehicle wheels** (`class Wheels`, the car's wheel and suspension
entries, `sim-vehicles.md` §2). `0x14068e440` steers the steering wheels by
`rudder·(−54°)/(0.4·|v| + 1)`. On top, the legacy contact loop (`0x14121f920` points, `n =
max(count, min(k, 3))` contacts, depth `d' = min(d, 0.1)`, `w = (9/n)·d'`, model-space tangential
velocity `t`):

```text
normal  F += N·m·(90/n)·d'   (alive only)
friction (·m·(3/n)·0.0001):
  x: ±100000·w + roughness·tx·10000·w
  y: (300·ty ± 500)·w + roughness·ty·1000·w
  z: (300·tz ± 500)·w + roughness·tz·1000·w + brakeTerm
  each clipped to |f| ≤ 10·|t| with the sign of t (0x140690640)
  plus ±1.5·w per axis (stiction), Tf += (0.3/n)·L
brakeTerm: belly (destroyed / not upright): ±40000 (|tz| ≤ 2) or ±80000 ·w
           gear up (> 0.5):                ±20000·w
           gear down: ±20000·w·brake·b,  b = 1 below max(0.1·vmax, 22.2), 0 above max(0.2·vmax, 36.1)
```

### 3.6 Integration of both (`0x14068e440`, same as the helicopter's `0x140db2f30`) (high)

See §4.

## 4. The friction integrator (`0x140e845f0`) (high)

In model space, per axis, with `a = F/m`, `μ = Ff/m` (both rotated into model space):

```text
v += a·dt
if v·μ > 0:  v = |v| ≤ |μ·dt| ? 0 : v − μ·dt
```

Friction never reverses a velocity. The same is applied to the world angular momentum with
`T` and `Tf`. Then `ω = R·I⁻¹·Rᵀ·L`; external impulses (`+0x420..`, explosions) are added; the
engine hands PhysX the force `m·(v' − v)/dt` and the torque `I·(ω' − ω)/dt` (`addForce`,
`addTorque`, mode `eFORCE`) and restores `v`, so PhysX applies the change with gravity and
contacts. _Assumption (medium): the PhysX scene step covers the entity's step, so the force
acts for `dt`._

`a3-flight` integrates the body itself (no PhysX): semi-implicit, gravity added **with** the
forces before the friction step (as the legacy non-PhysX vehicles did), so that the legacy
contact model of §2.6 settles at rest. In flight the two orders differ only when a friction
stops a velocity within one step.

## 5. Open points

- PhysX contact material for aircraft hulls (helicopter skids, belly landings): we use the legacy
  contact model of §2.6 / §3.5 instead.
- The PhysX vehicle drive of plane wheels (`vtable+0x160` with ±1 when the thrust pushes along the
  motion on land) is not decoded; wheels free-roll and brake.
- The mouse-flight autopilot branches (`+0x150b`) are not transcribed, and neither is the
  engine's AI pilot (`0x140db5810` main body: `flyInHeight`, landing). `sim/air.rs` flies a
  stand-in altitude hold for it: with a commanded height (`flyInHeight` / `flyInHeightASL`,
  `sqf-object-state.md`) and nobody at the controls, a helicopter asks for the climb rate
  `0.2 m/s per metre of height error` (saturating at the collective's `±10 m/s`) with the cyclic
  centred, and a plane for the elevator that closes the same error through the vertical speed
  (saturating at its full `±4.5`). The gain and the saturation points are ours; the engine's AI
  pilot was not traced.
- VTOL vectoring (`VTOL > 0`, `+0x1fc`): thrust split by `sincos`, VTOL control torques
  `2.5·m·influence·control·σv` — read but not implemented.
- Animation source name → value mapping for `rotorh`, `rotorv` (§2.7).
- PhysX's own damping on aircraft bodies.
- The rotor dive wanted (`+0x1360`) is only set by the autopilot branch; the keyboard player
  leaves it at 0, so the rotor-dive helicopters (Blackfoot, Kajman) fly with the lift vertical in
  the body and top out at 71 % of `maxSpeed` in our scripted flights (§6). Whether the game's
  keyboard pilot does better needs the oracle.

## 6. Measured (`crates/a3-flight/tests/real_flight.rs`, `A3_ROOT`, 1/15 s steps)

Scripted flights of the shipped classes against their own config numbers; flat ground, still
air. Helicopters: the nose held at a fixed pitch with the collective keys released (vertical
speed hold), 1° steeper each run until the hold runs out of collective. Planes: a take-off run
at full throttle with the stick pulled from 90 % of `landingSpeed`, and four minutes of level
flight at full throttle.

| class | measured | config |
|---|---|---|
| `B_Heli_Light_01_F` hover, keys released | ±0.00 m over 60 s, collective 0.320 | §2.5 predicts 0.3196 |
| `B_Heli_Light_01_F` rotor start | 20.1 s | `startDuration` 20 |
| `B_Heli_Light_01_F` top level speed | 232 km/h (95 %) | `maxSpeed` 245 |
| `O_Heli_Light_02` / `I_Heli_light_03` | 255 / 255 km/h (88 / 87 %) | 290 / 293 |
| `B_Heli_Transport_01` / `I_Heli_Transport_02` / `B_Heli_Transport_03` | 251 / 264 / 266 km/h (84–89 %) | 300 |
| `O_Heli_Transport_04_F` | 248 km/h (99 %) | 250 |
| `B_Heli_Attack_01` / `O_Heli_Attack_02` (rotor dive) | 276 / 260 km/h (72 / 71 %) | 385 / 365 |
| `C_Plane_Civil_01_F` lift-off / level | 130 km/h (100 %) / 428 km/h (95 %) | 130 / 450 |
| `B_Plane_CAS_01` lift-off / level | 243 (93 %) / 735 km/h (105 %) | 260 / 700 |
| `O_Plane_CAS_02` | 243 (97 %) / 837 (93 %) | 250 / 900 |
| `I_Plane_Fighter_03` | 204 (95 %) / 809 (101 %) | 215 / 800 |
| `B_Plane_Fighter_01_F` | 254 (98 %) / 1028 (86 %) | 260 / 1200 |
| `O_Plane_Fighter_02_F` | 267 (103 %) / 1446 (96 %) | 260 / 1500 |
| `I_Plane_Fighter_04_F` | 255 (98 %) / 925 (93 %) | 260 / 1000 |
| `B_UAV_02` | 133 (95 %) / 425 (106 %) | 140 / 400 |

The config's `maxSpeed` and `landingSpeed` are the designers' targets, not measurements of the
original; parity with the game itself is for the server oracle to establish.
