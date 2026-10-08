# Input key codes and user actions

How RV encodes keybindings in `CfgDefaultKeysPresets` (config) and in the player profile, as
needed by `a3-input` (`Binding::from_rv_code`).

Sources: string scan of the shipped `ui_f.pbo` (build 2.22): the `defineDIKCodes.inc` header,
the `KEY_XINPUT`/`KEY_XBOX_*` defines, and expression strings stored in the rapified
`CfgDefaultKeysPresets` (some array entries are kept as unevaluated strings such as
`"0x00010000 + 1"` or `"256+0x25"`). No decompilation was used.

## Code layout

A binding is a 32-bit integer.

| Bits / value            | Meaning                                                     | Confidence |
| ----------------------- | ----------------------------------------------------------- | ---------- |
| `0x00` device, 0..0xFF  | Keyboard key, DirectInput scancode (`DIK_*`)                | high       |
| `+ 0x100` (256)         | Double tap of that key, shown as `2xKey`                    | high (`compassToggle = "256+0x25"`, `zoomInToggle = "256+0x4E"`) |
| `+ 0x200` / `0x400` / `0x800` | Legacy Ctrl / Shift / Alt modifier (`INPUT_CTRL_OFFSET` 512, `INPUT_SHIFT_OFFSET` 1024, `INPUT_ALT_OFFSET` 2048) | high for the values, medium that either side's key counts |
| `0x0001_0000 + n`       | Mouse button n (0 left, 1 right, 2 middle, ...)             | high (`holdBreath = 0x00010000 + 1` is RMB) |
| `0x0001_0000 + 128 + n` | Mouse button n with a modifier flag; we read it as double click | low (`optics = 0x00010000 + 128 + 1`, `closeContext = 0x00010000 + 128 + 1`) |
| `0x0002_0000 + n`       | DirectInput joystick button n                               | medium (joystick scheme classes) |
| `0x0004_0000 + n`       | Joystick POV hat direction (0 up, 2 right, 4 down, 6 left; odd = diagonals) | medium (`keyLookDown = 0x00040000 + 4`) |
| `0x0005_0000 + n`       | XInput gamepad (`KEY_XINPUT`), index table below            | high (defines) |
| `0x0008_0000 + n`       | Unknown device; bound to `lookLeftCont`, `leanLeft`, `zoomContIn` (probably TrackIR / head tracking axes) | low |
| `0x0010_0000 + n`       | Mouse axis: 0 left, 1 right, 2 up, 3 down, 4 wheel up, 5 wheel down | high (`aimHeadUp = +2`, `prevAction = +4`, `nextAction = +5`) |
| bits 24..32             | DIK of a combo modifier key, e.g. `LCtrl+X = 0x1D << 24 \| 0x2D` | medium: seen in profile keybindings, not in the preset strings |

In `CfgDefaultKeysPresets`, a combo is written as a nested two-element array
`{modifier, key}`, e.g. `minimapToggle[] = {{0x9D, 0x32}}` (RCtrl+M), and
`deployWeaponAuto[] = {{0x1D, 0x2E}}` (LCtrl+C) in one of the shipped presets. Decode with
`Binding::from_rv_combo`.

### XInput indices (`KEY_XBOX_*`, `KEY_XINPUT + n`)

| n | input | n | input | n | input |
| - | ----- | - | ----- | - | ----- |
| 0 | A | 8 | Start | 16 | LeftThumbXRight |
| 1 | B | 9 | Back | 17 | LeftThumbYUp |
| 2 | X | 10 | LeftBumper | 18 | RightThumbXRight |
| 3 | Y | 11 | RightBumper | 19 | RightThumbYUp |
| 4 | Up | 12 | LeftTrigger | 20 | LeftThumbXLeft |
| 5 | Down | 13 | RightTrigger | 21 | LeftThumbYDown |
| 6 | Left | 14 | LeftThumb | 22 | RightThumbXLeft |
| 7 | Right | 15 | RightThumb | 23 | RightThumbYDown |

## Action names

Action names are the `CfgDefaultKeysPresets >> <preset> >> Mappings` entry names and the
argument of `inputAction`, compared case-insensitively. Examples seen in `ui_f.pbo`:
`moveForward`, `moveBack`, `moveLeft`, `moveRight`, `turnLeft`, `turnRight`, `defaultAction`,
`reloadMagazine`, `ingamePause`, `optics`, `opticsTemp`, `holdBreath`, `zoomIn`, `zoomTemp`,
`compass`, `compassToggle`, `lookAround`, `lookAroundToggle`, `aimUp`..`aimRight`,
`aimHeadUp`..`aimHeadRight`, `prevAction`, `nextAction`, `buldMoveForward` (Buldozer).
The free camera (3DEN, splendid camera) uses `cameraMoveForward`, `cameraMoveBackward`,
`cameraMoveLeft`, `cameraMoveRight`, `cameraMoveUp`, `cameraMoveDown`, `cameraMoveTurbo1`,
`cameraMoveTurbo2`, `cameraLookUp`, `cameraLookDown`, `cameraLookLeft`, `cameraLookRight`
(strings in `3den.pbo` and `functions_f.pbo`).

Joystick schemes use `key`-prefixed names (`keyLookDown`, `keyAutoHover`); profile keybindings
use the same `key` prefix, e.g. `keyMoveForward[] = {17, 200}` _(medium: from memory of
`.Arma3Profile` files, not yet checked against a profile)_.

## Open questions

- Double-tap time window and hold threshold (we use 0.3 s and per-binding hold times).
- Whether a plain binding is suppressed while a combo on the same key is held (we suppress it).
- Meaning of `+128` on mouse buttons and of device `0x08`.
- How `inputAction` scales mouse-axis values.
