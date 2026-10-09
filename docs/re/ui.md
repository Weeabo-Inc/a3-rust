# UI: displays, controls, coordinates

How the engine builds and lays out config-driven UI. Implemented in `crates/a3-ui`. Sources:
`arma3_x64.exe` 2.22.0.154103 (Ghidra project, see `TOOLING.md`), the BI wiki pages "SafeZone",
"Arma 3: Pixel Grid System", "Arma 3: GUI Coordinates", "getResolution", and the shipped config.

## Coordinate system

UI positions are fractions of a centred 4:3 **viewport**. `(0, 0)` is its top-left corner,
`(1, 1)` its bottom-right, and `[0.5, 0.5]` is always the screen centre.

- **Viewport height:** the height of the largest 4:3 area that fits the screen, times the
  interface size `uiScale`. The width is that height × 4/3.
- **Interface sizes** (`getResolution select 5`): Very Small 0.47, Small 0.55, Normal 0.7,
  Large 0.85, Very Large 1.0.
  - The global holding the current value is `DAT_14225de90`. `FUN_1406ecc90` initialises it to
    0.85, then overrides it from the profile / `IGUIForcedScale`.
  - a3-ui defaults to Normal.
- **Safe zone:** the whole screen in viewport units:
  - `safeZoneW = screenW / viewportW`
  - `safeZoneH = screenH / viewportH`
  - `safeZoneX = -(safeZoneW - 1) / 2`
  - `safeZoneY = -(safeZoneH - 1) / 2`

  This reproduces the wiki's 1080p / 16:9 / Normal values (`-0.452381, -0.214286, 1.90476,
  1.42857`). Single monitor: `safeZoneXAbs = safeZoneX` and `safeZoneWAbs = safeZoneW`.
  (Confidence: high for 16:9; the narrow-screen case, where the 4:3 area is limited by the
  width, is medium.)
- **`pixelW` / `pixelH`** (handlers `0x1408a9f10` / `0x1408a9ea0`): `1 / n`, where `n` comes from
  the 2D viewport object (`DAT_1421d8498`, functions `0x141034230` and `0x141031b00`). With the
  definitions above, `n` is the viewport size in pixels. Without a viewport they return 0.
- **Pixel grid** (`FUN_1410341b0`, writes globals `0x14225dea0/a4/a8`):
  - `pixelGridBase = screenHeight / uiScaleMaxGrids`
  - `pixelGridNoUIScale = round(base / uiScaleFactor) * uiScaleFactor`
  - `pixelGrid = round(base * uiScale / uiScaleFactor) * uiScaleFactor`

  The config values in 2.22 are `uiScaleMaxGrids = 60` and `uiScaleFactor = 4`, not the wiki's
  64. `round` is the FPU default (to nearest, ties to even): 1080p gives base 18, NoUIScale 16
  and pixelGrid 12 at Normal. The screen height is from the display device (vtable `+0xc78`)
  _(medium)_.
- **`getResolution`:**
  `[screenW, screenH, viewportW, viewportH, aspect, uiScale, fovTop, fovLeft, tripleHead,
  textureQuality]`.

## Building a display (`FUN_141448350`)

The loader reads:
- `closeOnMissionEnd`, `enableSimulation` and `enableDisplay`;
- the display handlers `onLoad`, `onUnload`, `onChildDestroyed`, `onChildCreated`, `onKeyDown`,
  `onKeyUp`, `onChar`, `onMouseButtonDown/Up`, `onMouseMoving`, `onMouseHolding`,
  `onMouseZChanged` and `onDraw`;
- then the control lists `controls`, `objects` and `controlsBackground`.

For each list, it enumerates the list class's **own** entries (`GetEntryCount` / `GetEntry`,
vtable `+0x218` / `+0x210`) and resolves each entry by name through inheritance. `objects` holds
3D object controls: shipped `RscDisplayMainMap` has `Compass` (idc 102, type 80) and `Notepad`
(idc 104, type 82) there; they are found by `displayCtrl` but are not 2D-drawn. A control
missing `type` or `idc` logs `Warning: no type entry inside class %s/controls/%s` (or `no idc
entry`). A failed control logs `Error loading control %s`.

Because only own entries are enumerated, derived control lists repeat every child: shipped config
writes `class Controls: Controls { class Overlay: Overlay {}; ... }`. a3-ui does the same
_(medium: the per-control loader was not traced)_.

**Drawing order:** `controlsBackground`, then `objects` (3D, not drawn yet), then `controls`. A
controls group draws its children (`class Controls`, relative positions, clipped to the group)
after itself.

## Values

- Numbers may be config numbers or strings holding SQF expressions (`x = "safeZoneX + 0.1";`).
- Colour arrays may mix numbers and expressions:
  `{"(profileNamespace getVariable ['GUI_BCG_RGB_R', 0.13])", ...}`.
- `style` may be an expression too: `"0x02 + 0xC0"`.
- Text starting with `$STR_` is localized.

a3-ui evaluates these through the SQF VM when building a display.

## Fonts (`CfgFontFamilies`, FXY)

- A family lists one font per size (`fonts[]`), each optionally an array of fallback fonts.
- **Glyph placement:** in BIFo 0x102 fonts, `offset_x` is the left bearing from the pen and
  `offset_y` is the image top relative to the line top. Example, `Roboto-Condensed16`: line
  `height` 28, `ascent` 22, `A` at offset 7 with height 15, which ends on the baseline at 22.
  The font's number is not its line height.
- **Size selection:** text uses the size whose line height is closest to `sizeEx × viewportH`,
  scaled to fit _(uncertain, #155)_.
- **Glyph pages are white with coverage in alpha:** a shipped page (`caveat10-01.paa`, DXT5,
  256x128) has 2137 texels with alpha > 200 and every one of them is near-white (min channel
  255). A glyph quad is therefore the text colour multiplied by the page and the coverage is the
  page alpha — which is what `crates/a3-render-ui`'s shader does.

## Rendering (`crates/a3-render-ui`)

`UiRenderer` implements `RenderFeature`: `prepare` resolves the draw list's textures and turns
the quads into one vertex buffer (six vertices per quad, TL/TR/BL/BL/TR/BR, `[x, y, u, v]` and
sRGB colour); `draw` runs in `Phase::Ui`, after the post pass, with no depth attachment and
`LoadOp::Load`.

- **Batching:** consecutive quads join one draw call while their texture slot and clip match, in
  list order (painter's order). Clip rectangles (control groups) become `set_scissor_rect` per
  batch, floored/ceiled to whole pixels and clamped to the viewport.
- **Textures:** the cache resolves the list's normalized paths. A PAA keeps its block
  compression when the adapter supports it (DXT1/2/3/4/5 → BC1/2/3) and its whole mip chain, and
  is decoded to RGBA8 when it does not. `#(...)` strings go through `a3_paa::Procedural`. Slot 0
  is a 1x1 white texture, the source of untextured (solid colour) quads; a path that does not
  load drops its quads and is remembered so it is not retried. All views are sRGB, sampled with
  a clamped linear sampler.
- **Text:** glyph quads arrive with `Uv::Texels` into their `-NN.paa` page; texel UVs are
  normalized with the texture's size. A font page is just another texture; the glyph geometry
  comes from a3-ui's layout, so `sizeEx`, alignment, shadow and structured text are layout
  concerns (#53, #155).
- **Colour space (deviation):** the engine draws into an 8-bit LDR backbuffer and blends in it;
  we draw after the post pass on an sRGB output target, so the shader decodes the authored
  sRGB colour to linear once and the hardware re-encodes on store: opaque pixels are byte-exact
  (verified: an untextured `[1, 0, 0]` quad reads back `255, 0, 0, 255`). Blending a translucent
  UI element instead happens in linear space, which is the physically correct blend and
  therefore *brighter* than the engine's gamma-blind one: 50% white over black gives 188/255
  where the engine gives 128/255. Output alpha is straight, from the draw list.
- **Real data:** all 2106 PAAs under `a3\ui_f` decode, block-compressed and as RGBA8 (0
  failures), and a real display texture and real FXY text draw over the scene in the text
  colour (tests gated on `A3_ROOT`).
