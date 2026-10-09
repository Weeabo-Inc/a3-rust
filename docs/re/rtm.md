# RTM animation format

Implemented in `crates/a3-rtm`. Findings come from parsing every `.rtm` in the install (build
2.22.0.154103) and from the BMTR serializer of `arma3_x64.exe` (`0x1412557a0`, a read/write
function; addresses are VAs in the Ghidra project). Confidence is given per item: **high** = holds
for every shipped file and the meaning is clear (or read from the executable); **medium** = holds
for every file, meaning inferred; **low** = not present in the install, from community notes.

## Survey

5,870 `.rtm` files in the VFS: 5,831 binarized `BMTR` version 5 and 39 plain `RTM_0101` (fish,
turtles, butterflies, two parachute poses). No `RTM_MDAT` section and no other BMTR version.
Every file decodes with no trailing bytes: 739,588 keyframes, 75.7 M bone transforms, 878
keystones (all `StepSound`, type 0, with an empty value). Most character moves have 103 bones.

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

followed by the plain `RTM_0101` data. `a3-rtm` reads these as keystones of type -1.

## Binarized `BMTR` version 5

```
char[4]  "BMTR"
u32      version          5 (the engine accepts 2..=5; layouts of 2..4 unknown)      high
u8       ?                1 in every file; stored at anim+0x10c                      unknown
f32[3]   step             move vector                                                high
u32      phase_count                                                                 high
u32      ?                0 (5,053 files) or 1 (778 files); engine keeps (v > 0) as a flag
u32      first-phase bone count   written by the binarizer, skipped by the reader    high
u32      bone_count                                                                  high
asciiz   bone_names[bone_count]   lower case                                         high
-- version >= 4:
u32      name_count       0 in every file
asciiz   names[name_count]          purpose unknown                                  medium
u32      keystone_count                                                              high
keystone[keystone_count]:
  i32    type             keystone type number; -1 = resolve from name           high
  asciiz name             "StepSound" (type 0) in every file                     high
  f32    phase                                                                   high
  asciiz value            "" in every file                                       high
--
array<f32>        phase_times    count == phase_count                                high
array<transform>  transforms[phase_count]   count == bone_count each                 high
```

The engine calls the phase events "animation keystones"; a name it cannot resolve logs
`Invalid animation keystone name: "%s"`. It loads the first phase's transforms at once and keeps
the file open to stream the others.

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
No animation ships in both encodings, so this is not cross-checked yet. **High**: the engine's
decoder (`0x1212520`) fills the columns of its matrix with the rows of that usual matrix, so the
rotation it applies is the quaternion's **conjugate**. It also rewrites each translation on load
using the skeleton pivots; see `model-animations.md` ("RTM skeletal poses").

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

- Meaning of the header byte, the 0/1 flag and the always-empty name list; the keystone type
  numbering (the name-to-type table).
- ~~Whether transforms are relative to the skeleton's bind pose or absolute~~: answered in
  `model-animations.md` ("RTM skeletal poses"): each record is the bone's frame relative to its
  parent, stored a half turn about Y from the model space, bound by name.
- How the engine samples between keyframes (linear vs. slerp, looping past the last phase).
