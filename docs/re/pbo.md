# PBO archive format

Implemented in `crates/a3-pbo`. Confidence: **high** for everything below marked _verified_ (checked
against every PBO of game build 2.22.0.154103); the rest is from community format notes.

## Layout

All integers are little-endian `u32`. Strings are NUL-terminated byte strings.

```
[properties record]   optional, first record only
[entry record] * N
[terminator record]   empty name; all five fields 0 in shipped files
[entry data] * N      in header order, no padding or alignment
0x00                  one zero byte
[SHA-1]               20 bytes
```

Record: `name\0`, then `method`, `original_size`, `reserved`, `timestamp`, `data_size`.

- **Properties record**: empty name, `method = 0x56657273` (bytes `sreV`, "Vers"), other fields 0.
  Followed by `key\0value\0` pairs up to an empty key (`\0`). _Verified._
- **method**: `0` stored; `0x43707273` (bytes `srpC`, "Cprs") LZSS-compressed, `original_size` is
  the unpacked size; `0x456e636f` (bytes `ocnE`, "Enco") encrypted.
- **Cprs data**: BI LZSS stream followed by a `u32` LE additive checksum of the unpacked bytes
  (format in `docs/re/compression.md`). Whether the bytes are summed signed or unsigned is
  _unverified_: no shipped PBO has a `Cprs` entry. `a3-pbo` writes the signed sum, as PAA uses,
  and accepts either when reading (issue #59).
- **reserved**: 0 in every shipped entry. _Verified._
- **timestamp**: seconds since the Unix epoch.
- **Data offset** of an entry = header length + sum of `data_size` of all earlier entries. The header
  has no offset field. _Verified._
- **SHA-1** covers every byte before the trailing zero byte (header + data), not the zero byte
  itself. _Verified on all 510 PBOs._
- An empty name ends the header. A second empty-name `Vers` record is not expected and also ends it.

## Properties seen

| key       | in how many archives (of 869 PBO + EBO) |
|-----------|-----------------------------------------|
| `prefix`  | 866 (all but the two MPMissions PBOs)   |
| `version` | 865                                     |
| `product` | 570 (`Arma 3`)                          |
| `Mikero`  | 71 (third-party packer version, EBOs)   |

## Survey of the install (build 2.22.0.154103)

- 510 `.pbo` files, 44.4 GB, 504,774 entries. Every entry is **stored** (method 0); no `Cprs` or
  `Enco` entries in any shipped PBO. No duplicate names (case-insensitive), no non-ASCII names, no
  forward slashes, no leading backslashes, no `original_size` on stored entries.
- Every PBO ends with exactly the 21-byte trailer after the data, and every SHA-1 matches.
- 867 distinct prefixes across 869 archives; no prefix is shared. The two MPMissions PBOs
  (`*.tem_chernarusd.pbo`) have no `prefix`.
- 359 `.ebo` files, 96.1 GB, in the creator-DLC folders `CSLA/`, `GM/`, `RF/`, `WS/`, `vn/`.

## EBO (encrypted PBO)

Creator DLC ship `.ebo` files. The properties record is in clear text (`prefix`, `version`,
`Mikero` packer version), but the bytes after it, from the first entry record onward, are
encrypted: names and fields read as random data. `a3-pbo` identifies EBOs by extension, reports
`Error::Encrypted` from `Pbo::open`, and reads their properties with `read_properties`.
How the game decrypts them is unknown _(needs reverse engineering)_.
