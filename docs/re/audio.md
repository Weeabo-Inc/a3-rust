# Audio: simple expressions, sound sets, attenuation, environment

Reverse-engineering notes for the sound system of `arma3_x64.exe` 2.22.0.154103, implemented in
`crates/a3-audio` (`expr`, `config`, `player`, `spatial`, `filter`, environment). All addresses
are VAs in the Ghidra project. Confidence: **high** (read from decompiled code, unambiguous),
**medium** (read from code, one step of interpretation), **low** (inferred), **unknown**.

The engine mixes with XAudio2 and positions voices with X3DAudio. `a3-audio` has its own
software mixer (cpal output) and reproduces the parts that shape what is heard: expression
values, distance curves, the distance low-pass (as an XAudio2-style state-variable filter),
doppler limits, sample choice and randomizers. Panning is our own equal-power stereo law with
the X3DAudio inner radius as a near-field spread; it is not X3DAudio's matrix.

## Config survey (merged vanilla config)

3,535 CfgSoundShaders, 2,845 CfgSoundSets, 17 CfgSoundCurves, 8 CfgDistanceFilters, 58
CfgSound3DProcessors, 2,949 CfgSounds, 350 CfgMusic, 38 CfgSFX; CfgEnvSounds lists 17
`soundSetEnvironment` sets. Every shader/set expression parses, given two lenient rules:
a few expressions end with a stray `)` (accepted), and `envelope` is used by the railgun
shaders. 94 set references name shaders, 3D processors or distance filters that do not exist
(footsteps, tyres, some vehicles); such shaders are skipped. 9,283 shader sample paths resolve
in the VFS (paths omit the extension: `.wss`, then `.ogg`, then `.wav` is tried); 193 distinct
paths are missing.

Expression variables used by shaders (count of shaders): speed 1529, interior 485, rain 250,
forest 217, houses 179, sea 160, meadows 140, trees 114, altitudeground 68, daytime 38, windy
41, ... plus vehicle controllers (rpm, thrust, angvelocity, latslip, damper0, surface types).

Sections 1 (expression language) and 4 (environment coefficients and the sound map) come
first.

---

## 1. Simple expression language

### 1.1 Architecture (high)

The "simple expression" language is the **SQF parser and evaluator** running on a separate
`GameState`, `DAT_1421695a8`, created at `0x1403829c0` (called from `0x1407a0490`). Nothing is
tokenized by a custom lexer. Consequences:

- Tokens, numeric literals, parentheses, identifier rules and operator priorities are those
  of SQF (see `docs/re/sqf-semantics.md` and `docs/re/sqf-command-table.md`).
- Each new `GameState` gets the 155 **"Default"** category commands (`0x1402dbb20`, called
  from the `GameState` constructor `0x1402c8ef0`): scalar `+ - * / % mod ^ atan2 min max`,
  unary `- + abs sqrt sin cos tan asin acos atan exp ln log floor ceil round rad deg`,
  `pi`, comparisons, `and/or/not`, `if/then/else` and so on. These only accept SCALAR/BOOL, so
  in an expression they can only fold **constant** sub-expressions.
- On top of that, `0x1403829c0` registers 46 overloads on the new type
  `EXPRESSION` (`DAT_1421695b0`; category "Simple expression"). An `EXPRESSION` is a compiled
  RPN program (`GameDataExpression`, vtable `0x141aac5d8`), not a number.

The 46 overloads (tsv: `docs/re/sqf-commands.tsv`, category `Simple expression`):

| Op | Forms | Priority | Leaf (evaluator step) | Semantics on the float stack |
|---|---|---|---|---|
| `*` | E*E, E*S, S*E | 7 | `0x1403811a0` | a*b |
| `/` | E/E, E/S, S/E | 7 | `0x140380cb0` | a/b (no zero check: IEEE inf/NaN) |
| `+` | binary 3 forms | 6 | `0x140380a60` | a+b |
| `-` | binary 3 forms | 6 | `0x1403814f0` | a-b |
| `min` | 3 forms | 6 | `0x140381100` | `b <= a ? b : a` |
| `max` | 3 forms | 6 | `0x140381060` | `a <= b ? b : a` |
| `pow` | 3 forms | 9 | `0x140381250` | `powf(a, b)` |
| `>=` `>` `<=` `<` | 3 forms each | 3 | `0x140380ba0` `0x140380af0` `0x140381400` `0x140381350` | 1.0 if true else 0.0 |
| `factor` | E factor ARRAY[2] | 4 | `0x140380e50` | see 1.3 |
| `interpolate` | E interpolate ARRAY[4] | 4 | `0x140380f50` | see 1.3 |
| `envelope` | E envelope ARRAY[4] | 4 | `0x140380d40` | see 1.3 |
| unary `+` | E | - | `0x1403822c0` (returns its argument) | identity |
| unary `-` | E | - | `0x140381230` | -a (sign flip) |
| `abs` | E, S | - | `0x140380a40` | \|a\| |
| `sqr` | E, S | - | `0x1403814b0` | a*a |
| `sqrt` | E, S | - | `0x1403814d0` | sqrtss (NaN for a<0) |
| `randomGen` | E, S | - | `0x140381300` | a * rand01() (new value at every evaluation) |

