# Game messages (user-message layer)

What travels inside the transport (`net-transport.md`) once a client is connected
(`net-handshake.md`). Arma 3 2.22.0.154103; RVAs of `arma3_x64.exe` (VA = RVA + 0x140000000).

Reference code: `tools/re/a3net.py` (`msg_encrypt`, `msg_decrypt`, `msg_key_from_token`,
`crc16_ccitt`, varints). Cross-checks against the original code under emulation:
`tools/re/verify_net_emu.py` (12/12 pass). Full format table: `docs/re/net-message-formats.tsv`,
produced by `tools/re/net_formats.py`, which runs the engine's own registration code under
Unicorn.

**Confidence:** the crypto and primitive encodings marked *verified* were executed in the
emulator and match the Python code byte for byte; framing and envelope are read from the
decoder/encoder (high); message *meanings* are mostly unknown yet (names are stripped from the
retail build — `"Type_%d"` at 0xbc0d20 is all the engine itself prints).

## Pipeline

```
user message bytes (one guaranteed message may span several datagrams, see fragmentation)
  └ DContext layer: AES-128-CBC + CRC16 + padding/junk          ← per player, both directions
     └ u32 time, varint type, items...                         ← message envelope
        └ items encoded per NetworkMessageFormat (491 formats)
```

Server receive: `0xbbc3b0` → `DContext::process` (`vtable+0x30` of the per-player context,
`0x7b4790`) → envelope → `TransferMsg` (`0xc757d0`) → per-item `0xc760c0` → dispatch
(`NetworkServer::OnMessage`, `0xca32c0`, too large for the decompiler; key-exchange part in
`0xcad580`). Client receive mirrors this (`0xb89560`/`0xb89640`).

## DContext layer (message encryption) — verified

Classes (names are deliberate decoys): `RunningSimulation` (vtable 0x1b41b98) /
`SoundComponentEx` (0x1ba9358) on one side, `ComponentValidation` (0x1b41c10, base
`ModelVerification`) on the other; key objects `SecondaryFiremode` (0x1a7ac68);
`SEncryptionContext::Update` (`0x7b3fc0`) switches keys.

### Key and IV

A key is defined by a *token* (byte string). `msg_key_from_token(token)`:
`c = CRC32(token) & 0xFF` (standard CRC-32; only its low byte is used), then
- `K[i]` (`0x7b4c40`): `v = (c*0x16 - i*0x1D - 0x21) & 0xFF; repeat 16×j: v += (i*-0x69 + 0x1D)*j + 2c + i`
- `IV[i]` (`0x7b4d00`): sum of 16 xor terms of `c` and `i` (see code)
- `G[i]` (`0x272ed0`): `((i*i*7 + 0x4B)*i - 0x20)*i`
- AES-128 key = `K xor G`, IV = `IV` (`0x272ed0` stores IV at ctx+0, K at +0x10, G at +0x20; the
  cipher `0x270b80`/`0x26c570` uses `K^G` as the AES key).

Default context (every player starts with it): token = 16 zero bytes (`0x471a0` →
`0x7b42d0`), giving

- AES key `65 12 87 ec 61 c6 fb a0 fd fa cf 04 99 3e 83 08`
- IV `b3 2a 11 a8 2f e6 ed 44 2b e2 c9 00 87 3e 25 7c`

Because only one CRC byte enters the derivation, there are only 256 possible keys.

### Framing (encoder `0x7b4940`/`0x7b4fd0`, decoder `0x7b4790`)

```
plain   = payload ‖ crc16_le(payload) ‖ check(1)
padded  = plain ‖ p bytes (p = (16 - len(plain) % 16) % 16, content irrelevant)
wire    = AES-128-CBC(key, IV, padded) ‖ junk(r bytes, 0..12) ‖ info(1)
```

- CRC16 = CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF, no reflection, no final xor;
  `Hashes::CRC16`, 0x281bb0/0x281ca0).
- CBC restarts with the same IV for every message (no chaining across messages).
- `r` is random (engine PRNG); the junk bytes are random data encrypted once more with the key
  (any bytes are accepted).
- `info`/`check`:
  - `r ≥ 1`: `info = (p << 4) | r`, `check = (r << 4) | p`
  - `r = 0`, `p > 0`: `info = p`, `check = p << 4`
  - `r = 0`, `p = 0`: `info = 0x80 | q` (q random 0..14), `check = (q << 4) | 8`
