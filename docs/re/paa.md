# PAA/PAC textures, texHeaders.bin, procedural textures

Implemented in `crates/a3-paa`. Confidence is given per item. "Survey" means the real-data test
`crates/a3-paa/tests/real_data.rs` over every texture of the install (build 2.22.0.154103, all
official and optional folders mounted, EBOs skipped).

## PAA/PAC container

All integers little-endian. `.pac` files have the same layout as `.paa` (**high**: all 15
shipped `.pac` files parse with the same reader).

```text
u16 type tag
TAGG*            while the next 4 bytes are "GGAT"
u16 palette_len  then palette_len * 3 bytes (B, G, R)
mipmap*          until width == 0 && height == 0
(u16 0)          two more zero bytes after the terminator in TexConvert output
```

### Type tags (**high**)

| tag | format | bytes |
|---|---|---|
| `0xFF01` | DXT1 (BC1) | 8 per 4x4 block |
| `0xFF02` | DXT2 (BC2, premultiplied) | 16 per block |
| `0xFF03` | DXT3 (BC2) | 16 per block |
| `0xFF04` | DXT4 (BC3, premultiplied) | 16 per block |
| `0xFF05` | DXT5 (BC3) | 16 per block |
| `0x4444` | ARGB4444 | `u16` `AAAA RRRR GGGG BBBB` |
| `0x1555` | ARGB1555 | `u16` `A RRRRR GGGGG BBBBB` |
| `0x8888` | ARGB8888 | bytes B, G, R, A |
| `0x8080` | AI88 (grey + alpha) | bytes I, A (verified visually on a `_gs` UI icon) |

Survey: DXT5 22,353, DXT1 20,306, AI88 117, ARGB4444 4, ARGB1555 2; no DXT2/3/4, no ARGB8888,
no untagged OFP-style palettised files.

### TAGGs

Each is `"GGAT"`, a four-character name stored reversed, `u32` length, payload. The engine's
reader is at RVA `0x10bce40` (see `file-formats-in-binary.md`).

| file bytes | name | payload | notes |
|---|---|---|---|
| `GGATCGVA` | AVGC | 4 bytes B, G, R, A | average colour; for `_nohq` it is the *unswizzled* average (`#8080f4`). **high** |
| `GGATCXAM` | MAXC | 4 bytes B, G, R, A | max colour; nearly always white. Absent in some old files. **high** |
| `GGATGALF` | FLAG | `u32` | 1 = interpolated alpha (blend), 2 = binary alpha (alpha test). **high** (matches `texHeaders.bin` `is_alpha` / `is_transparent` on all 38,312 entries) |
| `GGATZIWS` | SWIZ | 4 bytes, order A, R, G, B | channel swizzle, see below. **high** |
| `GGATCORP` | PROC | text | procedural source text (TexView). Not in shipped data. **medium** |
| `GGATSFFO` | OFFS | 16 `u32` | file offset of each mipmap header, zero-padded; all zero in some old `.pac`. Always matches the real offsets in the survey. **high** |

No other TAGG names occur in shipped data. TexConvert writes them in the order AVGC, MAXC, FLAG,
SWIZ, OFFS.

### Swizzle codes (**high** for the values seen, **medium** for 8/9)

Each byte says where that stored channel came from: 0 A, 1 R, 2 G, 3 B, 4 1-A, 5 1-R, 6 1-G,
7 1-B, 8 constant 1, 9 constant 0. Shipped values:

- `_nohq`: `05 04 02 03` (A = 1-R, R = 1-A): X of the normal goes to the DXT5 alpha block.
- `_sky`: `06 01 04 03` (A = 1-G, G = 1-A).
- `_as`: `08 08 02 08` (only green carries the ambient occlusion).

The engine's shaders expect the stored layout; `Swizzle::restore` undoes it for viewing
(`a3-tools paa topng --unswizzle`). Restored `_nohq` images look like ordinary tangent-space
normal maps.

### Mipmaps

```text
u16 width   top bit (0x8000) set: DXT data is LZO1X-compressed
u16 height
            (1234, 8765): marker, the real width and height follow (OFP-era); never in A3 data
u24 stored size
data[stored size]
```

