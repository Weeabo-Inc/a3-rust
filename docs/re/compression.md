# Compression in RV4 data

Implemented in `crates/a3-compress`. Findings come from decoding the 2.22 install
(`A3_ROOT`), not from the decompiler. Confidence is noted per item.

## BI LZSS

- Flag byte, bits LSB first: `1` = literal byte, `0` = back-reference `b0 b1`.
- Reference: `distance = b0 | (b1 & 0xF0) << 4`, counted back from the current output position;
  `length = (b1 & 0x0F) + 3`. Positions before the output start read as spaces (`0x20`).
  **Confirmed**: every non-DXT PAA mipmap in the install decodes with this rule and its
  checksum matches. The classic Okumura rule (absolute ring-buffer position) does not apply.
- Decoding stops at the expected output length. A `u32` LE checksum may follow: wrapping sum of
  the output bytes.
  - PAA (ARGB4444 `0x4444`, ARGB1555 `0x1555`, AI88 `0x8080`): **signed** (`i8`) sum. Confirmed
    on all 115 such mipmaps in `Addons/`.
  - PBO `Cprs` entries, rapified files: not verified. The shipped PBOs contain no `Cprs` entries.

## LZO1X

- Standard LZO1X stream with the `11 00 00` end marker. The executable links "LZO Professional".
- PAA DXT1/DXT5 mipmaps with the width field's top bit set: LZO1X. **Confirmed** on ~18 000
  mipmaps in `Addons/`; each block ends exactly at the stored mipmap data length.
- ODOL v73 (all shipped P3D files): `u32` element count, then, when the data is non-empty, a flag
  byte: `0` raw bytes follow, `2` an LZO1X block follows. The flag decides, not the size: in a
  sample of 1,313 arrays, 78 under 1024 bytes were LZO and 20 of 1024 bytes or more were raw.
  **Verified**: `a3-p3d` decodes every LOD of all 10,472 shipped models with this rule, each LOD
  ending exactly at its table offset. Details: `p3d-odol.md`.
- OPRW v25 (all shipped WRP files): the elevation grid (`MapSizeX * MapSizeY` floats) is an
  inline LZO1X block with no flag byte. **Observed** on Stratis.

## LZ4

No LZ4 data found in shipped files, and no LZ4 strings in the executable.

## Header fields seen while probing

- ODOL v73 and OPRW v25 both store `0x0001A392` (107410, Arma 3's Steam app ID) right after the
  version.
- Stratis OPRW: layer grid 256 x 256, terrain grid 2048 x 2048, layer cell size 32.0.
