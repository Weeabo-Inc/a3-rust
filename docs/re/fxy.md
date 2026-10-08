# FXY bitmap fonts

Implemented in `crates/a3-fonts`. Source: the font loader of `arma3_x64.exe` (build
2.22.0.154103; VAs in the Ghidra project): `0x14103d460` opens `%s.fxy` (log `Cannot load font
%s`), `0x14103d8d0` reads version 0x102, `0x14103dbe0` reads the older layouts. Every `.fxy` in
the install parses with the layouts below. Confidence: **high** unless marked.

## Survey

481 `.fxy` files (`a3\uifonts_f\data\fonts\...`, `a3\ui_f_enoch\data\cfgfontfamilies\...`,
`core\data\fonts\`): 300 `BIFo` 0x102, 178 `BIFo` 0x101 (Etelka, ...), 3 unversioned
(`core\data\fonts\LucidaConsoleB*`). 698,864 glyphs, 104,776 kerning pairs, 2,486 page
textures, none missing. 22 glyphs (in 13 fonts) reach one or a few pixels past the edge of their
page texture; the data is like that.

A font of one size is `<name>.fxy` plus pages `<name>-01.paa`, `<name>-02.paa`, ... (page
number, two digits, from 1).

All integers little-endian.

## Header

```
char[4]  "BIFo"
i32      version     0x101 or 0x102
```

No `BIFo`: unversioned layout from byte 0. Version above 0x102: the engine logs `%s: Too new
font (version %x)` and still reads it as 0x102 (`a3-fonts` rejects it). Version below 0x102: the
record layout below.

## Version 0x102: page blocks until end of file

```
u16  page                 texture page number
i32  height               per-page metric, e.g. 17 for caveat10   (meaning: medium)
i32  ascent               per-page metric, e.g. 13 for caveat10   (meaning: low)
i32  kerning_count
{ u16 first; u16 second; i32 amount } [kerning_count]
i32  glyph_count
{ u16 code; u16 x; u16 y; u16 w; u16 h; i32 offset_x; i32 offset_y; i32 advance } [glyph_count]
```

The engine keeps `w`, `h`, the two offsets and `advance` as single bytes (the offsets signed).
It tracks the largest `w`, `h` and `advance` as font-wide maxima. The order of the two offsets
(x first) is _medium_: blank glyphs (`w = h = 0`, such as space) carry `-10, -10`.

## Version 0x101 and unversioned: records until end of file

```
u16  code - 0x20          stored minus 0x20
u16  page
u16  x, y, w, h
u16  advance              0x101 only; unversioned: advance = w
```

12-byte records (unversioned) or 14-byte records (0x101); offsets are 0.

## Loading rules

- A glyph is registered when `w` or `h` is non-zero or the code is below 0x401 (empty glyphs of
  higher codes are dropped). `a3-fonts` keeps every record.
- After loading, the engine looks up glyph `o` (0x6f) and space (0x20) for font metrics
  (`0x14103c080`); not modelled yet.
- Codes are UTF-16 code units.

## Open questions

- Exact meaning of the 0x102 page metrics and the two offsets (needs the text renderer).
- How config `CfgFontFamilies` picks a size (`fonts[]` lists) and scales it.