Not present for EXPRESSION: `==`, `!=`, `?`, `%`, `mod`, `^` (only `pow`), `sin`, `cos`,
`exp`, `log`, `ln`, `floor`, `round`, `and`, `or`, `not`, `if`. `^` and `sin` and so on work
only on constants. A config expression that uses them on a variable does not compile.
(high)

Priorities are the SQF ones: higher binds tighter. `pow` 9 > `* /` 7 > `+ - min max` 6 >
`factor interpolate envelope` 4 > comparisons 3. Unary operators bind tighter than any
binary operator. So `windy factor [0.1,0.5]` must be parenthesised when combined with
`*`: `1.2 * windy factor [0.1,0.5]` parses as `(1.2*windy) factor [...]`. Same-priority
binary operators associate left to right, as in SQF. (high for priorities, medium for
associativity: taken from the SQF parser, not re-checked here)

The comparison overloads are binary EXPRESSION commands, so `(windy > 0.01)` gives 1.0/0.0.
`houses max interior` is `max(houses, interior)`.

Comparison NaN behaviour (exact): `a >= b` is `!(a < b)` (NaN → 1), `a > b` is `!(a <= b)`
(NaN → 1), `a <= b` is `!(b < a)` (NaN → 1), `a < b` is `!(a >= b)` (NaN → 1). With
non-NaN inputs they are the ordinary comparisons. (high)

### 1.2 Compile and evaluate (high)

Compile: `0x14037fec0(out_code, text, names[], count)` and the variant `0x140380460`.

1. For each `i` in `0..count` it sets the SQF variable `names[i]` in the expression state
   to an EXPRESSION whose program is `[LoadVar(i)]` (`0x14037fcd0`, step `0x140381580`).
   Variables are SQF globals, so **identifier lookup is case-insensitive**
   (`altitudeGround` matches the engine name `altitudeground`).
2. It evaluates `text` with the SQF evaluator (`0x1402d31f0`).
3. Result type EXPRESSION: its program is copied; the highest variable index used is stored
   (`out[6] = maxIndex+1`).
   Result type SCALAR: the program becomes `[PushConst(value)]` (step `0x140380c50`).
   Any other result (nil from an unknown identifier, BOOL, a type error, a parse error):
   compile fails and returns false. The sound loader then treats the sound set as not
   playable (`0x140943450` returns 0 and the SoundObject is not created). An **unknown
   identifier does not become 0**: it fails the whole expression. (high)

Program = array of `{fnptr step, f32 arg}` (16 bytes each). The EXPRESSION builders
(`0x140381b10` binary E,E; `0x140381c70` E,S; `0x140384350` S,E; `0x140381dc0` E,ARRAY)
concatenate the operand programs, add `PushConst` for scalar operands, and append the
operator step. ARRAY operands of `factor/interpolate/envelope` must have exactly 2/4/4
elements and every element must be a SCALAR (constant). Otherwise a type error occurs and
compile fails. (high)

Evaluate: `0x1403815f0(code, out*, values*, valueCount)`. It returns false if
`valueCount < code.maxIndex+1`. It runs the steps on a float stack (in-place, a 32-element
inline buffer that grows). The result is the single remaining value. It returns true only
if exactly one value is left **and** it is finite (`_finite`). The value is written to `*out`
even when it is not finite. Callers in the sound code ignore the return value. (high)

Numbers are `f32` throughout evaluation. (high)

### 1.3 factor / interpolate / envelope (high)

`x factor [a, b]` maps a→0 and b→1, linear, clamped to [0,1]:

```
if b > a:  x < a → 0;  x > b → 1;  else (x - a) / (b - a)
if b <= a: x < b → 1;  x > a → 0;  else 1 - (x - b) / (a - b)      // = (a - x)/(a - b)
```
Edge case a == b: x < a → 1, x > a → 0, x == a → 1 - 0/0 = NaN.
Example: `altitudeGround factor [75, 0]` = 1 on the ground, 0 at 75 m and above.

`x interpolate [a, b, c, d]` maps a→c and b→d, clamped:

```
if b > a:  x < a → c;  x > b → d;  else c + (x - a)/(b - a) * (d - c)
if b <= a: x < b → d;  x > a → c;  else d + (x - b)/(a - b) * (c - d)
```

`x envelope [a, b, c, d]` is a trapezoid, 0 outside the open interval (a, d):

