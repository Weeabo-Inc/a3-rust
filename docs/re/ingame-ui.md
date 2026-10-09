# In-game UI: the HUD (`InGameUI`)

What the engine draws over the 3D view during play and how it fills it: the unit info
displays (`RscInGameUI >> RscUnitInfo*`), the weapon info displays (`weaponInfoType`), the
stance indicator. Implemented in `crates/a3-ingame-ui`. Sources: `arma3_x64.exe`
2.22.0.154103 (Ghidra project, see `TOOLING.md`), the shipped config and
`\a3\ui_f\hpp\defineResincl.inc` (IDC names), `\a3\functions_f\gui\fn_displayColorGet.sqf`.

Engine classes (RTTI): `InGameUI` (vtable `0x141c7ed20`), `DisplayUnitInfo`,
`DisplayWeaponInfo`, `DisplayStanceInfo`, `DisplayStaminaInfo`, `DisplayHint`,
`DisplayTaskHint`, `CStanceIndicator`.

## Frame order (`FUN_1411e4a90`, InGameUI::DrawHUD)

1. `FUN_140aa9c10`: the two weapon text colours (below).
2. If unit info is on (`InGameUI+9`, the `showHUD` element) and the player is not in a
   camera/cutscene state: `FUN_1411897f0` (weapon info displays), then `FUN_1411859e0` (unit
   info displays), then `FUN_141185850` (stance indicator) when the person is not inside a
   vehicle, then the stamina bar.
3. Group/command bar, cursor (`InGameUI` vtable `+0x70`, crosshair size `0.08`/`0.1067` ×
   cursor scale), tactical ping, action menu, hints.

Each display is drawn right after it is updated, with its own alpha
(`Display::DrawHUD(vehicle, alpha)`, vtable `+0x688` slot 0). a3-ui models this as
`Display::alpha`.

## Unit info displays (`FUN_1411859e0`)

- **Which displays:** `FUN_14118d1d0(turret, vehicleType)` returns a list of `RscInGameUI`
  class names: the turret's `unitInfoType` (turret `+0x768`, or `unitInfoTypeRTD` `+0x780`
  with the advanced flight model) when it has one, else the vehicle type's (`+0x1690`, RTD
  `+0x16a8`). `unitInfoType` is read by `FUN_140fc5730`: a string is a one-element list, an
  array is taken as is. On foot the "vehicle" is the soldier: `RscUnitInfoSoldier`.
- One display per entry (`InGameUI+0xae8` array of `{name, display, time}`). When an entry's
  class name differs from the one built, the display is rebuilt from
  `RscInGameUI >> name` (`FUN_1411c9120`) and the time stamp reset.
- The display class is `DisplayUnitInfo` (`FUN_141190270`, 0xab8 bytes). Its control
  factory (`FUN_1411b79a0`, vtable slot 16) keeps a pointer per IDC; a second control with the
  same IDC overwrites the first. **Only these IDCs are engine-driven**; every other control
  keeps its config.

### Display loading: `controls[]` arrays

`RscInGameUI` displays list their controls as `controls[] = {"A", "B"};`. The loader
(`FUN_141448350`) accepts both forms for `controls`, `objects` and `controlsBackground`: a
class is enumerated (own entries), an array names classes that are looked up **in the display
class through inheritance**. Classes defined in the display but not named are not created
(`RscUnitInfoSoldier` inherits ~20 classes from `RscUnitInfo` and shows only its 3).
_(high)_

### IDC → field of `DisplayUnitInfo` (`FUN_1411b79a0`)

