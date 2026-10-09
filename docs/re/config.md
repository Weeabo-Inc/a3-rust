# Config: rapified format, text syntax, merge and inheritance

Implemented in `crates/a3-config`. Sources: public community documentation of the `raP` format,
HEMTT's rapifier read as format documentation, and verification against every rapified file in
the 2.22 install (see "Verification"). No disassembly was done for this topic yet; behaviour
marked _assumed_ needs confirmation in the executable.

## Rapified format (`\0raP`) — confirmed on real data

All integers little-endian. `asciiz` = NUL-terminated bytes. `cint` = unsigned varint, 7 bits per
byte, low group first, high bit = continuation.

```text
header   "\0raP"  u32 0  u32 8  u32 enum_table_offset
body     asciiz base ("" = none), cint entry_count, entry * entry_count, u32 end_of_subtree
entry    u8 type:
           0 class     asciiz name, u32 body_offset
           1 value     u8 subtype, asciiz name, scalar
           2 array     asciiz name, array
           3 external  asciiz name                       (class X;)
           4 delete    asciiz name                       (delete X;)
           5 append    u32 flags (always 1 = +=), asciiz name, array
array    cint count, (u8 subtype, scalar) * count
scalar   subtype 0 asciiz string | 1 f32 | 2 i32 | 3 nested array (arrays only)
                 | 4 asciiz expression | 6 i64
enums    u32 count, (asciiz name, u32 value) * count
```

Layout used by BI's tools (we reproduce it byte-for-byte): the root body starts at 16. Each body
is followed by its `end_of_subtree` u32, then the bodies of its child classes in entry order, each
child immediately followed by its own descendants (depth-first). `end_of_subtree` is the offset
just past the last descendant body; for the root it equals `enum_table_offset`.

Observations:
- Strings are UTF-8 except a few Windows-1252 bytes (e.g. `0x97` em dash) in
  `dubbing_radio_f`, `dubbing_radio_f_exp` and `sounds_f_contact` config.bin. We decode invalid
  UTF-8 as Windows-1252 (re-written as UTF-8, so those files are not byte-identical).
- Five config.bin files (3den, editor_f, ui_f, ui_f_enoch, missions_f_oldman) declare a larger
  enum count than the table holds; the table just ends at EOF. The reader stops at EOF.
- Value subtype 4 (expression) and array subtype 4 never occur in shipped data.
- `delete` (1342), `+=` (16) and int64 (16) occur in shipped data.

## Text syntax (config.cpp after preprocessing)

- Statements: `class N: B {...};`, `class N;`, `delete N;`, `n = v;`, `n[] = {...};`,
  `n[] += {...};`, `enum { A, B = 5 };` (enum constants go to the enum table).
- Strings: `"..."` or `'...'`; the quote character doubled escapes it. No backslash escapes.
- Unquoted values run to `;` (or `,`/`}` in arrays, or end of line), trimmed. A number literal
  becomes a number, anything else a string (`x = some text;` -> `"some text"`).
- Numbers _(assumed rapifier rules)_: decimal integer -> int32 if it fits, else int64; `0x` hex ->
  int32 (u32 bit pattern); with `.` or exponent -> f32.
- Tolerated: missing `;` after `}` of a class and before `}`; `#` lines (preprocessor line markers).
- Floats print with Rust's shortest round-trip form and keep a `.0` (`2.0`), so they re-parse as
  floats.

## A config that cannot be loaded: a missing `#include` (_confirmed_, #348)

`a3\missions_f_oldman\missions\repro_objectsimulationloadgame.tanoa\description.ext` includes two
files that are nowhere in the install (`...\Systems\UI\Sleeping\RscTestControlTypes.inc` and
`...\RscRestUI.inc`; that folder holds `CreateRestUI.sqf`, `RestTimeControl.sqf` and
`RscDisplayOMRest.sqf`). Probe: `tools/oracle/probes/missing_include.py`, three one-mission servers
on `arma3server_x64.exe` 2.22.0.154103 (`control`, `include`, `syntax`).

- A missing include is a preprocessor error, engine **error 1**, and preprocessing stops at the
  first one: the second missing include of the same file is never reported.
- The **whole config fails**: nothing of it is installed, not even the entries before the failing
  directive. `missionConfigFile >> "Header"` is not a class, and `loadConfig` returns `configNull`
  for the shipped `description.ext`.
- The **mission still loads and starts**. It runs with an empty mission config.
- A config that fails to *parse* behaves the same (the `syntax` case), so a config failure of
  either kind is survivable at the mission level.