```
if !(a < x && x < d) → 0
rise = (x <= b) ? (x - a)/(b - a) : 1;   if rise <= 0 → 0
fall = (x >= c) ? 1 - (x - c)/(d - c) : 1
result = rise * fall
```

### 1.4 randomGen (high)

`randomGen m` pushes `m * r`. `r` comes from the engine-global LCG `0x14030e310` (state
`DAT_142165668`):
`state = (state * 1103515245 + 12345) & 0x7fffffff; r = state * 4.656613e-10` (r in [0,1)).
The same generator is used for sample choice and the randomizers (section 2).

---

## 4. Environment coefficients at the listener

### 4.1 Variable lists (high)

Two name lists are built at static-init time:

**List B: CfgEnvSounds / `soundSetEnvironment` (22 values).** Names array
`DAT_1421a0130`, initialised in `0x140049cf0`. Values are computed by `0x140928d60(pos, out[22])`.
`pos` is the listener (camera) position.

| # | Name | Value (pos = listener; y = height ASL) |
|---|---|---|
| 0 | `rain` | weather rain intensity (`weather+0x88`), 0 if the world has snow (`Landscape+0xea0`) |
| 1 | `night` | scene "night" factor (`*(world+0x1a0)+0xf4`). Exact source not traced (low) |
| 2 | `meadow` | sound map, layer meadow (bilinear) |
| 3 | `trees` | sound map, layer trees (bilinear) |
| 4 | `hills` | `clamp((terrainY(x,z) - minHillsALtitude) / (maxHillsALtitude - minHillsALtitude), 0, 1)`. CfgWorlds `minHillsALtitude`/`maxHillsALtitude` (`0x141155750`); defaults 0 / 500 if no world |
| 5 | `houses` | sound map, layer houses |
| 6 | `windy` | `clamp(|wind|/10, 0, 1) * clamp((y - surfaceY(x,z) + 0.1)/1.1, 0, 1) * Landscape+0xea4` (`0x141118c70`; `+0xea4` defaults to 1.0) |
| 7 | `deadbody` | nearest dead body within 10 m: `max(0, 1 - dist2D * 0.1)`, 0 if none (`0x141118fe0`) |
| 8 | `sea` | sound map, layer sea |
| 9 | `forest` | `max(0, T(p) + T(p±20 m diagonals, 4 samples) - 4)`: 5 samples of the trees layer (centre and (±20, ±20)); 1 only when all 5 are full trees |
| 10 | `waterdepth` | `max(0, waterSurfaceY(p) - terrainY(x,z))` (`0x140e6c9c0`) |
| 11 | `camdepth` | `max(0, waterSurfaceY(p) - y)` (`0x141118d90`) |
| 12 | `anomaly` | always 0 |
| 13 | `coast` | `sin(pi * clamp(c, 0, 1)) * clamp((y - surfaceY + 0.1)/1.1, 0, 1)`. `c` = water fraction from the geography grid, see 4.4 (`0x141118dd0`) |
| 14 | `altitudeground` | `max(0, y - terrainY(x,z))` (`0x141655d30` = terrain height, triangle interpolation) |
| 15 | `altitudesea` | `max(0, y)` |
| 16 | `daytime` | time of day as a fraction of a day, 0..1 (`DAT_14225db2c`; SQF `dayTime` = this * 24) |
| 17 | `shooting` | `DAT_1420a5e80`, a 0..1 ramp: accumulator / 30 s, see 4.5 (medium) |
| 18 | `fog` | `clamp(fogBase - (y - fogBaseAltitude) * fogDecay * 0.1, 0, 1)` (`0x141146c60`, world `+0x2dd8/+0x2de4/+0x2df0`; medium on field names) |
| 19 | `yearTime` | `DAT_14225db24` (fraction of year, medium) |
| 20 | `ambientTemp` | `world+0x1880` |
| 21 | `snow` | 1.0 if the world has snow (`Landscape+0xea0`), else 0 |

Config spellings such as `meadows`, `altitudeGround` and `waterDepth` resolve through
case-insensitive SQF lookup. Only `meadows` (plural) is **not** in list B. An env sound
expression that uses `meadows` fails to compile in this context (medium: check the shipped
CfgEnvSounds; they appear to use `meadow`).

**List A: CfgEnvSpatialSounds (17 values).** Names array `DAT_1421a0090`, initialised in
`0x1400497b0`. Used by `0x140927710` (positional ambient emitters):
`rain, night, wind, daytime, distance, shooting, meadows, trees, houses, forest, sea,
altitudeSea, altitudeGround, rainDrops, yearTime, ambientTemp, snow`.

A third list (`0x14004ac80`, `DAT_1421a0420..`) has `speed, distance, meadows, meadow,
forest, houses, trees, sea, interior` (vehicle/other contexts, not traced).

### 4.2 The sound map: byte layout (high)