| IDC | name (`defineResincl.inc`) | field | control |
|---|---|---|---|
| 101 | IGUI_TIME | 0x6b8 | special (`FUN_14118fc60`) |
| 102 | IGUI_DATE | 0x6c0 | text: `STR_DATE_FORMAT` |
| 103 | IGUI_NAME | 0x6c8 | unit name |
| 104 | IGUI_UNIT | 0x6d0 | `STR_UI_UNIT` / `STR_UI_UNIT_CIVIL` |
| 106 | IGUI_VALUE_EXP | 0x6e0 | progress |
| 107 | IGUI_COMBAT_MODE | 0x6f0 | text |
| 109 | IGUI_VALUE_HEALTH | 0x780 | progress (health colours) |
| 111 | IGUI_HITZONES | 0x798 | `CHitZones` |
| 112 | (vehicle toggles) | 0x7a0 | `CVehicleToggles` |
| 113 | IGUI_VALUE_FUEL | 0x788 | progress |
| 114–117 | IGUI_CARGO_* | 0x7a8–0x7c0 | `STR_UI_CARGO_*` |
| 118 | IGUI_WEAPON | 0x7c8 | weapon name |
| 119 | IGUI_AMMO | 0x7d0 | `STR_UI_AMMO` / `STR_UI_AMMO_EMPTY` |
| 120 | IGUI_VEHICLE | 0x6d8 | vehicle `displayName` (+0x1430) |
| 121/122 | IGUI_SPEED/ALT | 0x6f8/0x700 | `STR_UI_SPEED`, `STR_UI_ALT` |
| 123 | IGUI_FORMATION | 0x6e8 | |
| 124 | IGUI_BG | 0x830 | background (records its w/h) |
| 125–127 | COMMANDER/DRIVER/GUNNER | 0x840–0x850 | |
| 128–131 | ALT_WANTED/SPEED_WANTED/POSITION/OPTIC | 0x740–0x758 | |
| 148 | IGUI_HEADING | 0x718 | `STR_UI_HEADING` |
| 149 | IGUI_WEAPON_MODE | 0x7e8 | mode `displayName` |
| 150 | IGUI_WEAPON_GUNNER | 0x7d8 | |
| 151 | COUNTER_MEASURES_AMMO | 0x7f8 | throwable count `"x%d"` |
| 152 | COUNTER_MEASURES_MODE | 0x800 | throwable `displayNameShort` |
| 153 | IGUI_RADARRANGE | 0x720 | `STR_UI_RADARRANGE` |
| 154 | IGUI_VALUE_RELOAD | 0x790 | progress |
| 155 | IGUI_WEAPON_AMMO | 0x7f0 | magazine `displayNameShort` |
| 184 | IGUI_AMMOCOUNT | 0x808 | `"%d"` |
| 185 | IGUI_MAGCOUNT | 0x810 | `"\| %d"` |
| 186 | IGUI_DEPTH | 0x728 | `STR_UI_DEPTH` |
| 187 | WEAPON_MODE_TEXTURE | 0x7e0 | picture |
| 189/190/191 | GPS_PLAYER/SPEED2/ALT2 | 0x730/0x708/0x710 | |
| 192 | PILOT_OPTIC_ZOOM | 0x760 | |
| 205 | IGUI_THROTTLE | 0x738 | `%.0f%%`-style ×100 |
| 380/381/382 | SPEED_/SPEED_VERTICAL_/ALT_FREEFALL | 0x770/0x778/0x768 | |
| 383–393, 401–413, 501–550 | horizon, groups, injuries, heli gauges | 0x858–0x920 | |
| 26006/26106/26206 | — | 0x818/0x820/0x828 | `"%d"` |

`DisplayWeaponInfo` (`FUN_1411ba050`) has its own map for IDCs 151–207 (`IDC_IGUI_WEAPON_*`:
distance, vision/FLIR/FOV mode, compass, heading, javelin, artillery, lased distance, ...);
the same IDC means something else there. Updated by `FUN_1411897f0` (not implemented yet).

### What the soldier panel shows (weapon part)

The selected muzzle and its magazine come from `FUN_140fb4c70(weapons, slot, ...)`:

- magazine = the weapons' pending magazine (`weapons+0x40`) else the slot's loaded one;
- `ammo` = its rounds; when it belongs to a shared ammo pool (`magazine+0x88 >= 0`) the
  rounds of every pooled magazine are summed and `mags` is forced to 0;
- `mags` = other carried magazines of the muzzle's types with rounds left;
- `capacity` = `count` of its type; `total` = summed `count`;
- `empty` = no rounds; reload state: `cycle = magazine+0x70` (fraction of the mode's
  `reloadTime` left before the next shot), `reload = magazine+0x78` (magazine reload seconds
  left), `reloadDuration = magazine+0x7c`. While no magazine reload runs: state = `cycle`,
  progress = `cycle × reloadTime`, total = `reloadTime`; while it runs: state 1, progress =
  `reloadTime × cycle + reload`, total = `reloadDuration + reloadTime`.
