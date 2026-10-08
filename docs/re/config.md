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
