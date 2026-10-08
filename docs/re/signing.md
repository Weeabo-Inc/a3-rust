# Bikey / bisign signatures

Implemented in `crates/a3-signing`. Layouts and hash rules follow community format notes (BI
community wiki, HEMTT documentation) and are **confirmed** by verifying every signed vanilla PBO
(build 2.22.0.154103): 507 of 507 signatures verify. Engine loader for bisigns: `0x267b20`
(`Corrupted bisign file: %s`); RSA there is done by Botan (not traced).

## Survey

- 14 `.bikey` files (`Keys\` and DLC `keys\` folders): `a3`, `a3c`, `csla`, `ef`, `gm`, `rf`,
  `spe`, `vn`, `ws`. All 1024-bit RSA, exponent 65537.
- 866 `.bisign` files, all version 3, named `<archive>.<authority>.bisign` next to the archive.
  485 PBOs are signed by `a3`, 22 (the Contact campaign folder) by `a3c`; the rest sign
  creator-DLC EBOs (not verifiable without decrypting them).
- Unsigned: `Dta\splashwindow.pbo` and the two `MPMissions\*.tem_chernarusd.pbo`.

## .bikey

```
asciiz  authority                 e.g. "a3"
u32     blob_length               148 for 1024-bit keys
PUBLICKEYBLOB (Microsoft CryptoAPI):
  u8    type          0x06
  u8    version       0x02
  u16   reserved      0
  u32   algorithm     0x00002400 (CALG_RSA_SIGN)
  char  magic[4]      "RSA1"
  u32   bits          1024
  u32   exponent      65537
  u8    modulus[bits/8]           little-endian
```

`.biprivatekey` is the same with type `0x07`, magic `RSA2`, and after the modulus the CryptoAPI
`PRIVATEKEYBLOB` fields: p, q, d mod (p-1), d mod (q-1), q^-1 mod p (each bits/16 bytes) and d
(bits/8 bytes), all little-endian. _Medium_ (standard CryptoAPI layout; no private key ships).

## .bisign

```
asciiz  authority
u32     blob_length, PUBLICKEYBLOB      the signer's public key, as in the .bikey
u32     length (128), u8 sig1[length]   little-endian
u32     version                         2 or 3
u32     length, u8 sig2[length]
u32     length, u8 sig3[length]
```

Each signature `s` decodes as `m = s^e mod n`; written big-endian over `bits/8` bytes, `m` is a
PKCS#1 v1.5 block with a SHA-1 `DigestInfo`:
`00 01 FF..FF 00 30 21 30 09 06 05 2b 0e 03 02 1a 05 00 04 14 <20-byte digest>`.

## The three hashes

For a PBO with header entries `E` (in header order) and the `prefix` property `P`:

- **names hash** = SHA-1 over the lower-case names of the entries whose data size is non-zero,
  concatenated without separators.
- **prefix** = `P` followed by `\` (unless it already ends with one); empty when the PBO has no
  prefix.
- **hash1** (sig1) = the PBO's SHA-1 trailer (SHA-1 of every byte before the trailer's 0 byte).
- **hash2** (sig2) = SHA-1(hash1 + names hash + prefix).
- **hash3** (sig3) = SHA-1(content hash + names hash + prefix), where the content hash is SHA-1
  over the stored data of the non-empty entries the version selects, in header order:
  - version 2: every extension **except** `paa jpg p3d tga rvmat lip ogg wss png rtm pac fxy
    wrp`;
  - version 3: **only** `sqf inc bikb ext fsm sqm hpp cfg sqs h sqfc` (note: not `cpp` or `bin`).
  When no entry is selected the content hash is SHA-1 of the ASCII text `nothing` (v2) or
  `gnihton` (v3).

Confidence: **high** for v3 (all vanilla signatures), **medium** for v2 (same structure, no v2
signature ships). Every vanilla PBO header is already sorted by lower-case name, so whether the
engine hashes in header order or sorted order is not visible in vanilla data (_unknown_); 161
zero-size entries exist in vanilla PBOs and skipping them is required for the hashes to match.
For compressed (`Cprs`) entries `a3-signing` hashes the stored bytes _(unverified: no vanilla
entry is compressed)_.

## Open questions

- Hash order for PBOs whose header is not sorted (third-party packers).
- How the engine checks EBO signatures (the entries are encrypted).