RPT, `include` case (timestamps cut; `12604` is that run's pid):

```text
Warning Message: Include file a3ro_probe\does_not_exist.inc not found.
 ➥ Context: Preprocessing file: mpmissions\a3ro_probe_include_12604.VR\description.ext at 10
Cannot include file \a3ro_probe\does_not_exist.inc
Warning Message: Preprocessor failed on file 'mpmissions\a3ro_probe_include_12604.VR\description.ext' - error 1 (source '\a3ro_probe\does_not_exist.inc', line 0).
Mission a3ro_probe_include_12604.VR: Missing 'description.ext::Header'
Starting mission:
 Mission file: a3ro_probe_include_12604
 Mission world: VR
 Mission directory: mpmissions\a3ro_probe_include_12604.VR\
```

The mission's `initServer.sqf` reads `missionConfigFile` after it has started:

```text
"A3RO header=false before=0 after=0"       nothing of the failed config is installed
"A3RO shipped_null=true shipped_class=false shipped_idd=0"
```

The `control` case is the same `description.ext` without the `#include`, and the same driver:

```text
"A3RO header=true before=11 after=22"      the driver does read missionConfigFile
```

So the difference is the config, not the reader. `loadConfig` of the shipped file, in every case:

```text
Warning Message: Preprocessor failed on file 'a3\Missions_F_Oldman\Missions\REPRO_objectSimulationLoadGame.Tanoa\description.ext' - error 1 (source '\a3\Missions_F_Oldman\Systems\UI\Sleeping\RscTestControlTypes.inc', line 0).
```

The `syntax` case (an unterminated class) logs
`Warning Message: File mpmissions\...\description.ext, line 12: /A3ROBroken/: Missing '}'`, leaves
the mission config empty the same way, and starts the mission.

Implemented as: `load_text_config` (`crates/a3-gamedata/src/scripts.rs`) fails, as it did — our
`ErrorKind::Include` is the engine's error 1, and we too stop at the first one — while
`load_mission` (`crates/a3-mission/src/load.rs`) now logs the failure and loads the mission with an
empty mission config instead of failing the load. Our message text differs from the engine's
(`cannot include \a3\...: file ... not found` for `Cannot include file \a3\...` plus
`Preprocessor failed on file '<path>' - error 1`); log-text parity is not attempted here.

## Merging addon configs (configFile)

Each `config.bin` in a PBO, including those in subfolders, is one addon config (1432 in 2.22).

Load order _(assumed)_: discovery order (mod folders in order, PBOs alphabetical) refined by a
stable topological sort on CfgPatches `requiredAddons`: among addons whose requirements are
loaded, the earliest-discovered goes next. Unknown requirements are ignored (reported); cycles are
broken at the earliest-discovered addon. On 2.22 vanilla + DLC data: 0 missing, 0 cycles.

Patching a class that already exists:
- Entries merge recursively; a value replaces the old value at its position; new entries append.
- The class's base becomes the patch's base. The engine logs
  `Updating base class Old->New, by <path>` when it changes, including to empty (`Old->`), so a
  patch without `: Base` drops inheritance. _(confirmed by RPT messages; mechanism assumed)_
  130 such updates happen when loading 2.22 (vanilla, DLC and CDLC folders in a3-vfs discovery
  order), mostly in UI (`Rsc*`) classes.
- `class X;` creates a placeholder only if X is absent; a later definition fills it in.
- `delete X;` removes X unless some class's base resolves to X; then the engine logs
  `Cannot delete class X, it is referenced somewhere (used as a base class probably)`.
- `x[] += {...}`: extends an existing own array in place; otherwise kept as an append and resolved
  at lookup as inherited value + items _(assumed: the engine may resolve at load time instead)_.

## Lookup and inheritance

- Names compare ASCII-case-insensitively; original case is kept for display.
- `cls >> name`: own entries, then the base chain. `class X;` placeholders are skipped (they stand
  for an inherited or later-defined class). An unresolved placeholder reads as missing
  _(assumed)_.
- Base resolution for `class C: B` defined inside class P: search P's own entries, then P's base
  chain, then P's enclosing class (and its base chain), up to the root; C itself is never its own
  base. This makes `class Turrets: Turrets { class MainTurret: MainTurret {} }` and
  `class NewTurret;` forward declarations work. Verified: all 12037 classes directly under
  CfgVehicles/CfgWeapons/CfgAmmo/CfgMagazines with a declared base resolve it.
- An inherited subclass keeps the access path (`configHierarchy`, `str`) while its own base is
  resolved where it is defined.

SQF accessor semantics as implemented:

| accessor | number entry | string entry | array | class/missing |
|---|---|---|---|---|
| `getNumber` | value | number literal, `true`=1, `false`=0, else 0 _(engine evaluates the string as an expression — TODO)_ | 0 | 0 |
| `getText` | formatted number _(assumed)_ | value | `""` | `""` |
| `getArray` | `[]` | `[]` | items (with `+=`) | `[]` |
| `isNumber`/`isText`/`isArray`/`isClass` | by stored type; numeric strings are text | | | |

## Verification (2.22.0.154103, `cargo test -p a3-config --release --test real_data`)

| kind | files | read | rap round-trip | byte-identical | text round-trip |
|---|---|---|---|---|---|
| config.bin | 1432 | 1432 | 1432 | 1424 | 1432 |
| rvmat | 38073 | 38073 | 38073 | 38073 | 38073 |
| mission.sqm | 237 | 237 | 237 | 237 | 237 |

The 8 non-identical config.bin are the Windows-1252 and short-enum-table files above. Merged
tree: 508 PBOs, 1432 addon configs, ~1.15M nodes, under 1 s to merge in release builds. PBOs are
enumerated with `a3-pbo` in `a3-vfs` discovery order (Dta, Addons, official DLC, optional folders).