- The engine's mip header reader (`0x10bb9c0`) masks the width with `0x7fff`, handles the
  1234/8765 marker by reading two more `u16`s, stops on (0, 0), and rejects sizes outside
  2..=4096 with "Extreme texture size (%dx%d)". **high** for the code path. Shipped raw-format
  textures do contain 1x1 levels; how the engine treats them is **unknown** (probably never
  reaches them).
- DXT data is LZO1X-compressed when the width's top bit is set, raw otherwise. TexConvert
  compresses the large levels only (survey: DXT1 57,476 LZO / 119,381 raw levels; DXT5 47,843 /
  127,701; e.g. a 2048² `_co` has LZO down to 256², raw from 128²). **high**
- Non-DXT data is BI LZSS with a *signed* 32-bit checksum (sum of the output bytes as `i8`)
  whenever the stored size differs from the raw size `w*h*bpp`; equal size means raw. Some LZSS
  levels are *larger* than raw (the compressor never falls back), so "smaller than raw" is the
  wrong test. **high** empirically (882 LZSS and 6 raw levels all decode and checksum), not
  confirmed in the executable.
- Raw DXT sizes are whole 4x4 blocks: `ceil(w/4) * ceil(h/4) * 8|16`.
- Mip chain length: halving until the *shorter* side reaches 4 (DXT) or 1 (other formats);
  e.g. 32x8 DXT ends at 16x4, 2:1 AI88 ends at 2x1. **high** (survey of non-square textures).
- The 24-bit size field caps one stored level at 16 MiB - 1, so a 4096² DXT5 top level must be
  LZO-compressed.

## texHeaders.bin

Per-PBO cache written by Binarize so the engine knows each texture's size, mipmap offsets and
average colour without opening the PAA. The string `texHeaders.bin` is in the executable.

```text
"0DHT"  u32 version = 1  u32 count  entry[count]

entry:
  u32 palette_count = 1     u32 palette_pointer = 0
  f32 average[4]            AVGC as R, G, B, A / 255
  u8  average_bgra[4]       always 0
  u8  max_bgra[4]           MAXC (white when absent)
  u32 clamp_flags = 0       u32 transparent_color = 0xFFFFFFFF
  u8  has_maxc  u8 is_alpha (FLAG&1)  u8 is_transparent (FLAG&2)  u8 is_alpha_non_opaque
  u32 mip_count             u32 pixel_format
  u8  little_endian = 1     u8 is_paa (0 for .pac)
  char path[]               NUL-terminated, relative to the PBO prefix
  u32 texture_type          see below
  u32 mip_count             (repeated)
  mip[mip_count]: u16 width  u16 height  u16 0  u8 pixel_format  u8 3  u32 offset_in_paa
  u32 paa_file_size
```