- Decoder: `r = (len & 15) - 1` (a wire length that is a multiple of 16 is invalid), decrypt
  the first `len & ~15` bytes, drop `p` padding bytes, verify check byte and CRC16, payload is
  what remains. Any failure drops the message (and flags the context).

### Key rotation (messages 170 / 171)

1. Client generates a 32-byte token with `0x7b3580` (structured: an RC4/AES-scrambled 16-byte
   value whose 32-bit word sum is divisible by 0x591DF010, parameters
   `0x43d0, 0x4ff, 0xf5b, 0x4bd, 0x989`; validator `0x7b3b30`), encrypts the 32 bytes with its
   *current* key (CBC, in place) and sends message **170** `{rawdata token}`.
2. Server (`0xcad580`): decrypts the 32 bytes with the player's current key, validates
   (`0x7b3b30`), queues it as pending (`player+0x2b8`), answers **171** `{rawdata token}`
   (plaintext token inside the normally encrypted message).
3. On **171** the receiver calls `SEncryptionContext::Update`: if the token equals the pending
   one, the context switches to `msg_key_from_token(token)`.

Our server may simply accept any 32-byte token, answer 171, and switch keys at the same points
as the original; exact switch timing per direction should be confirmed with a capture.

## Envelope (`0xbbc3b0`)

```
u32   time        raw little-endian (sender's message time)
var   type        varint (unsigned LEB128)
if type != 15:    items of format[type]
if type == 15:    var count; count × { var type; items of format[type] }     ("message batch")
```

Message type ids are indexes into the format table (0..490). Format 15 has no items: it is the
batch container.

## Item encodings (`0xc760c0`; primitives 0xc79*/0xc7a*)

All little-endian, byte aligned. `var` = unsigned LEB128 (≤ 5 bytes for u32). Each item has a
data type and a compression (`net-message-formats.tsv` prints `type:compression`).

| Type | Name in TSV | Encoding |
|---|---|---|
| 0 | `variant` | `var dataType`, `var compression`, then that item (types ≤ 27, compression ≤ 13) |
| 1 | `optmsg` | `u8 present (0/1)`; if 1 the nested message body (`>N` = its type) |
| 2 | `bool` | `u8` 0/1 (anything else is an error) |
| 3 | `int` | c=0: `i32`; c=1: `var`; c=2: `var z`, value = `z>>1`, negated if `z&1` |
| 4 | `int64` | `i64` |
| 5 | `float` | c=0,2: `f32`; c=4: `u8/254`; c=5: `u8/127`; c=6: `u8/127 - 1`; c=7: `u8*2π/254 - π`; c=8: `(var z → ±(z>>1)) + 127` then `/254`; c=9: same with the angle scale; c=11: IEEE half (`u16`, 0 → 0.0) |
| 6 | `string` | c=0: NUL-terminated bytes; c=2 / c=3: `var id`, 0 → NUL-terminated string follows, else entry `id-1` of string table A (0x21cce90) / B (0x21ce490) |
| 7 | `rawdata` | `var len` + bytes (other compressions: see 0xc798c0) |
| 8 | `u32` | `u32` |
| 9 | `vector` | c=0,2: 3×`f32`; c=4: packed `u32` direction (bits 22-31 → x·(-1/511), 11-21 → y·(-1/1023), 0-10 → z·(-1/1023), signed fields); c=10: packed camera position (`u32`, scale table 0x1baac80, centre 6400/500/6400, `0xFFFFFFFF` = −FLT_MAX); c=11: 3× half |
| 10 | `matrix` | c=0,2: 9×`f32`; c=12: 11 bytes — two `i16` pairs (two unit vectors, z from √(1−x²−y²), sign bits in byte 8) + half-float scale (`0x3c00` = 1.0) |
| 11/12/13/14 | `boolarr`/`intarr`/`floatarr`/`stringarr` | `var count` + elements encoded as type 2/3/5/6 with the item's compression |
| 15 | `idstrarr` | array of (id, string) pairs (`AutoArrayWithID<RString>`) |
| 16 | `strfloatarr` | `var count` × {string, float} |
| 17 | `msg` | nested message body of type `>N`, inline |
| 18 | `msgarr` | `var count` × {`u8 present`, body} |
| 19 | `optmsgarr` | `var count` × {`u8 present`; if 1: `var type` + body of that type} (polymorphic) |
| 20 | `netid` | `var creator`, `var id` — network object id (`net-object-model.md`) |
| 21 | `netidarr` | `var count` × netid |
| 22 | `arr22` | array (element encoding: 0xc760c0 case 0x16) |
| 23 | `blob36` | 36 raw bytes |
| 24 | `arr16` | `var count` × 16 raw bytes |
| 25 | `locstr` | `var` + string (localised string: id, text) |
| 26 | `locstrarr` | `var count` × locstr |
| 27 | `byte` | `u8` |