- Without a loaded magazine the first carried compatible magazine stands in (its name and
  `count`), with `ammo` 0.

Then, for the display:

- No valid muzzle: hide 118, 119, 184, 185, 26006, 26106, 26206, 187, 149, 155.
- `capacity < 1` (a weapon without magazines): 118 shows the weapon name in the side's ready
  colour; 119, 184, 185 get empty text; 149 the mode name (hidden when nothing is left and
  the name is empty).
- Otherwise, with the text colour `C` (below):
  - 154: shown with value `progress` in `[0, max(range, total)]` when `state > 0.001`, not
    empty and `total > 0.5` (slow weapons, magazine reloads); else hidden at 0.
  - 118: weapon `displayName` (weapon type `+0x160`), colour `C`; hidden when
    `ammo + mags < 1` and the name is empty.
  - 119: `STR_UI_AMMO` (`"%d | %d"`, ammo, mags) or `STR_UI_AMMO_EMPTY` (`"%d"`) without mags.
  - 184: `"%d"` ammo (a laser designator magazine, ammo simulation 0x11, shows
    `STR_ACTION_LASER_ON/OFF` instead). 185: `"| %d"` mags, empty text without mags.
  - 26006 `"0"`, 26106 `total`, 26206 `capacity`.
  - 187: texture of `CfgInGameUI >> CfgWeaponModeTextures >> <mode textureType>` (mode
    `+0x20`), `default` when missing; the lookup is a case-sensitive hash map.
  - 149: mode `displayName` (mode `+0x18`); hidden when nothing is left and it is empty.
  - 155: magazine `displayNameShort` (magazine type `+0x48`); hidden when empty.
  - All shown in colour `C`; 184/185/155 use `colorPrepare` while `weapons+0x40` is set and
    185 then reads `"<<   >>"`, 155 `STR_A3_OPTIONS_STANDARD` when the name is empty
    _(medium: `weapons+0x40` looks like a pending magazine change; not implemented)_.
- Throwables (`FUN_141180f20`, person with a `Throw` muzzle selected at `weapons+0x38`):
  151 `"x%d"` with (carried magazines of the type + 1 if one is in hand), 152 its
  `displayNameShort`, both in the throwable colour; hidden without one. Vehicles use
  `FUN_141180490` (countermeasures).
