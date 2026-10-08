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

### Texture type (**high** for the seen values, names **medium**)

`texture_type` follows the file-name suffix. Values seen, with the suffixes carrying them:

| value | name | suffixes |
|---|---|---|
| 0 | Diffuse | `_co`, `_ca`, `_lco`/`_lca` terrain layers, `_ads`, `_adshq`, `_mca`, `_ti`, `_ti_co`, no suffix |
| 1 | Diffuse linear | `_sky`, `sky_*_lco` |
| 2 | Detail | `_dt`, `_cdt`, `_mco`, `_detail` |
| 3 | Normal map | `_nohq`, `_no`, `_nopx`, `_nofhq`, `_non`, `_ns` |
| 7 | Macro | `_mc` |
| 8 | Ambient shadow | `_as` |
| 9 | Specular | `_smdi`, `_sm` |
| 11 | Detail specular | `_dtsmdi` |
| 12 | Mask | `_mask` |
| 13 | Thermal | `_ti_ca` only (541 entries) |

4, 5, 6 and 10 are unused by files; the gaps line up with the procedural texture sources in the
RTTI (`TextureSourceIrradiance`, `TextureSourceRandomTest`, `TextureSourceTreeCrown`,
`TextureSourceDither`), hence the names Irradiance, RandomTest, TreeCrown, Dither. **medium**

## Texture suffixes

`TextureKind::from_path` in `crates/a3-paa/src/kind.rs`. Suffix counts in the install: `_ca`
8,020, `_co` 7,537, `_nohq` 4,984, `_lco` 4,246, `_lca` 4,208, `_smdi` 2,820, `_no` 2,001, `_mc`
1,191, `_as` 1,173, `_mask` 831, `_ads` 621, `_dtsmdi` 464, `_adshq` 374, `_mca` 210, `_gs` 117,
`_dt` 101, `_sky` 75, `_nopx` 60, `_ti` 33 (the `_ti_ca` thermal textures count under `_ca`).
Terrain layers: `layers\s_XXX_YYY_lco` satellite segment, `m_XXX_YYY_lca` surface mask segment,
`n_XXX_YYY_no` normal segment.

`TextureKind::texture_type` reproduces the cached type for every entry except the 9 sky/sea
`*_lco` files (type 1 while terrain `_lco` segments are type 0); Binarize evidently decides
those by more than the suffix. `_detail` and `_ns` are older spellings of `_dt` and `_no`.

Uncertain: what the `l` in `_lco`/`_lca` stands for; the channel use of `_ads`/`_adshq`;
`_non`.

## Procedural textures

Wherever a texture path is accepted, `#(format,width,height,mipmaps)function(args)` makes the
engine generate the texture. Formats accepted by the executable's parser: `ai`, `argb`, `rgb`
(strings `AI`, `ARGB`, `RGB`); the wiki also lists `a` and `i` (**low**). Generator names in the
executable: `Color`, `Irradiance`, `Dither`, `PerlinNoise`, `WaterIrradiance`, `FresnelGlass`,
`TreeCrown`, `TreeCrownAmb`, `Fresnel`, plus `R2T` (render-to-texture), `UI`, `UIEx`,
`Extension`; RTTI also has `TextureSourcePoint`, `TextureSourceRandomTest`, `TextureSourceText`,
`TextureSourceVideo`, `TextureSourceWebBrowser`. Names are matched case-insensitively (shipped
data uses `fresnelGlass` and `fresnelglass`).

Occurrences in shipped PBOs (raw string scan, 239,959 hits):

- `color(r,g,b,a[,type])`, 1,497 distinct strings. The optional fifth argument is a texture
  suffix, unquoted, either case (`co`, `ca`, `dt`, `cdt`, `mc`, `as`, `nohq`, `smdi`,
  `dtsmdi`); it makes the result behave as that texture type. Most common:
  `#(rgb,1,1,1)color(0.5,0.5,0.5,1,cdt)`, `#(argb,8,8,3)color(0,0,0,0,mc)`,
  `#(argb,8,8,3)color(0.5,0.5,1,1,nohq)` (flat normal), `#(argb,8,8,3)color(1,1,1,1,as)`.
- `fresnel(n,k)`, 980 distinct, always `ai` (e.g. `#(ai,64,64,1)fresnel(1.3,7)`): a Fresnel
  reflectance lookup for rvmat environment stages, from refractive index and extinction
  coefficient.
- `fresnelGlass(n)` / `fresnelGlass()`: glass variant.
- `perlinNoise(xScale,yScale,min,max)`, e.g. `#(ai,512,512,9)perlinnoise(256,256,0.8,1)`.
- `waterIrradiance(n)`: `#(ai,16,64,1)waterIrradiance(16)`.
- `irradiance`, `treeCrown`, `treeCrownAmb`, `dither`: none in shipped data.

`a3-paa` parses every form and generates `color` (ARGB8888 for `rgb`/`argb`, AI88 for `ai`,
intensity taken from the red argument, alpha forced to 1 for `rgb`). The other generators are
follow-up work; their maths needs the executable.

## Performance (release build, warm file cache, 16 threads)

Decompressing every level of all 42,782 textures (27 GiB) takes ~120 s; the first, cold-cache
run took ~530 s single-threaded, bound by disk reads.