The sound map is a `u8` grid on the Landscape (`Landscape+0x7f0`). It is the WRP
`QuadTree<u8>` that follows the geography tree. Each byte packs four 2-bit weights. The
weight is `field / 3` (0, 1/3, 2/3, 1):

| Bits | Layer (EnvType) | Variable |
|---|---|---|
| 0-1 | 0 | `sea` |
| 2-3 | 1 | `trees` |
| 4-5 | 2 | `meadow` |
| 6-7 | 3 | `houses` |

The "Bad EnvType" error string marks the switch in the sampler and the writer.

### 4.3 Sampling (high)

`0x14163bc20(Landscape, pos, envType)` interpolates bilinearly between cell centres:

```
fx = pos.x * invSoundCell - 0.5          // invSoundCell = Landscape+0x44
fz = pos.z * invSoundCell - 0.5
ix = round(fx - 0.5) (≈ floor(fx)),  iz likewise;  tx = fx - ix,  tz = fz - iz
v(i,j) = ((cell(i,j) >> (2*envType)) & 3) / 3
result = v00 (1-tx)(1-tz) + v10 tx (1-tz) + v01 (1-tx) tz + v11 tx tz
```

Cell fetch `0x14163dc80`: inside the grid it returns the byte. Outside the grid it clamps
the indices to `[0, Landscape+0x744]` and **masks the byte with 0x33**. So off-map only
`sea` and `meadow` survive and `trees`/`houses` read 0. Index `x` is along world X and `z`
is along world Z. This is the same orientation as the land grid in `wrp.md` (medium on the
north/south row order: it follows whatever order the WRP quad tree uses for the land grid).

Sound cell size = land cell size / soundMapSizeCoef. In the generator (4.4), every
object-grid cell (size `Landscape+0x738`) is split into `soundMapSizeCoef x soundMapSizeCoef`
sound cells of size `Landscape+0x40`, and the grid side is `Landscape+0x38`. So the sound
grid side is `land * coef` and the sound cell is `landCell / coef` (high, consistent with
`wrp.md`).

### 4.4 How the map is generated (`[InitSoundMap]`, `0x14104cbb0`) (high for the formulas, medium for when it runs)

The shipped WRP already contains the map. The engine regenerates it only in
`0x14104b810` (a land-initialisation path that also logs "using non-binarized object"; it
looks like the text-WRP/Buldozer path). A runtime reader should use the WRP data. The
generator explains what the bytes mean:

1. Clear the map. For each object-grid cell and each of its N×N sub cells
   (N = soundMapSizeCoef): query the objects whose bounding rectangle overlaps the sub cell,
   enlarged by `0.2 * soundCellSize`. Walk them in order and keep running sums by the
   model's map class (`shape+0x4f0`, the P3D `class` property):
   - trees layer: `treehard` +0.2, `treesoft` +0.05, `bushhard` +0.025, `bushsoft` +0.0125,
     `forest` +1.0
   - houses layer: `house` +0.4, `church` +0.75

   After each object, if a sum is > 0, splat it (`0x141049390`, radius 1) into the 3×3
   neighbourhood of the sub cell. Each target field becomes `clamp(round(3 * sum), 0, 3)`
   if its current weight is lower (the fields only ever increase; the distance weight
   `max(1, 0.25/r²)` is always 1 at radius 1).
2. Sea: for every land cell whose geography word has `maxWaterDepth != 0`
   (`geo & 0x60`), write sea weight 1.0 (field 3) into all its N×N sub cells (same splat).
3. Meadow, for every sound cell: with s, t, h = sea, trees, houses fields (0..3),
   `meadow = clamp(round(3 * (1 - s/3)(1 - t/3)(1 - h/3)), 0, 3)`.

### 4.5 Other inputs (medium)

- `coast` (`0x14163df20`): on the **geography** grid (`Landscape+0x7c0`, `+0x73c` = 1/land
  cell). Per land cell, `w = minWaterDepth + maxWaterDepth` (bits 0-1 plus bits 5-6, 0..6).
  Each of the 4 bilinear corners is the sum of a 2×2 block of `w` divided by 24. The
  corners are bilinearly interpolated to give `c` in [0,1]. Then
  `coast = sin(pi * clamp(c,0,1)) * aboveWater`, so coast is largest where half the
  neighbourhood is water.
- `aboveWater(p) = clamp((y - surfaceY(x,z) + 0.1) / 1.1, 0, 1)` (`0x14117fff0`).
  `surfaceY` (`0x141659910`) is the terrain or water surface. `waterSurfaceY`
  (`0x141659820`) is the water level.
- `shooting`: `0x14092a630` keeps `acc = clamp(acc + dt_signed, 0, 30)` and sets
  `shooting = acc / 30` (`DAT_1420a5e88` = 30.0). The sign/source of the per-frame increment
  is in the caller `0x1411572e0` (not traced). (low/medium)

