# P3D models: ODOL v73 and MLOD

Implemented in `crates/a3-p3d`. Game build 2.22.0.154103.

**How this was established.** Field order and sizes come from community format notes (BI
community wiki "P3D File Format - ODOLV4x/ODOLV7x", Mikero's notes), then were checked against
the shipped data with an exploratory parser: a layout counts as _verified_ when, for **every one of
the 10,472 `.p3d` files** in the install, parsing it ends exactly at the next structure (the header
ends where the first LOD starts; every LOD ends exactly at its end offset from the LOD table, and
`size_of_rest` matches). Verified sizes and order are **high** confidence. Field _meanings_ are
taken from the community notes unless stated; meanings marked _(uncertain)_ were not confirmed in
the executable.

## Survey

| what | count |
|---|---|
| `.p3d` files in the VFS (base game + DLC, EBOs excluded) | 10,472 |
| ODOL v73 | 10,472 (100 %) |
| MLOD, other ODOL versions | 0 |

So the shipped game needs ODOL v73 only. Older versions (Arma 2 era 40..72) can appear in mods
and are not supported yet.

## Conventions

Little-endian. `asciiz` = NUL-terminated string. `bool` = 1 byte. `vec3` = 3 × f32 (x, y, z;
the engine's left-handed space: x right, y up, z forward). `i8 lod` = LOD index, -1 for none.

### Compressed array (`carray<T>`)

```
u32  count
if count * sizeof(T) > 0:
    u8   flag            // 0: raw bytes follow, 2: an LZO1X stream follows
    [count * sizeof(T) bytes, raw or LZO1X-compressed]
```

The compressed size is not stored: the LZO stream is decoded until `count * sizeof(T)` output bytes
are produced, and reading continues after the last input byte consumed (the stream's `11 00 00`
end marker included). A zero count has no flag byte. _Verified_ for v73.

The flag, not the size, decides: in a sample of 1,313 arrays, 78 arrays under 1024 bytes were
LZO and 20 arrays of 1024 bytes or more were raw. (The Arma 2 rule "compressed when 1024 bytes
or more" does not hold for v73.) No other flag value occurs.

### Condensed array (`fill<T>`)

```
u32  count
bool default_fill                     // present even when count is 0
if default_fill: T value              // every element equals value
else:            the bytes of carray<T> after its count (flag + data; nothing when count is 0)
```

_Verified_ for the arrays marked `fill` below, including LODs with no vertices (proxy-only and
empty LODs store `count = 0, default_fill = 0`).

## File layout

```
"ODOL"
u32      version                 // 73
u32      app_id                  // Steam app ID: 107410 base game, DLC IDs otherwise; 0 in a few
asciiz   muzzle_flash            // usually ""; a muzzle-flash proxy path (or selection name) in 13 % of vehicles
u32      lod_count
f32      resolution[lod_count]   // see LodResolution
ModelInfo
bool     has_animations
if has_animations: Animations
u32      lod_start[lod_count]    // absolute file offsets
u32      lod_end[lod_count]
bool     permanent[lod_count]
for each LOD with permanent == false, in LOD order:
    LodSummary                   // 25 bytes
[LOD data at lod_start[i] .. lod_end[i]]   // not necessarily in LOD order
```

`LodSummary` (_verified_ size and position):

```
u32 face_count; u32 color; u32 special; u32 or_hints; bool has_skeleton; u32 vertex_count; f32 face_area
```

The values match the LOD's own data (e.g. `face_area`).

### ModelInfo (v73, _verified_)

| field | type | notes |
|---|---|---|
| special | u32 | model flags |
| bounding_sphere, geometry_sphere | f32 ×2 | |
| remarks, and_hints, or_hints | u32 ×3 | |
| aiming_center | vec3 | |
| color, color_type | u32 ×2 | packed ARGB |
| view_density | f32 | |
| bbox_min, bbox_max | vec3 ×2 | over **all** LODs (includes memory points) |
| lod_density_coef, draw_importance | f32 ×2 | |
| bbox_visual_min, bbox_visual_max | vec3 ×2 | over visual LODs |
| bounding_center, geometry_center, center_of_mass | vec3 ×3 | |
| inv_inertia | 9 × f32 | symmetric, so row/column order does not matter |
| auto_center, lock_auto_center, can_occlude, can_be_occluded, ai_covers | bool ×5 | |
| ht_min, ht_max, af_max, mf_max, m_fact, t_body | f32 ×6 | thermal imaging _(meanings uncertain)_ |
| force_not_alpha | bool | |
| sb_source | i32 | shadow source _(uncertain)_ |
| prefer_shadow_volume | bool | |
| shadow_offset | f32 | `f32::MAX` when unset |
| animated | bool | |
| Skeleton | | below |
| map_type | u8 | |
| mass_array | carray\<f32\> | always empty in shipped models |
| mass, inv_mass, armor, inv_armor, explosion_shielding | f32 ×5 | `inv_mass` = 1e10 when mass is 0 |
| special LOD indices | i8 ×14 | order: memory, geometry, geometry_simple, geometry_physx, fire_geometry, view_geometry, view_pilot_geometry, view_gunner_geometry, view_commander_geometry (always -1), view_cargo_geometry, land_contact, roadway, paths, hitpoints. _Verified_ against the resolution of the LOD each index points to. Without a 7e15 LOD, fire_geometry points at the View Geometry or, failing that, the Geometry; without a 6e15 LOD, view_geometry points at the Geometry (counts over all shipped models: fire → Fire 2438, View 1299, Geometry 2602; view → View 3482, Geometry 2853). |
| min_shadow | u32 | mostly equals `lod_count` _(meaning uncertain)_ |
| can_blend | bool | |
| class, damage | asciiz ×2 | model properties |
| frequent | bool | |
| unknown | u32 | always 0 |
| preferred_shadow_volume_lod | i32 × lod_count | |
| preferred_shadow_buffer_lod | i32 × lod_count | |
| preferred_shadow_buffer_lod_visible | i32 × lod_count | |

### Skeleton (_verified_)

```
asciiz name                     // "" = no skeleton, nothing else follows
bool   inherited
u32    bone_count
{ asciiz bone; asciiz parent; } [bone_count]   // parent "" = root
asciiz pivots_model
```

Parent names are matched case-insensitively. Some shipped models name a parent that is not a bone
of the skeleton (e.g. `plane_civil_01_basic_f.p3d`: bone `lights_hide`, parent `zbytek`); the
reader keeps the name and treats the bone as a root.

### Animations (_verified_)

```
u32 class_count
AnimationClass[class_count]
u32 table_lod_count             // lod_count, or 0 when no table follows (24 shipped models)
for each LOD:  u32 bone_count; { u32 n; u32 anim_index[n]; } [bone_count]   // bone -> animations
for each LOD:  for each class:
    i32 bone                    // skeleton bone index, -1 = not in this LOD
    if bone != -1 and type is not direct (8) or hide (9): vec3 axis_pos; vec3 axis_dir
```

AnimationClass:

```
u32    type        // 0 rotation, 1 rotationX, 2 rotationY, 3 rotationZ,
                   // 4 translation, 5 translationX, 6 translationY, 7 translationZ, 8 direct, 9 hide
asciiz name
asciiz source
f32    min_value, max_value, min_phase, max_phase
f32    anim_period
f32    init_phase
u32    source_address   // 0 clamp, 1 mirror, 2 loop
type 0..7:  f32 angle0/offset0, f32 angle1/offset1
type 8:     vec3 axis_pos, vec3 axis_dir, f32 angle, f32 axis_offset
type 9:     f32 hide_value, f32 unhide_value
```

Types seen in a 10 % sample: rotation 937, rotationX 69, rotationY 22, rotationZ 12, translation
252, hide 974; direct only in the two Enoch hunter shotguns.

## LOD layout (v73, _verified_ on every LOD of every shipped model)

```
u32     proxy_count;   Proxy[proxy_count]
u32     n;  u32 sub_skeleton_to_skeleton[n]      // LOD bone index -> skeleton bone index
u32     n;  { u32 k; u32 lod_bone[k]; }[n]       // skeleton bone -> LOD bones
u32     vertex_count
f32     face_area
u32     or_hints, and_hints                      // clip flags
vec3    bbox_min, bbox_max, bbox_center; f32 bbox_radius
u32     n;  asciiz texture[n]
u32     n;  EmbeddedMaterial[n]
carray<u32> point_to_vertex                      // empty in every shipped LOD
carray<u32> vertex_to_point                      // empty in every shipped LOD
u32     face_count
u32     face_alloc_size                          // sum over faces of 4 + 4 * vertex_count
u16     zero
{ u8 n; u32 vertex[n]; }[face_count]             // n = 3 or 4; u32 indices in v73
u32     n;  Section[n]
u32     n;  NamedSelection[n]
u32     n;  { asciiz key; asciiz value; }[n]     // named properties
u32     n;  { f32 time; u32 k; vec3 pos[k]; }[n] // frames; none shipped
u32     icon_color, selected_color
u32     special
bool    vertex_bone_ref_is_simple
u32     size_of_rest                             // bytes from here up to and including `unknown_u32` below
fill<u32>        clip_flags                      // per vertex
UVSet            uv0
u32              uv_set_count                    // including uv0; 1 or 2 in shipped data
UVSet            uv[uv_set_count - 1]
carray<vec3>     positions
fill<u32>        normals                         // compressed, see below
carray<STPair>   tangents                        // 8 bytes each, 0 or vertex_count
carray<BoneWeights> vertex_bone_ref              // 12 bytes each, 0 or vertex_count
carray<NeighborBones> neighbor_bone_ref          // 32 bytes each, 0 or vertex_count
u32     unknown_u32                              // 0 in shipped data
u8      unknown_u8                               // 1 in all but one LOD of a 10 % sample
```

Face winding and quad split: a quad `a b c d` renders as `(a,b,c)` and `(a,c,d)` _(medium)_.

### Section (_verified_)

```
u32 face_start, face_end    // BYTE offsets into the face block, in units of the in-memory face
                            // size (4 + 4 * n), not face indices; the last end = face_alloc_size
u32 min_bone, bone_count
u32 mat_dummy               // 0
i16 texture                 // index into textures, -1 none
u32 face_flags
i32 material                // index into materials, -1 none
if material == -1: asciiz material_name        // always "" in shipped data
u32 n; f32 area_over_tex[n]                    // n = 2 in all shipped sections
u32 unknown                                    // 0 except in one section of a 10 % sample
if unknown != 0: f32 unknown_floats[11]
```

### NamedSelection (_verified_)

```
asciiz      name
carray<u32> faces           // face indices
u32         zero
bool        sectional
carray<u32> sections
carray<u32> vertices
carray<u8>  vertex_weights  // empty or one per selected vertex _(meaning of the byte uncertain)_
```

### Proxy (_verified_)

```
asciiz model
f32    orientation[9]       // 3x3, determinant ±1 on all 39,453 shipped proxies
vec3   position
i32    sequence_id          // the .NNN suffix
i32    named_selection      // the selection "proxy:<model>.<NNN>" (true for every shipped proxy)
i32    bone                 // skeleton bone, -1 none
i32    section              // -1 none
```

The proxy triangle itself is not kept: the proxy's named selection has no vertices in ODOL.
Whether the 9 orientation floats are rows or columns is not settled _(medium)_.

### EmbeddedMaterial (_verified_ for material version 11, the only one shipped)

```
asciiz name                 // rvmat path
u32    version              // 11
f32    emissive[4], ambient[4], diffuse[4], forced_diffuse[4], specular[4], specular2[4]
f32    specular_power
u32    pixel_shader, vertex_shader, main_light, fog_mode
asciiz surface              // .bisurf path
u32    render_flag_count?, render_flags   // two u32 _(meaning uncertain)_
u32    stage_count, tex_gen_count
{ u32 filter; asciiz texture; u32 tex_gen; bool use_world_env_map; }[stage_count]
{ u32 uv_source; f32 transform[12]; }[tex_gen_count]
{ u32 filter; asciiz texture; u32 tex_gen; bool use_world_env_map; }   // the TI stage
```

### UVSet (_verified_)

```
f32       min_u, min_v, max_u, max_v
fill<i16 pair>  uv          // per vertex
```

`u = min_u + (raw_u + 32767) / 65534 * (max_u - min_u)`, same for v. Raw values span
-32767..=32767 _(high: decoded UVs fall within the stored range)_.

### Compressed normal / tangent vector (u32)

Three signed 10-bit fields: x = bits 0..9, y = 10..19, z = 20..29 (bits 30..31 zero). A field
value `f` in -512..=511 decodes to `f / 511`. _High_: decoded normals and tangents have length
1 ± 0.01 (or are zero in degenerate geometry).

**Sign.** Community readers decode `-f / 511`; with that sign the normals of a closed convex model
(`bottleplastic_v1_f.p3d`: 79 of 79 vertices) point **inward**. With `+f / 511` they point outward,
and the right-handed cross product `(b - a) x (c - a)` of each face agrees with its vertex normals
(MX rifle 8144/8144, house 7078/7078, soldier 8697/8775 triangles). So faces are wound clockwise
seen from outside in the engine's left-handed space (the D3D front-face convention). The reader
returns outward normals. The tangent sign uses the same rule _(unverified)_.

`STPair` = two compressed vectors (S then T).

### BoneWeights (12 bytes) and NeighborBones (32 bytes)

```
BoneWeights:   u32 count; { u8 bone; u8 weight; }[4]      // bone = LOD bone index (sub_skeleton)
NeighborBones: u16 pos_a; u16 pad; BoneWeights rtw_a; u16 pos_b; u16 pad; BoneWeights rtw_b
```

Over every shipped vertex with a bone-weight entry: 31.1 M have `count = 0` (unskinned), 15.6 M
one bone with weight 255, 2.5 M several bones whose weights sum to exactly 255. _High_.

## Geometry checks over the whole install

The real-data test (`crates/a3-p3d/tests/real_data.rs`) decodes all 68,522 LODs (92.2 M vertices,
74.2 M faces) and checks: every position lies inside its LOD's stored bounding box; every
normal has length 1 ± 0.01 or is zero (381 k zero normals, all in degenerate geometry);
sections are contiguous and cover every face; proxies as described above; bone weights as
above. Release build: ~15 s including reading 3 GB from the PBOs.

## LOD resolutions

`LodResolution` in `crates/a3-p3d/src/resolution.rs` lists the named values (1e13 Geometry, 1e15
Memory, 6e15 View Geometry, 7e15 Fire Geometry, ...). Shipped models also use 10000+n (shadow
volume) and 11000+n (shadow buffer _(uncertain name)_). Every special LOD index in ModelInfo
points at a LOD of the expected kind (checked by the real-data test).

## MLOD

```
"MLOD"; u32 version (257); u32 lod_count
per LOD:
  "P3DM"; u32 header_size (28); u32 version (0x100)
  u32 point_count, normal_count, face_count, flags
  { vec3 pos; u32 flags; }[point_count]
  vec3 normal[normal_count]
  { u32 n (3|4); { u32 point; u32 normal; f32 u; f32 v; }[4]; u32 flags; asciiz texture; asciiz material }[face_count]
  "TAGG"
  { bool active; asciiz name; u32 size; u8 data[size]; }...   until name "#EndOfFile#"
  f32 resolution
```

Tags: `#Property#` (64-byte key + 64-byte value), `#Mass#` (f32 per point), `#UVSet#` (u32 id, then
u, v per face corner), `#SharpEdges#` (u32 point pairs), `#Animation#` (f32 time + vec3 per point),
editor state (`#Selected#`, `#Hidden#`, `#Lock#`) ignored; any name without `#` is a named
selection: one weight byte per point (0 = not selected, 1 = full, other values partial
_(uncertain encoding)_) then one byte per face. No MLOD file ships with the game; the reader is
tested on synthetic files only. Older `SP3X` LODs are not supported. Whether MLOD normals point
the same way as ODOL normals is unconfirmed.
