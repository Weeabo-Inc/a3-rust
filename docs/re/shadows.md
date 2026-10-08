# Sun shadows

What the shipped data says about RV's shadow settings, as used by `a3-render::shadow`.

Sources: merged config of build 2.22 (`a3-tools config dump`) and strings in `arma3_x64.exe`. No
decompilation was used.

## `CfgVideoOptions >> ShadowQuality`

| preset (`text`) | `shadowType` | `textureSize` | `shaderQuality` | `cascadeLayers` |
| --------------- | ------------ | ------------- | --------------- | --------------- |
| Disabled | 0 | 0 | 0 | 0 |
| VeryLow (Low) | 2 | 1024 | 0 | 2 |
| Low (Standard) | 2 | 1024 | 1 | 3 |
| Normal (High) | 2 | 1024 | 2 | 4 |
| High (Very High) | 2 | 2048 | 2 | 4 |
| VeryHigh (Ultra) | 2 | 2048 | 3 | 4 |
| Extreme | 2 | 4096 | 3 | 8 |

Our reading: `textureSize` is the edge of one cascade's shadow map, `cascadeLayers` the cascade
count, `shaderQuality` the filtering quality (PCF taps), `shadowType` 2 = shadow maps (0 = off;
1 probably the legacy stencil shadow volumes, which models still carry as Shadow Volume LODs)
_(medium confidence on all but the texture size)_.

## Shadow distance

The exe contains `shadowZDistance` (and `shadowsZ`), the profile entry behind the "Shadow
distance" video option. The default value and the range of that option are not yet confirmed;
we default to 150 m.

## Our implementation

- `ShadowSettings` defaults to the "High" preset: 4 cascades of 2048 texels.
- Cascade splits blend logarithmic and uniform (λ = 0.75); each cascade is fitted with a
  rotation-invariant bounding sphere of its frustum slice and snapped to whole texels in world
  space (no shimmering as the camera moves).
- Receivers use 3×3 hardware PCF, a normal offset in texels and a constant + slope depth bias
  in the caster pass; shadows fade over the last 10 % of the shadow distance.

## Open questions

- RV's split scheme and whether it blends between cascades.
- What `shaderQuality` changes exactly (tap count, kernel size).
- Default and range of `shadowZDistance`; how it interacts with view distance and
  `CfgWorlds >> ... >> shadow*` entries.