### 4.6 How environment sounds play (medium)

Per frame `0x14092a630(env, dt, listenerPos)`:

1. Compute the 22 values (`0x140928d60`).
2. If the world defines `soundSetEnvironment[]` (`0x14093d030` false), `0x14092a7a0` is
   used. For each sound set it creates a SoundObject once, with the 22 names
   (`0x140932620(obj, names, 0, 22)`, flags 0x40, loop, category 5). Then every frame it
   pushes the new values (`0x140932a30`). This **re-evaluates** the shader `volume` and
   `frequency` expressions and the set-level volume expression with the current values, and
   multiplies every shader's volume by the set-level expression. The object is placed at
   the camera, with direction `normalize(cam.x, 0, cam.z)`. The sets are non-positional
   loops in practice (the configs use `spatial = 0`, `loop = 1`). There is no extra fade
   logic in this path, beyond voice-level volume smoothing (`0x140965160`).
3. Otherwise (legacy classes `{ sound[]; volume = "expr"; soundsRandom[] }`, `0x140929ff0`):
   `v = clamp(expr(values) * altFactor, 0, 1) * sound[].volume`, with
   `altFactor = clamp(1 - altitudeGround * 0.004, 0, 1)` (0 at 250 m AGL). If
   `v <= dB(-80)` the loop is stopped and released. Otherwise a non-positional looped voice
   is created (handle flags 0x40) and its volume is set to `v` every frame. Random sounds:
   the first timer is 10..20 s (`rand(10,20)*1000` ms). Then `soundsRandom[]` entries are
   picked by probability weight.
4. Spatial env sounds (CfgEnvSpatialSounds, list A) are updated by `0x140927710`.

---

## 2. Distance attenuation and sound set / shader parameters

### 2.1 Curves (`Sound::SoundCurve`, vtable `0x141b5ada0`) (high)

Loader `0x140930870(bank, cfgClass, key)`:
- If the entry is a string: look it up in `SoundCurveBank` (`DAT_1421a01e0`) by name. If it
  is not there, read `CfgSoundCurves >> name >> points` (`0x140931a30`) and cache it under
  that name.
- If the entry is an array: parse it inline.

Points parser `0x140931c40`: `points[] = {{x,y},...}`. At least 2 points are needed (else
the curve is null). **The x values are always renormalised** to [0,1]:
`x' = (x - minX)/(maxX - minX)`. An inline `rangeCurve[] = {{0,1},{150,0.3}}` therefore
behaves exactly like `{{0,1},{1,0.3}}`: the absolute metres are discarded, and the
distance scale comes from `range`. Points are expected in ascending x (binary search).

Evaluate `0x140931830(curve, d, scale)`:
```
if count < 2 → 0
if d >= scale → y_last
t = d / scale; piecewise-linear in t (binary search); t < x0 → y0; t > x_last → y_last
```
Variant `0x140931920(curve, d, lo, hi)` uses `t = (d - lo)/(hi - lo)` (for emitter/panner).

Built-in default curves (`0x14004a330`, overridden by `CfgSoundGlobals`
`defaultVolumeCurve / defaultWeaponVolumeCurve / defaultLFEVolumeCurve` in `0x14092bc70`):

| Bank index | Name | Points |
|---|---|---|
| 0 | defaultVolumeCurve | (0,1) (.01,.7) (.035,.45) (.085,.25) (.14,.15) (.22,.09) (.325,.05) (.45,.02) (.7,.01) (1,0) |
| 1 | defaultWeaponVolumeCurve | (0,1) (.0025,.7) (.0088,.45) (.0213,.25) (.035,.15) (.0812,.1) (.175,.075) (.7,.01) (1,0) |
| 2 | defaultLFEVolumeCurve | (0,.55) (.0625,.25) (.125,.1) (.25,.05) (1,0) |
| 3 | (linear) | (0,1) (1,0) |

### 2.2 Sound shader (`CfgSoundShaders`, loader `0x140945220`, object 0xd0 bytes) (high)

| Key | Storage | Default / clamp |
|---|---|---|
| `samples[] = {{path, p},...}` | `+0x20` array {path, prob} | entries with < 2 elements are skipped; probabilities normalised to sum 1 (`0x140945b30`; a single sample gets 1) |
| `volume` | number → `+0x38`; string starting `db` (`d` lowercase, `b` any case) → `10^(x/20)`; other string → expression (`+0x40`, flag bit 0) | `invalid db value %s` on parse error |
| `frequency` | number → `+0x84`; string → expression (`+0x88`, flag bit 1) | |
| `volumeFactor` | `+0x80` | 1, clamped [0, 10^(10/20)=3.1623] |
| `rangeCurve` | `+0xc8` | none (null) |
| `limitation` | flag bit 2 | false |
| `range` | `+0x10` | 0 |
| `category` | `+0x18` | |