- 150 (gunner's weapon) only for a vehicle gunner other than the player.
- 380–382 shown only while the unit falls freely (vtable `+0x1898`): `STR_UI_SPEED_FREEFALL`
  (`"SPD %.0fkmph"`, rounded speed), `STR_UI_SPEED_VERTICAL_FREEFALL`, `STR_UI_ALT_FREEFALL`.
- 124 (background) shows while 118, 150 or 121 does. With the type flag `+0xab2` the old
  layout widths of 118/119/184/185 are re-fitted to their texts (not used by A3 displays).

### Colours (`FUN_140aa9c10`, `FUN_140aa9b60`, `FUN_140aa9db0`)

`InGameUI::Init` (`FUN_140aa9ec0`) reads `RscInGameUI >> colorReady` (0x1618),
`colorReadyWest` (0x161c), `colorReadyEast` (0x1620), `colorReadyIndependent` (0x1624),
`colorReadyCivilian` (0x1628), `colorPrepare` (0x162c), `colorUnload` (0x1630), each packed
to 8-bit ARGB. Every frame:

- dead player (or no unit): both colours `colorUnload`;
- else for the selected weapon (`+0x6c`) and the selected throwable (`+0x70`): with a loaded
  magazine and no magazine reload running, `cycle <= 0` → the side's ready colour
  (east 0, west 1, resistance 2, civilian 3, other → `colorReady`), `cycle <= 0.2` →
  `colorPrepare`, else `colorUnload`; no magazine or reloading → `colorUnload`.

The configured colours are profile expressions (`profilenamespace getvariable
['IGUI_TEXT_RGB_R', 0]`). The profile variables come from `CfgUIColors`: the script
`[true] call BIS_fnc_displayColorGet` writes every `<TAG>_<VAR>_R/G/B/A` that is missing or
off its preset; the default IGUI preset `PresetA3` has `TEXT_RGB {0.95, 0.95, 0.95, 1}`,
`BCG_RGB {0.2, 0.2, 0.2, 0.4}`, `WARNING_RGB {0.8, 0.5, 0, 1}`, `ERROR_RGB {0.8, 0, 0, 1}`.
The engine does not read `CfgUIColors` itself. a3-ingame-ui runs the script before reading
the colours.

### Fade

`FUN_140a0c990(range, t)`: 1 for `0 <= t < start`, `(end - t) / (end - start)` until `end`,
else 0, with `start/end = CfgInGameUI >> PlayerInfo >> dimmStartTime/dimmEndTime` (5, 10).
For the unit info `t` is 0 (always 1) unless the vehicle type sets flag `+0x15b2 & 1`, then
the seconds since the display last changed _(medium: which types set it)_. With the
difficulty option `weaponInfo` = 0 the unit info is not drawn for a soldier on foot.

## Stance indicator (`FUN_141185850`, `CStanceIndicator`)

- `RscInGameUI >> RscStanceInfo` (IDD 303) in a `DisplayStanceInfo`, created on first use
  (`FUN_1411a4c90`). Its IDC 188 control is a `CStanceIndicator` (`FUN_140aa3ea0`).
- Textures: `CfgInGameUI >> CfgStanceIndicatorTextures >> {Normal, Rested, CanDeploy,
  RestedCanDeploy, Deployed} >> texture{Prone,Crouch,Stand}[Adjust{Up,Down,Left,Right}]`,
  an array `[state][stance][adjust]` (`FUN_140aa4220`); stance index 0 (undefined) has no
  texture.
- Each draw (`FUN_140aa4ff0`) picks `[state][stance][adjust]` of the player: stance from his
  current Move's action map `stance` (prone 1, crouch 2, stand 3), adjust from his stance
  adjustment (none = 4), state from weapon resting/deployment.
- Alpha: difficulty `stanceIndicator` 0 → not drawn, 1 → fade with `t` = seconds since the
  last stance change (`InGameUI+0xb48`), 2 → 1. Only drawn on foot.

## Hints (`DisplayHint`, `InGameUI::ShowHint`)

- `InGameUI` builds `RscInGameUI >> RscHint` (IDD 301) into a `DisplayHint` in its
  constructor (`InGameUI+0xb50`). Controls: 101 background, 102 structured text.
- `hint` / `hintSilent` (`FUN_14055a3c0` / `FUN_14055ac40`) call `InGameUI` vtable `+0x220`
  (string, `FUN_1411d1820`) or `+0x218` (structured text, `FUN_1411d1610`) with `sound` 1 / 0.
  A string longer than 4096 bytes is cut to 4096.
- `DisplayHint::SetHint` (`FUN_1411cf100`): resets the text control's attributes to its config
  ones, sets the text, measures the text height (structured text `FUN_141486420`) and sets
  `102.h = height`, `101.h = (101.h - 102.h) + height` (the background keeps its margin).
- Timing: `InGameUI+0xb6c` (seconds left) = `CfgInGameUI >> Hint >> dimmEndTime` (35); every
  simulation step subtracts the frame time (`FUN_1411d1bc0`). Draw (`FUN_1411e66a0`): skipped
  when the text is empty or the time is negative; alpha 1 while more than
  `dimmEndTime - dimmStartTime` (5 s) is left, else `left / (end - start)`.
- Position: before drawing, both controls move vertically so the background's top is at
  `InGameUI+0xb70` (`FUN_1411d1170`): each frame `CfgInGameUI >> PlayerInfo >> top`
  (`0.177 + SafeZoneY`, `InGameUI+0x108c`), or just below an old-style unit info background
  (IDC 124) when one is shown. So the config `y` of `RscHint` does not decide where it shows.
- Sound: `CfgInGameUI >> Hint >> sound` (`InGameUI+0xb78`, volume `+0xb80`, frequency
  `+0xb84`) for `hint` with non-empty text.
- Structured text controls take their base size from `size` and their default alignment from
  `class Attributes >> align` (a3-ui).

## Chat area (`ChatList`, global `DAT_142221660`)

- Constructed at start (`FUN_141394bc0`): border 0.008, visible, all channels shown, scroll
  `-1`, default colours (system channel 0x10 `{1, 0.1, 0.1, 1}`, ...). Loaded from
  `RscChatListDefault` at the main menu and `RscChatListMission` by `DisplayMission`
  (`FUN_141398090`): `x`, `y`, `w`, `h` (row height), `rows` (integer), `font`, `size`,
  `colorBackground`, `color<Channel>`, `color<Channel>PlayerBackground`,
  `color<Channel>PlayerText` for Global, Side, Command, Group, Vehicle, Direct (slots 0–5),
  `colorSystemChannel` (0x10) and `colorBattlEyeChannel` (0x11) only when present; custom
  radio channels (6–15) and slots 0x1a–0x41 copy the global channel unless a custom channel
  exists; `colorMessage`, `colorMessageProtocol`, `shadow`, `shadowPlayer`, `shadowColor`.
- Messages (0x48 bytes, newest first): channel, sender name, text, real and game time, player
  flag (`+0x40`), forced-visible flag (`+0x41`), type (`+0x44`: 0 chat, 1 protocol).
  `systemChat` adds channel 0x10, no sender, protocol type _(medium: flag order)_.
- Draw (`FUN_141398e40`), bottom row first at `y + (rows - 1) * h`:
  - text = protocol ? text : `"` + text + `"`; prefix = `sender + ": "` when there is a sender;
    the prefix takes at most 70 % of the inner width `w - 2 * border`;
  - the text is wrapped to the rest by `FUN_1413974b0`: characters are measured one by one; on
    overflow the line breaks after its last whitespace (or before the character) and the
    width count restarts at zero;
  - the prefix box is drawn at the message's top visible row, then the lines from the last up,
    each a box `[x + prefixW, y, 2 * border + textW, h]` in `colorBackground` with the text at
    `x + border + prefixW`, vertically centred (`(h - size) / 2`); when rows run out the top
    line gets a `...` prefix;
  - colours: prefix in the channel colour on `colorBackground` (player messages: player text
    on player background); lines in `colorMessage` / `colorMessageProtocol`; a drop shadow
    offset `(0.075 h, 0.1 h)` in `shadowColor` when `shadow` = 1 (prefix: unless a player
    message with `shadowPlayer` != 1);
  - age fade (not while scrolled): 25–30 s alpha `(30 - age) * 0.2`, gone after 30 s;
  - the bottom row is also used by the voice indicator (who speaks) — not implemented.

## Difficulty options

`CfgDifficultyPresets >> defaultPreset` (`Regular`) `>> Options`: `weaponInfo` and
`stanceIndicator`: 0 never, 1 fade, 2 always (`Regular`: 2 and 2; `Veteran`: 1 and 1).
Read through the difficulty object `DAT_14225df58` (`FUN_14045e440` weaponInfo,
`FUN_14045e3c0` stanceIndicator) _(medium: the mapping of the two accessors)_.

## Strings

Engine string ids are globals (`DAT_1422xxxxx`) filled at start from a table of
`{id*, name*, length}` records in `.rdata`; `.work/ui-re/strids.tsv` lists them (produced by a
scan of that table). Ids used here: `STR_UI_AMMO`, `STR_UI_AMMO_EMPTY`,
`STR_UI_SPEED_FREEFALL`, `STR_UI_SPEED_VERTICAL_FREEFALL`, `STR_UI_ALT_FREEFALL`,
`STR_UI_SPEED`, `STR_UI_ALT`, `STR_UI_HEADING`, `STR_UI_DEPTH`, `STR_UI_RADARRANGE`,
`STR_UI_CARGO_*`, `STR_ACTION_LASER_ON/OFF`, `STR_A3_OPTIONS_STANDARD`. Literal formats:
`"%d"` (0x141a78f60), `"| %d"` (0x141c7c6b0), `"x%d"` (0x141c7c6ac).

## Open points

- `DisplayWeaponInfo` (optics overlays, `weaponInfoType`) — `FUN_1411897f0`.
- Injuries 401–413 (`FUN_1411811b0`), vehicle readouts, helicopter gauges.
- The initial throwable choice of a new unit (a3-ingame-ui takes the first loaded `Throw`
  muzzle).
- The unit info fade flag `+0x15b2` and the `weapons+0x40` state.