Items with a fixed offset (`item+0x10 != -1`) are bound directly to a field of the message
object; it does not change the wire encoding. The `/e<type>:<coef>` annotations are error
metrics used to prioritise object updates.

**String tables A and B** (compressions 2 and 3) are built at start-up on both sides; their
content and order must match the client's exactly. Building them is an open question
(follow-up) — until then, always send compression-2/3 strings with id 0 + literal text, which
every reader accepts.

## Who handles which id

`docs/re/net-message-dispatch.tsv` lists, for every id, the case of the server dispatcher
(`NetworkServer::OnMessage`, jump tables 0xcad310/0xcad0e4, type−2 indexed) and of the client
dispatcher (`NetworkClient::OnMessage` 0xc3ed30, table 0xc52440, type−1 indexed), plus the
strings each case references. "unhandled" = the case logs `Unhandled user message` (that side
never receives the id). Message object classes are named `NMMA_<n>` (decoys, no semantics).

## Known message ids

From handler strings and code (medium confidence unless noted):

| Id | Items | Direction | Meaning |
|---|---|---|---|
| 15 | — | both | batch container (high) |
| 14 | 12 | S→C | player identity / BattlEye client init (`"Identity"`, BE errors) |
| 29 | 84 | S→C | mission header: mission, island, difficulty, required DLCs/addons (`"Missing DLCs:"`, `"Unknown difficulty: %s"`, `.pbo`) |
| 36 | 5 | S→C | mission file offer: name + hash, client looks in `$\MPMissionsCache\` (`"Mission-Hash:"`, `"Mission-File-Name:"`) |
| 35 | 6 | both | mission file transfer / download status (server logs `"Client mission file download from HTTP mirror failed"`) |
| 37 | 3 | C→S | mission download report (same server case as 35) |
| 39 | — | C→S | AskForDamage, obsolete (`"NMTAskForDamage obsolete"`) |
| 42 | — | S→C | get-in (`"Client: Unknown get in position %d"`) |
| 57 | — | S→C | fire weapon (`"NMTFireWeapon"`) |
| 73 | — | S→C | replace container (`"NMTReplaceContainer"`) |
| 168 / 169 | `rawdata` | C→S | key exchange, second pair (decrypt with `player+0x290`, validator 0x7b3b00) |
| 170 / 171 | `rawdata` | both | key proposal / acknowledgement (see "Key rotation") (high) |
| 174 | 2 | both | server command / admin login (`#login`; server case reads `admins`, `passwordAdmin`, `missionWhitelist`) |
| 232–240, 326 | — | C→S | briefing messages, logged "NOT IMPLEMENTED - briefing!" on the server |
| 268 | 2 | C→S | mission selection / vote (`missionWhitelist`) |
| 300 | `rawdata` | C→S | data block handed to `0x7aeeb0` (`server+0x458`); sets player flag 4 |
| 342 | 2 | S→C | mission event (`"MissionEvent:%d"`) |
| 355 | 2 | C→S | change owner (`"Server: OwnerChanged of %d:%d arrived from non owner %d"`) |
| 376, 403, 407 | — | C→S | set object material (`.rvmat`, `"User tried to set invalid material"`) |
| 395 | 7 | C→S | remoteExecCall (no JIP) |
| 396 | 8 | C→S | remoteExec / JIP-capable (`"JIP is not enabled for %s"`, command/function, targets) |
| 105–156 etc. | — | C→S | object updates from the owning client (`"Unit %d:%d not found, cannot update"`, one shared server case for ~40 ids) |

The rest still needs naming (follow-up issue): login request, publicVariable, chat, time sync,
object create/delete, JIP queue replay.