Shader volume (`0x140945080` / `0x1409451c0`): constant → `volumeFactor * volume`.
**Expression → the expression value only; volumeFactor is not applied** (as compiled;
medium that this is not compensated elsewhere). Frequency: constant or expression value.

Sample choice (`0x140945ec0`): `r = rand01()`. Subtract the normalised probabilities in
order and take the first index where `r` drops below 0 (else the last). If the chosen index
equals the previous one for this slot, use `(index + 1) % count` (`0x140945110`).

### 2.3 Sound set (`CfgSoundSets`, loader `0x140943ae0`) (high)

| Key | Offset | Default / clamp | Meaning |
|---|---|---|---|
| `volume` (`0x140944cb0`) | `+0x1c` or expr `+0x28` (flag 0x400) | number, `db..` string or expression | set-level volume |
| `volumeFactor` | `+0x60` | 1, [0, 3.1623] | |
| `volumeRandomizer` | `+0x64` | 1, [1, 10^(6/20)=1.995] | **linear** ratio, converted to dB at use |
| `volumeRandomizerMin` | `+0x68` | 1, [1, 1.995] | |
| `frequencyFactor` | `+0x10` | 1, [2^-1, 2^1] = [0.5, 2] | |
| `frequencyRandomizer` | `+0x14` | 0, [0, 12] | **semitones** |
| `frequencyRandomizerMin` | `+0x18` | 0, [0, 12] | semitones |
| `soundShadersLimit` | byte `+0x6c` | 0 | |
| `volumeCurve` | `+0x70` | null → bank 0 (defaultVolumeCurve) | |
| `loop` | flag 1 | false | |
| `spatial` | flag 2 | true | |
| `doppler` | flag 4 | true | |
| `speedOfSound` | flag 8 | true | (consumer not traced) |
| `occlusionObstruction` | flag 0x80 | true | |
| `sound3DProcessingType` | `+0x78` | `CfgSoundGlobals.defaultSound3DProcessingType` | `CfgSound3DProcessors` class |
| `distanceFilter` | `+0x80` | `CfgSoundGlobals.defaultDistanceFilter` | `CfgDistanceFilters` class |
| `spatialityRange` | `+0x88` | 0.5 | X3DAudio InnerRadius (m) |
| `spatialityRangeAngle` | `+0x8c` | 0.7854 | X3DAudio InnerRadiusAngle (rad) |
| `posOffset[3]` | `+0x9c` | flag 0x1000 | |
| `delay` | `+0x90` | -1 (none); < 0.01 → -1; ≥ 0.01 clears `loop` | s |
| `delayRandomizer` / `Min` | `+0x94/+0x98` | 0, clamped [0, delay] | |
| `occlusionFactor` | `+0xa8` | 0.96, [0,1] | consumer not traced |
| `obstructionFactor` | `+0xac` | 0.7, [0,1] | consumer not traced |
| `shape` | `+0xb0` | CfgSoundShapes (cone: `innerVolume`, `outerVolume` dB, `innerAngle`, `outerAngle` deg, `azimuth`, `elevation`; `0x140946150`) | |
| `playTrigger` | expr, flag 0x100 | | |
| `customCategory` | `+0xf8` | | |

`soundShaders[]` is read elsewhere (not traced). The SoundObject holds per shader
`{?, volume +8, frequency +0xc, shader* +0x10}` (stride 0x18).

### 2.4 Randomisation at sound start (`0x140932390`) (high)

```
dbRand(maxV, minV) = let m = minV + rand01()*(maxV - minV) in (rand01() > 0.5 ? m : -m)   // 0x140932290
linToDb(x) = max(20*log10(x + 1e-25), -100)                                                  // 0x140924f30
setVolume = 10^(dbRand(linToDb(volumeRandomizer), linToDb(volumeRandomizerMin))/20)
            * volumeFactor * volume                      // SoundObject+0xc8
setFreq   = 2^(dbRand(frequencyRandomizer, frequencyRandomizerMin)/12) * frequencyFactor
                                                         // SoundObject+0xcc
```
So the randomizer gives a symmetric ± offset between Min and Max: in dB for volume (the
config values are linear ratios ≥ 1) and in semitones for frequency.

### 2.5 Combining into a gain (high for the parts listed, medium for the full product)

Per voice (spatial set):

1. **Per shader**, `d` = |listener − emitter| (`0x140966eb0`):
   `shaderGain = (d <= shader.range) ? (rangeCurve ? curve(d / shader.range) : 1) : 0`.
   Multi-shader path: a shader beyond its `range` is silent. Single-shader fast path
   `0x140968350`: `vol = shaderVol * setVolume * curve(d / range)` without the hard cut
   (past range the curve gives y_last).
   `vol_i = shaderVolume_i * setExprVolume * shaderGain_i`.
