# Steam server queries (A2S) — what the official server publishes

The official dedicated server does not answer A2S itself. It hands its data to the Steamworks
game-server library (`ISteamGameServer015`, through `SteamInternal_ContextInit(0x2087710)`),
which answers `A2S_INFO`, `A2S_RULES` and `A2S_PLAYER` on the **Steam query port**. Our server
must speak the standard Source query protocol itself, with the values below. RVAs are in
`arma3_x64.exe` 2.22.0.154103 (the server code is the same in `arma3server_x64.exe`).

**Confidence:** high for the rules binary format (decoder `0x994d60` and encoder `0x178220` agree)
and the game-tag letters (table at 0x20b6b70); medium for the meaning of individual
values (names follow config/usage); unvalidated against a capture.

## Ports and init (`0x177500`)

`SteamInternal_GameServer_Init_V2(ip, gamePort, queryPort, eServerModeAuthenticationAndSecure=2,
"2.22…" version string at 0x1a7c8c8, …)`, logged as `"Initializing Steam server - Game Port: %d,
Steam Query Port: %d"`. The query port defaults to game port + 1 (2303 for 2302; server.cfg /
command line `-port`). Then `LogOnAnonymous`, `SetAdvertiseServerActive(true)`,
`SetModDir("Arma3")`, `SetGameDescription("Arma 3")`. The game expects AppID 107410
(`steam_appid.txt` check, `"Warning: Current Steam AppId: %s doesn't match expected value"`).

ISteamGameServer015 vtable offsets used (index 0 = `InitGameServer`): +0x10 SetGameDescription,
+0x18 SetModDir, +0x30 LogOnAnonymous, +0x60 SetMaxPlayerCount, +0x78 SetMapName,
+0x80 SetPasswordProtected, +0x98 ClearAllKeyValues, +0xa0 SetKeyValue, +0xa8 SetGameTags,
+0xb0 SetGameData, +0xc0 SetAdvertiseServerActive.

## A2S_INFO fields (updated by `0x179ae0`)

| A2S field | Source |
|---|---|
| Name | server hostname (server.cfg `hostname`) |
| Map | world (terrain) name (`SetMapName(netServer+0x60)`) |
| Folder | `"Arma3"` |
| Game | mission name; `"Waiting"` when no mission is loaded (`SetGameDescription`) |
| AppID | 107410 (Steam fills it from the init) |
| Players / Max players | Steam's player list / `SetMaxPlayerCount(maxPlayers)` |
| Visibility | `SetPasswordProtected(netServer+0x791)` |
| Version | the version string given at init (`2.22.<build>` form, 0x1a7c8c8) |
| Keywords (EDF 0x20) | the **game tags** string below |
| Server type / OS | dedicated / windows (Steam) |

`SetGameData` (`0x178610`) is set to `"<a> <b>"` built from the session and mission names (used by
the master-server text filter; medium).

### Game tags (keywords)

Comma-separated `letter + value` items (each followed by `,`), max 128 chars
(`"Error, Server tag string too long!"`). Built by `0x179ae0` into an indexed table and joined by
`0xb7d070` with letters from the table at 0x20b6b70. Booleans are `t`/`f`.

| Letter | Index | Value |
|---|---|---|
| `b` | 0 | BattlEye enabled (`t`/`f`) |
| `m` | 1 | flag `netServer+0x792` (t/f) — believed "required mods equal" (medium) |
| `r` | 2 | actual version, `222` |
| `n` | 3 | required build (`+0x794`) |
| `t` | 4 | mission game type, first 7 chars (e.g. `coop`) |
| `s` | 5 | server state (`+0x1038`, the same state number as `getClientStateNumber`-style session states) |
| `d` | 6 | dedicated (t/f) |
| `l` | 7 | session locked (`+0x790`) |
| `v` | 8 | verifySignatures (`+0x793`) |
| `g` | 9 | language id (`%d`) |
| `i` | 10 | difficulty index |
| `p` | 11 | platform, always `w` |
| `c` | 12 | `"%d-%d"` of two rounded floats (`+0x7c8`, `+0x7cc`); believed server longitude/latitude (medium) |
| `h` | 13 | content hash string (global at 0x220df68) |
| `o` | 14 | Steam IP country code (ISteamUtils `GetIPCountry`) |
| `e` | 15 | minutes left: 15 by default; when state = 7 (playing) the mission time left rounded, min 1 |
| `j` | 16 | `%g` of float `+0x2f8` (mission parameter 1) |
| `k` | 17 | `%g` of float `+0x2fc` (mission parameter 2) |
| `f` | 18 | allowedFilePatching (`+0x7ac`, 0/1/2) |
| `y` | 19 | (not set in this function) |

## A2S_RULES — binary mod/DLC block (`0x178b50` → `0x178220`, decoder `0x995a70`/`0x994d60`)

### Transport inside rules

1. Serialise the block (below); max 1024 bytes (DAT 0x20aecd0 = 0x400); on overflow the overflow
   flags are set and the trailing lists dropped.
2. Escape (`0x177140`, inverse `0x994b40`): byte `0x01` → `01 01`, `0x00` → `01 02`, `0xFF` → `01 03`.
3. Split the escaped string into chunks of **127** bytes (`0x177960`).
4. `ClearAllKeyValues`, then for chunk *i* (1-based) of *n*: `SetKeyValue(key, chunk)` with a
   2-byte key `{ (char)i, (char)n }`. Clients require `n` identical in all keys and `i < 'A'`
   (65); parts are concatenated in index order.

Other rules keys, if any, are ordinary strings (none found in this function).

### Block layout (all little-endian)

| Field | Type | Meaning |
|---|---|---|
| version | u8 | **3** (client accepts 2 or 3) |
| flags | u8 | bit0 = mod list dropped (overflow), bit1 = signature list dropped, bit2 = world is `tanoa`, bit3 = world is `enoch` (Livonia; only checked when state > 1) |
| dlc_mask | u16 | bit *i* set → a u32 DLC hash follows (bits 0..12 are read) |
| difficulty | u8 | bits 0-2 difficulty, bits 3-5 AI level, bit 6 flag, bit 7 flag (community names: advanced flight model, third person; medium) |
| flag | u8 | read as bool (crosshair, medium) |
| dlc_hashes | u32 × popcount(dlc_mask) | one per set bit, ascending bit order |
| mod_count | u8 | omitted if flags bit0 |
| mods | × mod_count | `u32 hash`, `u8 info` (low nibble = byte length *k* of the Steam workshop id, 0-8; bit 4 = is DLC), `k` bytes workshop id LE, `u8 len` + name bytes |
| sig_count | u8 | omitted if flags bit0 or bit1 |
| signatures | × sig_count | `u8 len` + key name bytes (bikey names) |

The DLC bits come from the table at 0x21a0d00 (DLC descriptors, matched by the loaded mods);
the exact DLC bit ↔ DLC mapping is a follow-up.

## A2S_PLAYER

Filled by Steam from the users the server registers (`SendUserConnectAndAuthenticate` /
`BUpdateUserData`). Our server reports player names (profile names) and score; duration from
join time. To confirm which score the official server reports (follow-up, capture).

## Challenge handling

Steam's A2S implementation (2020+) answers `A2S_INFO`, `A2S_RULES` and `A2S_PLAYER` with an
`S2C_CHALLENGE` (`0x41`) first; clients repeat the request with the challenge. Our server should
do the same (Steam's behaviour, not engine code).
