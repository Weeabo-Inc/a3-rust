# File formats: magic values and version checks in arma3_x64.exe

Where the engine recognises each file format, which magic it compares and which versions it
accepts. Addresses are RVAs of arma3_x64.exe 2.22.0.154103 (VA = RVA + 0x140000000); "site" is
the instruction that uses the magic, "function" the Ghidra function that contains it. Found by
scanning code for 32-bit immediates (`cmp/mov reg, 'magic'`) and string cross-references, then
reading the decompiled code. Use `python tools/re/a3re.py decompile <function>` to read more.

## Shared helper: versioned magic/version field

`0x326390` — `bool SerializeBinStream::Version(stream, int min, int max)` (name assigned by us).
When **loading** it reads an int32, stores it in the stream object (`stream+0x20`) and returns
false if it is outside `[min, max]`. When **saving** it writes `max`. Every binary format below
calls it twice: once with `(magic, magic)` for the signature, once with the version range. On
failure the loader sets the stream error code (`stream+0x10`: 1 = bad magic, 3 = bad version).

So for each format: the version the game **writes** is the maximum, and it **reads** anything in
the range.

## Summary

| Format | Magic in file (bytes) | int32 LE compared | Versions read / written | Function (RVA) | Confidence |
|---|---|---|---|---|---|
| PBO header entry | `Vers` as mime field → bytes `73 72 65 56` ("sreV") | 0x56657273 | — | read `0x31b7b0`, write `0x310650` | high |
| PBO compressed entry | packing-method field, bytes `73 72 70 43` ("srpC", the int32 reads 'Cprs') | 0x43707273 | — | open entry `0x31be40`, write `0x310fc0` | high |
| Encrypted PBO | file extension `.ebo` | — | — | ext. string 0x1a7867c, xrefs in `0x31ef20`, `0x132eb0`, `0x498010`, `0xb5d660` | medium |
| Rapified config (config.bin, rapified .sqm/.ext) | `00 72 61 50` ("\0raP") | 0x50617200 | — | `0x2b48f0`, `0x2bb200`, `0x2bcaf0`, `0x2bcd30`, `0x2be180` | high |
| P3D ODOL | `ODOL` (`4f 44 4f 4c`) | 0x4c4f444f | **28–75** (0x1c–0x4b), writes 75 | `0x15da1d0` | high |
| P3D MLOD | `MLOD` string 0x1ce2b34 | — | — | `0x15bf6e0` | medium |
| WRP (current) | `OPRW` (`4f 50 52 57`) | 0x5752504f | **3, or 15–25** (`"Bad version %d in landscape %s"`) | `0x1078fd0` | high |
| WRP (legacy) | `8WVR`, `4WVR` | 0x52565738 / 0x52565734 | — | read `0x106ded0`, write 8WVR `0x1076ac0` | high |
| RTM (legacy text tags) | `RTM_0101`, `RTM_MDAT` strings 0x1ca7060 / 0x1ca7070 | — | — | `0x124f8e0`, `0x124e6f0` | medium |
| RTM binarised | `BMTR` (`42 4d 54 52`) | 0x52544d42 | **2–5**, writes 5 | `0x12557a0` | high |
| PAA/PAC tags | `GGAT` + 4-char tag (`TAGG` reversed) | 0x54414747 | — | `0x10bce40` | high |
| WSS | `WSS0` (`57 53 53 30`) | 0x30535357 | — | `0x3991b0` | high |
| bisign | file name `<pbo>.<key>.bisign` (`%s.*.bisign`, `bo.%s.bisign`) | — | — | `0x267b20` ("Corrupted bisign file: %s") | medium |
| bikey | files in `keys\` | — | — | `RegisterBikeyFromBisign` (RTTI lambda name) | medium |
| mission.sqm / description.ext / config.bin | names at 0x1a78708 / 0x1aa5970 / 0x1a77e68 | — | — | — | — |

## Details

### PBO (`0x31b7b0`)

Reads the first entry header as five int32 after the empty name: `mime, original_size,
reserved, timestamp, data_size` (stored at +0x8..+0x18 of the entry object). It returns "this is
a header-extension entry" only if `mime == 0x56657273 && timestamp == 0 && data_size == 0`;
the property block (`prefix`, `version`, `product`, ... as zero-terminated key/value pairs, empty
key ends it) follows. Entry streams with `mime == 0x43707273` ("Cprs") are opened through a
decompressing stream sized by `original_size` (`0x31be40`). The writer `0x310650` emits the
"sreV" entry; `0x310fc0` writes compressed entries. All 869 PBO/EBO files in the install start
with the sreV entry (2 small ones have mime 0, see `data-inventory.md`).

### Rapified config (`0x2b48f0`)

The loader first asks the file source (vtable +0x68) whether the text form applies; otherwise it
checks the 4-byte signature `\0raP` (0x50617200) via the helper. No version range is read at
this point (the rapified header's following fields are read by the class-tree parser).

### ODOL (`0x15da1d0`)

`Version(ODOL, ODOL)` then `Version(0x1c, 0x4b)`. Versions ≥ 75 (0x4b) have an extra 8-byte
field read right after the version and stored at `shape+0x68`; versions in 59..74 (`0x3a < v <
0x4b`) and < 59 take different branches for the following data. Phase 1 must support the
range the shipped files use (inventory pending).

### WRP / OPRW (`0x1078fd0`)

`Version(OPRW, OPRW)`, then an int32 version `v`; accepted if `v == 3` or `15 <= v <= 25`,
otherwise logs `"Bad version %d in landscape %s"`. Branches at `v > 22`, `v > 24`, `v < 25`,
`v < 12` select optional blocks. Legacy `8WVR`/`4WVR` terrains are read by `0x106ded0`.

### RTM (`0x12557a0`)

Binarised animation: `Version(BMTR, BMTR)` then `Version(2, 5)`; next byte stored at
`anim+0x10d`. Error codes: 4 = bad magic, 3 = bad version, 6 = already loaded. Text-tagged
`RTM_0101`/`RTM_MDAT` animations are handled in `0x124f8e0` / `0x124e6f0`.

### PAA (`0x10bce40`)

Loops while the next int32 is `GGAT` (0x54414747, file bytes `GGAT`). Each tag is
`GGAT, name (int32), length (int32), payload`. Recognised names (int32 → file bytes):

| int32 | File bytes | Meaning |
|---|---|---|
| 0x41564743 | `CGVA` | average colour (ARGB8888) |
| 0x464c4147 | `GALF` | flags (bit 0 → alpha/transparent flag at `tex+0x39`) |
| 0x4d415843 | `CXAM` | max colour, stored as three floats `/255` |
| 0x4f464653 | `SFFO` | mip offsets |

Default max colour is `0xff802020` or `0x80c02020` depending on the texture type argument
(types 1, 4, 5).

### WSS (`0x3991b0`)

Reads 8 bytes; the first int32 must be `WSS0` (0x30535357), else the stream is closed and the
file treated as not-WSS (falls back to another decoder).

## Open questions (follow-up issue)

- bisign/bikey binary layout and the accepted bisign versions (v2/v3) — loader `0x267b20`,
  RSA via Botan.
- `.ebo` decryption scheme.
- ODOL versions actually present in the install; LZO/LZSS usage per format.
- `mission.sqm` text vs rapified detection and its version field.