2. **soundShadersLimit**: shaders with `limitation = 1` that have samples and
   `vol_i > dB(-50) = 0.00316` are collected, sorted by `vol_i` **descending**, and only the
   first `soundShadersLimit` are played. Shaders without `limitation` play whenever
   `vol_i > dB(-50)`.
3. **Set-level distance curve**: the voice's X3DAudio emitter gets `pVolumeCurve =
   volumeCurve` (or defaultVolumeCurve) and `pLFECurve = defaultLFEVolumeCurve`.
   `CurveDistanceScaler = max(1, max_i shader_i.range)`: `0x140966eb0` raises the voice's
   `+0x40` to the largest shader range, and `0x1409652d0` clamps it to ≥ 1. X3DAudio (from
   XAudio2_8.dll) evaluates the curve piecewise-linearly at `d / scaler` and holds the last
   point beyond 1. It also applies the panning matrix.
4. `setVolume` (2.4) multiplies the voice.

So the effective distance gain is `rangeCurve_shader(d/range_shader) *
volumeCurve_set(d/maxRange)`. With an empty `rangeCurve` it is a step at `range`.

### 2.6 X3DAudio setup (`0x1409652d0`, `0x140958970`) (high)

- `X3DAudioInitialize(mask, SpeedOfSound = 343.5, handle)` (`0x14094a540`, constant at
  `0x141b5d1d4`).
- Flags `MATRIX | LPF_DIRECT | DOPPLER | REDIRECT_TO_LFE` (0x20025) when the device has an
  LFE (`0x14094a90e`). `DOPPLER` (0x20) is cleared for sets with `doppler = 0`.
- Emitter: position/velocity of the source, OrientFront/Top, `InnerRadius =
  spatialityRange`, `InnerRadiusAngle = spatialityRangeAngle`, `ChannelCount` = source
  channels, `DopplerScaler = 1`, curves as in 2.5.

---

## 3. CfgDistanceFilters (`Sound::DistanceFilter`, vtable `0x141b5a880`) (high)

Loader `0x14092fa50` + `0x140930120`:

| Key | Offset | Default / clamp |
|---|---|---|
| `type` | `+0x18` | `lowPassFilter`=1, `bandPassFilter`=2, `highPassFilter`=4, `notchFilter`=8, `noneFilter`=0x10. Unknown or missing → 1 (low pass) |
| `minCutoffFrequency` | `+0x1c` | 44100 |
| `qFactor` | `+0x20` | 1 |
| `innerRange` | `+0x24` | 100, ≥ 0 |
| `range` | `+0x28` | 1500, ≥ innerRange |
| `powerFactor` | `+0x2c` | 2, clamped [0.0001, 100] |

Cutoff `0x14092f790(filter, vars)` with `d = distance`, `fs = sampleRate` (voice rate):
```
d > range        → minCutoffFrequency
d < innerRange   → fs                                  (effectively unfiltered)
else t = ((d - innerRange)/(range - innerRange))^(1/powerFactor)
     cutoff = fs + (minCutoffFrequency - fs) * t       (t outside [0,1] → minCutoff / fs)
