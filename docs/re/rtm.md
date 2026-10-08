# RTM animation format

Implemented in `crates/a3-rtm`. Findings come from parsing every `.rtm` in the install (build
2.22.0.154103), not from the executable. Confidence is given per item: **high** = holds for every
shipped file and the meaning is clear; **medium** = holds for every file, meaning inferred;
**low** = not present in the install, from community notes.

## Survey

5,870 `.rtm` files in the VFS: 5,831 binarized `BMTR` version 5 and 39 plain `RTM_0101` (fish,
turtles, butterflies, two parachute poses). No `RTM_MDAT` section and no other BMTR version.
Every file decodes with no trailing bytes: 739,588 keyframes, 75.7 M bone transforms, 878 events
(all `StepSound` with an empty value). Most character moves have 103 bones.

All integers and floats are little-endian. `asciiz` = NUL-terminated string.

## Plain `RTM_0101` (high)

```
char[8]  "RTM_0101"
f32[3]   step            move vector of one cycle
u32      frame_count
u32      bone_count
char[32] bone_names[bone_count]          NUL-padded
frame[frame_count]:
  f32      phase
  { char[32] bone_name; f32[12] matrix } [bone_count]   same order as the header
```

The 12 floats are three orientation columns (aside, up, direction) then the translation. Every
shipped orientation is orthonormal to within 0.002, so `a3-rtm` converts it to a quaternion.
_Medium_: columns, not rows. An identity-plus-small-rotation sample (`Butterfly_Fly.rtm`) reads
as a rotation about +Z either way; the sign of the angle depends on this choice.

### `RTM_MDAT` (low; not in the install)

An optional section before `RTM_0101`:

```
char[8]  "RTM_MDAT"
u32      0
u32      count
{ f32 phase; u32 len; char name[len]; u32 len; char value[len] } [count]
```

followed by the plain `RTM_0101` data. `a3-rtm` reads these as events.

## Binarized `BMTR` version 5

```
char[4]  "BMTR"
u32      version          5                                          high
u8       ?                1 in every file                            unknown
f32[3]   step             move vector                                high
u32      phase_count                                                 high
u32      ?                0 (5,053 files) or 1 (778 files)           unknown
u32      bone_count                                                  high
u32      bone_name_count  == bone_count in every file                high
asciiz   bone_names[bone_name_count]   lower case                    high
u32      ?                0 in every file (a list that is always empty; a3-rtm rejects non-zero)
u32      event_count                                                 high
event[event_count]:
  u32    ?                0 in every file                            unknown
  asciiz name             "StepSound"                                high
  f32    phase                                                       high
  asciiz value            "" in every file                           medium
array<f32>        phase_times    count == phase_count                high
array<transform>  transforms[phase_count]   count == bone_count each high
```

`array<T>`: `u32 count`, `u8 flag`, then the `count * sizeof(T)` element bytes, stored as-is when
`flag = 0` or as one LZO1X block when `flag = 2` (decompressed size known from the count; the
block ends with the LZO end marker). No other flag occurs. Arrays of 1,024 bytes or more are
compressed, smaller ones are not _(medium: observed threshold)_.

`transform` (14 bytes):

```
i16[4]   quaternion x, y, z, w   divide by 16384                     high
f16[3]   translation x, y, z     IEEE binary16                       high
```

_Medium_: the quaternion and the plain matrix describe the same rotation when the quaternion
is read as (x, y, z, w) and turned into a matrix by the usual right-handed formula (`glam`).
No animation ships in both encodings, so this is not cross-checked yet.

### Non-unit quaternions

251 of the 75.7 M stored quaternions, in 21 files, are not unit length (beyond 1%): parachute
open/land (lengths 0.3 to 0.75, a scaled canopy), bird flight, the crane, several cutscene moves,
`static_dead_10..13` (an all-zero quaternion), and `mortar_01_turret.rtm` (lengths above 2 with a
translation of hundreds of metres, likely a degenerate source matrix). The binarizer seems to fold
a uniform scale into the quaternion length _(low)_. `a3-rtm` keeps the stored values;
interpolation normalises its result.

## Phases and the move vector

- Phase times ascend in every file. 5,701 files start at 0; most end at 1. One file has NaN
  phase times; 27 files have every phase at 1.0. **High** (observed).
- Plain RTMs repeat the bone name per frame; it always matches the header name, ignoring case.
- `step`: forward moves have a negative Z step (the standard rifle walk
  `amovpercmwlksraswrfldf.rtm` steps `(0, 0, -1.62)` per cycle; fish swim cycles `(0, 0, -0.5)`
  to `-2`). How the engine applies it (sign, per-cycle distance divided by the move's `speed`
  config value) is **unknown**.

## Open questions

- Meaning of the unknown header byte and `u32`, the always-empty list and the per-event `u32`.
- Whether transforms are relative to the skeleton's bind pose or absolute in model space, and the
  bone order mapping onto a CfgSkeletons skeleton. Needed for Phase 4 animation.
- How the engine samples between keyframes (linear vs. slerp, looping past the last phase).