Pixel format enum (also in the executable's string table next to the DXT names): 0 P8, 1 AI88,
2 RGB565, 3 ARGB1555, 4 ARGB4444, 5 ARGB8888, 6 DXT1, 7 DXT2, 8 DXT3, 9 DXT4, 10 DXT5. **high**
for 1, 3, 4, 6, 10 (seen); the rest by position.

Survey: 219 files, 38,312 entries; every file writes back byte-identically, and every entry
matches its PAA in format, file size, MAXC presence and value, FLAG bits, average colour and the
full mipmap table (sizes and offsets). **high**

### Texture type (**high**)

`texture_type` is the engine's texture type enum. The engine derives it from the file name in
`0x10bb880` / `0x10bb490` (also used for the tag of procedural `color()`); `TextureType::
from_path` reproduces it and matches all 38,312 cached entries.

Rule: take the file name after the last `\`, find the last `.`, then the last `_` before it;
compare `_suffix.` case-sensitively (names are lower case in the engine; `a3-paa` lowercases).
A name without `.` never matches and gets 0.

| value | name | suffixes |
|---|---|---|
| 0 | Diffuse | everything else (`_co`, `_ca`, `_lca`, `_mca`, `_ads`, `_ti`, `_ti_co`, ...) and terrain segments `s_XXX_YYY_lco` |
| 1 | Diffuse linear | `_sky`, `_lco` (except names `s_` + 7 characters + `_lco`, i.e. the `_` at index 9) |
| 2 | Detail | `_detail`, `_cdt`, `_dt`, `_mco` |
| 3 | Normal map | `_no`, `_non`, `_nopx`, `_noex`, `_nohq`, `_novhq`, `_nofhq`, `_nof`, `_nofex`, `_ns`, `_nsex`, `_nshq`, `_normalmap` |
| 4 | Irradiance | procedural `irradiance`, `waterIrradiance`, `fresnel`, `fresnelGlass` |
| 5 | Random test | internal `TextureSourceRandomTest` (no procedural name reaches it) |
| 6 | Tree crown | procedural `treeCrown`, `treeCrownAmb` |
| 7 | Macro | `_mc` |
| 8 | Ambient shadow | `_as` |
| 9 | Specular | `_sm`, `_smdi` |
| 10 | Dither | procedural `dither` |
| 11 | Detail specular | `_dtsmdi` |
| 12 | Mask | `_mask` |
| 13 | Thermal | `_ca` directly preceded by `_ti` (`*_ti_ca`) |

Procedural `perlinNoise` and `point` report 2 (detail); `color()` see below. The values for
procedural sources come from vtable slot 8 of each `TextureSource*` class.

## Texture suffixes

`TextureKind::from_path` in `crates/a3-paa/src/kind.rs`. Suffix counts in the install: `_ca`
8,020, `_co` 7,537, `_nohq` 4,984, `_lco` 4,246, `_lca` 4,208, `_smdi` 2,820, `_no` 2,001, `_mc`
1,191, `_as` 1,173, `_mask` 831, `_ads` 621, `_dtsmdi` 464, `_adshq` 374, `_mca` 210, `_gs` 117,
`_dt` 101, `_sky` 75, `_nopx` 60, `_ti` 33 (the `_ti_ca` thermal textures count under `_ca`).
Terrain layers: `layers\s_XXX_YYY_lco` satellite segment, `m_XXX_YYY_lca` surface mask segment,
`n_XXX_YYY_no` normal segment.

`TextureKind` is a coarser, suffix-only view; use `TextureType::from_path` for the engine's
type. `_detail` and `_ns` are older spellings of `_dt` and `_no`.

Uncertain: what the `l` in `_lco`/`_lca` stands for; the channel use of `_ads`/`_adshq`;
`_non`.

## Procedural textures

Wherever a texture path is accepted, `#(format,width,height,mipmaps)function(args)` makes the
engine generate the texture. Implemented in `crates/a3-paa/src/procedural/`; `a3-tools paa
topng '#(...)...' out.png` renders one.

### Parsing (`TextureSourceProcFactory`, `0x10b0630` → `0x10c0f80`, `0x10b41b0`) **high**

- `#` then `(` at index 1. Format = text up to the first `,` (at most 63 characters), matched
  case-insensitively by FNV-1a hash: `ai`, `a`, `i` → AI88; `argb`, `rgb` → ARGB8888. `rgb` is
  *not* opaque-forced: it is the same as `argb`.
- Width, height, mipmaps: each must start with a digit (`strtol`), separated by `,` `,` `)`.
  All must be ≥ 1; width and height must be powers of two; `max(w, h) >= 1 << (mipmaps - 1)`.
- Function name: text up to the next `(` (≤ 63 characters), matched case-insensitively against
  the name table built in `0x0bd1a0`: 0 `Irradiance`, 1 `Color`, 2 `Dither`, 3 `PerlinNoise`,
  4 `WaterIrradiance`, 5 `FresnelGlass`, 6 `TreeCrown`, 7 `TreeCrownAmb`, 8 `Point`, 9 `Fresnel`,
  10 `R2T`, 11 `Text`, 12 `UI`, 13 `UIEx`, 14 `Extension`.
- Arguments: text up to the first `)` (up to the last `)` when it contains `"`). Numbers are
  `strtod` but each must start with a digit (`.5`, `-1` are rejected), followed by `,` or the
  end. A failed parse makes the texture fail to load ("Can not create source for procedural
  texture").

| function | arguments | stored as / defaults |
|---|---|---|
| `color` | `r,g,b,a[,tag]` | 4 numbers; anything after a 5th comma is the tag |
| `irradiance` | `power` | exactly 1 number |
| `dither` | `a,b` | 2 numbers, rounded to integers |
| `perlinNoise` | `xScale,yScale,min,max` | 4 numbers |
| `waterIrradiance` | `power` | 1 number |
| `fresnelGlass` | `[n]` | empty → 1.7; `n <= 0` → warning, 0.001 |
| `treeCrown`, `treeCrownAmb` | `density` | 1 number |
| `point` | anything | ignored |
| `fresnel` | `[n,k]` | empty → n 0.96977, k 0.0118; otherwise both required; `<= 0` → 0.001 |

### Levels **high**

Each level is generated independently at its own size (not downsampled). Generic sources
(`0x10bc020`): level *i* is `(w >> i, h >> i)` for *i* < mipmaps, stopping after the first level
with a side below 2. Exceptions fixed at creation: `color` is always one 1x1 level (the declared
size is ignored); `fresnel` and `fresnelGlass` are one `w` x 1 level; `dither` is a square of
`max(w, h)` with `floor(log2(size))` levels.

### Generators (vtable slot 14 of each class) **high** for structure and constants

All arithmetic is `f32` in this order; results are rounded half-to-even (`cvtss2si`) and
clamped to 0..255. In AI88, "value v in alpha" means bytes `I = 0, A = v` unless stated.
`step(n) = 1 / (n - 1)`.

- **color** (`0x10b8580`): ARGB8888: each channel clamped to 0..1, `round(c * 255)`.
  AI88: `I = round(g*149.685 + r*76.245 + b*29.07)` (Rec. 601 luma), `A = round(a * 255)`.
  The renderer may swap R and B at creation (a renderer flag at `0x1421d8498`, vtable +0xe40);
  that is a texture-layout detail, not a colour change.
  Texture type: from the tag via the suffix rule (`_<tag>.`, lowercased); without a tag the
  nearest of 7 reference colours by squared RGBA distance, if below 0.5 (table at
  `0x1420c0030`): (0.5,0.5,0.5,1) detail, (0.5,0.5,1,1) normal, (1,1,1,1) diffuse linear,
  (0,0,0,0) and (1,1,1,0) macro, (1,0,0,1) and (1,0,1,1) specular; otherwise diffuse.
- **fresnel(n,k)** (`0x10b88e0`), conductor reflectance in alpha, one row:
  `θ = acos(x·step(w))`, `s, c = sin θ, sin(θ + π/2)`, `t = tan θ`;
  `t0 = (n² − k²) − s²`, `A = sqrt(k²·n·4n + t0²)`, `a² = (A + t0)/2`, `2a = 2·sqrt(a²)`;
  `Rs = (A − 2a·c + c²) / (2a·c + A + c²)`;
  `Rp = Rs` at x = 0, else `Rs·(A − 2a·s·t + s²t²) / (2a·s·t + A + s²t²)`;
  value `round((Rs + Rp) · 127.5)`. Divisions use a guard: denominator below `FLT_MIN` gives
  ±`FLT_MAX` (or 1 for 0/0).
- **fresnelGlass(n)** (`0x10b8e80`), dielectric reflectance in alpha, one row:
  `θi = (float)(acos(x·step(w)) + 1e-6)`, `θt = asin(sin θi · 1.0002927 · (1/n))`;
  `r = (tan(θi−θt)/tan(θi+θt))² + (sin(θi−θt)/sin(θi+θt))²`; value `round(r / (r/2 + 1) · 255)`.
  1.0002927 is the refractive index of air.
- **irradiance(p)** (`0x10b9300`): `I(x) = round(x·step(w)·255)`, `A(y) = round((y·step(h))^p ·
  255)`; pixel is all zero where `I = 0`. ARGB: grey `I`, alpha `A`.
- **waterIrradiance(p)** (`0x10baa20`): `I(y) = round((y·step(h))^p · 255)`; alpha = air→water
  Fresnel of `cos θi = x·step(w)`: `cos θt = sqrt(1 − (1 − cos²θi)·0.5597403)`
  (`(1.0002927/1.337)²`), `rs = (cosθi·1.337 − cosθt·1.0002927)/(cosθi·1.337 + cosθt·1.0002927)`,
  `rp = (cosθi·1.0002927 − cosθt·1.337)/(cosθt·1.337 + cosθi·1.0002927)`,
  `A = round((rs² + rp²)·127.5)`.
- **perlinNoise(xs,ys,min,max)** (`0x10b9b90`, noise `0x38aca0`): Ken Perlin's improved noise
  (2002) with the reference permutation (copied at start-up from `0x1aaca60`), `f32`, `z = 0`,
  lattice cell `round_half_even(x − 0.5)`. Sample `((i + 0.5)/w · xs, (j + 0.5)/h · ys)`;
  value `round((max − min)·(noise + 1)·127.5 + min·255)` in all channels (AI88 `I = A`).
- **point** (`0x10b9d90`): `u = 2x·step(w) − 1`, `v = 2y·step(h) − 1` (computed as
  `((t − 0.5) + t) − 0.5`); alpha `round((1 − sqrt(u² + v²))·255)`, colour white (AI88 `I = 255`).
- **treeCrown(d)** (`0x10ba0b0`): `I(x) = round(exp(x·step(w)·ln d)·255)`,
  `A(y) = round(exp(y·step(h)·ln d)·255)`; all zero when `d <= 0`.
- **treeCrownAmb(d)** (`0x10ba710`, AI88 `0x10b0da0`): `u = x·step(w)`, `v = 2·step(h)·y − 1`,
  `r² = u² + v²`; value `exp((1 − r²)·ln d)` inside the unit circle, 1 outside; grey and alpha
  both; all zero when `d <= 0`.
- **dither(a,b)** (`0x10b88a0` → `0x10b3d80`), AI88 only ("Unsupported format" otherwise): an
  ordered-dither matrix built over block sizes 2, 4, 8, ...: each block's quadrants get
  (top-left, top-right, bottom-left, bottom-right) `a`, `(a−b)·2/4 + b`, `b − (b−a)/4`,
  `(a−b)·3/4 + b` added (C integer division), then `b = (b−a)/4`, `a = 0` for the next size.
  Cells are 16-bit wrapping; the final value is `min(cell + b/2, 255)` in intensity and alpha.
  `dither(0,255)` at 8x8 gives the classic Bayer matrix ×4 (first row 0 128 32 160 8 136 40 168).
- **R2T, Text, UI, UIEx, Extension**: rendered by the engine at run time; `a3-paa` parses them
  but cannot generate them.

The CRT maths functions (`acosf`, `sinf`, `tanf`, `asinf`, `powf`, `expf`, `logf`) are the
engine's own (MSVC); Rust's may differ in the last bit, so a generated byte can differ by one
from the engine. Tests check values against the formulas evaluated independently.

### Use in shipped data

Scan of `.rvmat`, `.p3d`, `.bin`, `.cpp`, `.hpp` files in the VFS (real-data test): 2,455
distinct strings, 239,030 uses: `color` 194,294, `fresnel` 43,512, `fresnelGlass` 930, `r2t`
287 (31 distinct), `waterIrradiance` 3, `perlinNoise` 2. All parse and generate except
`#(ai,64,64,1)fresnelGlass(0.9,0.9)` (2 uses), which the engine rejects too (one argument
only). Most common: `#(rgb,1,1,1)color(0.5,0.5,0.5,1,cdt)`, `#(argb,8,8,3)color(0,0,0,0,mc)`,
`#(argb,8,8,3)color(0.5,0.5,1,1,nohq)` (flat normal), `#(ai,64,64,1)fresnel(1.3,7)`.
`irradiance`, `treeCrown`, `treeCrownAmb`, `point` and `dither` do not occur in shipped data.

## Performance (release build, warm file cache, 16 threads)

Decompressing every level of all 42,782 textures (27 GiB) takes ~120 s; the first, cold-cache
run took ~530 s single-threaded, bound by disk reads.