```
Apply `0x140955680`: XAudio2 `FilterParameters { Type = {LowPass 0, BandPass 1, HighPass 2,
Notch 3}[type], Frequency = (cutoff*6 < fs) ? 2*sin(pi*cutoff/fs) : 1.0, OneOverQ =
qFactor }`. **qFactor is passed as OneOverQ unchanged.** `noneFilter` → no filter call.
The filter is applied only for spatial voices (flag bit 6) that have a distance filter and
flag `+0x90 & 1`. Otherwise the filter is reset (`0x140954570`).

---

## 5. CfgSound3DProcessors (medium)

Type ids (`0x1401bc540`): 1 = `Sound::MultiChannelEmitter` (`type = "emitter"`), 2 =
`Sound::MultiChannelPanner` (`"panner"`), 3 = `Sound::SurroundPanner` (non-spatial
surround).

Emitter (loader `0x140930e90`): `innerRange` `+0x28` (default 1, ≥ 0), `range` `+0x24`
(default 4, ≥ innerRange), `radius` `+0x20` (default 3), `rangeCurve` `+0x30` (default:
linear bank 3). Used for multichannel sources when there is no panner (`0x140958a20`):
OrientFront = normalize(emitter − listener). X3DAudio channel azimuths from `0x140930580`
(d = distance):

| Channel | d < inner | inner..range (linear in (d-inner)/(range-inner)) | d > range |
|---|---|---|---|
| 0 | 225° | 225° → 180° | 180° |
| 1 | 45° | 45° → 180° | 180° |
| 2 | 135° | 135° → 180° | 180° |
| 3 | -45° | -45° → 180° | 180° |

`ChannelRadius = radius * curve((d - inner)/(range - inner))` (`0x140930740`), `radius` for
d < inner, 0 for d > range. Near the listener the channels spread around; far away they
collapse into one point.

Panner (loader `0x140931450`): `innerRange` `+0x20` (default 0), `range` `+0x24` (default 0),
`rangeCurve` `+0x28`. Processing `0x140957780`, d = distance, S = CurveDistanceScaler:
- `d >= range`: plain X3DAudio point source.
- Else: non-spatial matrix (mono `0x140959110`; multichannel `0x140959240` with gains
  1,1,1,1 and 0.4) times `volumeCurve(d/S)`.
- If also `d > innerRange`: `w = curve((d - inner)/(range - inner))` (1 for d < inner, 0 for
  d > range). X3DAudio is computed with the listener moved to `range` metres from the
  emitter along the true direction. That result is scaled by `volumeCurve(innerRange/S)`,
  and `out = w * spatial + (1 - w) * nonSpatial`. (medium: the blend direction depends on the
  curve orientation in config)

Spatiality: `spatialityRange`/`spatialityRangeAngle` are passed directly as X3DAudio
`InnerRadius`/`InnerRadiusAngle` (high). The `CfgSoundGlobals.defaultSpatialityRange*`
consumer was not traced (unknown).

## 6. Doppler, occlusion

- Doppler comes from X3DAudio (`DSP.DopplerFactor`, speed of sound 343.5 m/s,
  DopplerScaler 1). Final frequency ratio = `baseFreq (voice+0xe0) * dopplerFactor`,
  clamped to [0.5, 2.0] (`0x140955660`, constants `0x141b5a7b4`). Applied only when the set
  has `doppler = 1` (high).
- `occlusionFactor` (0.96) and `obstructionFactor` (0.7) are loaded and clamped [0,1]. Their
  consumer was not traced (unknown).

## 7. Old-style `sound[] = {path, volume, pitch, distance}` (high for the dB rule)

- **Any** config number read as a float accepts the string form `db<number>`
  (`0x1402bd0e0`, `0x1402b2e50`): `"db-10"` → `10^(-10/20) = 0.3162`. Otherwise the config
  value is parsed as a number, or evaluated as an expression. So `volume` in `sound[]` is a
  linear gain, and `db..` is just a shortcut (`d` and `b` lowercase here). Pitch is a
  frequency ratio.
- `CfgSoundGlobals.OldConfigurationVolumeFactor` (default 1 if absent, `_DAT_1420a79a8`) is
  multiplied into every volume set through the legacy voice `SetVolume` (`0x140965ad0`,
  vtable slot `+0xe0`). Legacy CfgEnvSounds, CfgSounds/CfgSFX/old vehicle sounds use it; the
  sound-set path uses a different setter (`0x140965b20`) and is not scaled. (medium on the
  exact list of legacy callers)
- The legacy `distance` (4th element) consumer was not traced (unknown).

## Function index

| VA | Role |
|---|---|
| 0x1403829c0 | create expression GameState, register 46 EXPRESSION overloads |
| 0x14037fec0 / 0x140380460 | compile expression with variable names |
| 0x1403815f0 | evaluate expression (RPN, finite check) |
| 0x140380e50 / f50 / d40 | factor / interpolate / envelope steps |
| 0x14030e310 | engine LCG rand01 |
| 0x140931c40 / 0x140931830 / 0x140931920 | curve parse (x normalised) / eval d/scale / eval (d-lo)/(hi-lo) |
| 0x14004a330 / 0x14092bc70 | built-in default curves / CfgSoundGlobals overrides |
| 0x140945220 / 0x140945950 | sound shader loader / shader volume |
| 0x140943ae0 / 0x140944cb0 | sound set loader / set volume |
| 0x140932390 | SoundObject start: sample pick, randomizers |
| 0x140932a30 | push new variable values, re-evaluate volumes |
| 0x140966eb0 / 0x140968350 | multi-shader mixing with rangeCurve + soundShadersLimit / single shader |
| 0x1409652d0 / 0x140958a20 / 0x140958970 | voice 3D update / processor dispatch / X3DAudioCalculate |
| 0x14092fa50 / 0x14092f790 / 0x140955680 | distance filter load / cutoff / XAudio2 filter params |
| 0x140930e90 / 0x140931450 / 0x140957780 | emitter load / panner load / panner processing |
| 0x140928d60 | 22 environment values |
| 0x14163bc20 / 0x14163dc80 | sound map bilinear sampler / cell fetch |
| 0x14104cbb0 / 0x141049390 | [InitSoundMap] generator / splat |
| 0x14092a630 / 0x14092a7a0 / 0x140929ff0 | env update / soundSetEnvironment / legacy env |
| 0x1402bd0e0 | config float parse with `db` prefix |
| 0x14094e670 | CfgSoundGlobals (OldConfigurationVolumeFactor) |
