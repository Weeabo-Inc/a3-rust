# Connect handshake (transport level)

How a client gets from "first datagram" to "connected" with a player ID, before any game
message. Arma 3 2.22.0.154103, RVAs of `arma3_x64.exe` (VA = RVA + 0x140000000). Datagram
framing and the `flags` bits: `net-transport.md`. Builders/parsers: `tools/re/a3net.py`
(`connect_request`, `steam_blob_encode/decode`, `CTRL_*` constants).

Game-level login after this point (mission, addons, signatures, `#login`): `net-messages.md`
(in progress, issue #28).

**Confidence:** high for the message layouts, magic values and server checks (read from client
sender `0x7b6160` and server handler `0xb7efa0`); medium for field *names* where noted;
unvalidated against a capture.

## Sequence

```
client                                              server (NetServer, handler 0xb7efa0)
  | HELLO     0xBBBA1564  12 B   flags 0x0801  -->  |  magic ok? record pending entry, random challenge
  | <-- CHALLENGE 0xA5A48965   8 B   flags 0x0801   |
  | CONNECT   0xCCCA5E62 160 B   flags 0x0801  -->  |  challenge ok? checks (below), create channel
  | <-- RESULT 0xAAA51A7E  12 B   flags 0x8001      |  (accepted: reliable, on the new channel)
  |     RESULT 0xAAA51A7E  12 B   flags 0x1001      |  (rejected: no channel)
```

Client loop (`0x7b6160`): send HELLO **and** a challenge-less CONNECT (`0xCCCA1E12`, 156 B)
immediately; every 2 s resend; once a CHALLENGE has arrived (stored at `client+0xd0`) send
CONNECT `0xCCCA5E62` with the challenge appended; stop when RESULT arrives (`client+0xd4`) or
after 8 s (`"Ch(%u):cli(connected|failed,...)"`).

The server also accepts the challenge-less `0xCCCA1E12`: the first one from an address is only
recorded (list `server+0x760`); a repeat from the same address ≥ 1 ms later is processed like a
challenged CONNECT. So an implementation may ignore challenges entirely, but must answer HELLO
with a CHALLENGE so that the official client switches to `0xCCCA5E62`.

All control payloads are packed little-endian structs without padding.

## HELLO (client → server), 12 bytes

| Off | Type | Value |
|---|---|---|
| 0 | u32 | `0xBBBA1564` |
| 4 | u32 | magic `0x25252525` (server rejects silently if ≠ its magic, `server+0x738`) |
| 8 | u32 | client `actualVersion` = 222 (u16 zero-extended) |

## CHALLENGE (server → client), 8 bytes

| Off | Type | Value |
|---|---|---|
| 0 | u32 | `0xA5A48965` |
| 4 | u32 | challenge: `(rand() << 16) | rand()`, MSVC `rand` seeded with `GetTickCount() ^ 0x55555555` |

Pending entries live 8 s; the same address gets the same challenge while its entry exists.

## CONNECT (client → server), 156 or 160 bytes

| Off | Type | Field | Client value / server check |
|---|---|---|---|
| 0x00 | u32 | magic id | `0xCCCA1E12` (no challenge) or `0xCCCA5E62` (with challenge) |
| 0x04 | u32 | net magic | `0x25252525`; must equal the server's |
| 0x08 | char[40] | player name | profile name, NUL-terminated (server copies 56 bytes from here into its player record and warns `"name of a new player is too long => truncating"`) |
| 0x30 | char[40] | session password | `strncmp(.., server password, 40)`; mismatch → result **1** |
| 0x58 | char[40] | check string (name ours) | copied from the client's session descriptor (+0x30); if server flag `+0x329 & 1` is set, `_strnicmp` with server string `+0x301` (40), mismatch → result **6**. Meaning not yet identified (medium). |
| 0x80 | u8[9] | Steam blob | `'['` + SteamID64 (LE), RC4-drop512 with key `SHA1("8CFB1217-A5BC-465F-AB31-5DCB4AE7F58A")` (`0x9cb360` / decode `0x9cb2a0`, key init `0x58cd0`) |
| 0x89 | u32 | actualVersion | 222; server rejects (result **2**) if `< server requiredVersion` (`+0x29e`) |
| 0x8d | u32 | requiredVersion | 222; result **2** if `> server actualVersion` (`+0x29c`) |
| 0x91 | u32 | build | **154103** (0x259F7); result **2** if `<` server `requiredBuild` (server.cfg) |
| 0x95 | u8 | flag A | stored in the player record (`rec+5`); input to the BattlEye/platform check |
| 0x96 | u8 | flag B | bit 0 stored at `rec+4`; when set the server remembers this player ID at `+0x6b0` (likely "local/host player" — medium) |
| 0x97 | i32 | proposed player ID | client sends `(timeMs & 0x7FFFFFFF) + 0x14`; must be ≥ 0x14. Server takes the first free ID in `[id, id+12)` |
| 0x9b | u8 | BE state | client anti-cheat state; see check below |
| 0x9c | u32 | challenge | only in `0xCCCA5E62`; must match the pending entry |

### Server checks, in order (`0xb7efa0` from label LAB_140b7f691)

1. Length 0x9c or 0xa0 and `server+0x73c` (accepting) set; net magic matches; `id ≥ 0x14`.
2. Version: build ≥ `requiredBuild`, `actualVersion ≥ server.requiredVersion`,
   `requiredVersion ≤ server.actualVersion` → else **2**.
3. Anti-cheat (`0xb8fcb0(server_be_mode +0x32f, server_flag, client_be [0x9b], flagA [0x95])`):
   with BattlEye off on the server (mode 0, flag clear) a client with `be ≠ 0` is rejected unless
   its SteamID is in the server list at `+0x170` (`0xb7e2c0`) → **5**. **Our server: skip this
   check** (accept every value).
4. Check string (only if enabled) → **6**.
5. Password → **1**.
6. Duplicate address: an existing channel from the same address with the same ID is reused; an
   old one is destroyed first.
7. Free ID in the 12-wide window and `players < maxPlayers` (`+0x2f5`) else **4**.
8. Create the channel (`createChannel` failure → **3**), register the player record
   (`server+0x6b8`, 0x88 bytes each: id, flags, sockaddr, name, check string, build, SteamID).

## RESULT (server → client), 12 bytes

| Off | Type | Value |
|---|---|---|
| 0 | u32 | `0xAAA51A7E` |
| 4 | u32 | result: 0 accepted, 1 bad password, 2 version/build mismatch, 3 server error, 4 full / no ID, 5 anti-cheat/platform rejected, 6 check-string mismatch |
| 8 | u32 | assigned player ID (dpnid) when accepted |

Accepted: sent on the new channel with flags `0x8001` (reliable). Rejected: flags `0x1001`
(NOCHANNEL), addressed to the source.

## Session query (server info), `0xEEE191AE`

A 17-byte control message `{u32 0xEEE191AE, u32 magic, u8[9] Steam blob}` asks the server for its
session description. The server (token-bucket rate limited, `server+0x718..0x734`) answers with a
control datagram whose payload is the session-info block at `server+0x198`, first u32
`0xFFF1E8AC`, length `max(0x2d7, strlen(+0x46e) + 0x13f + 0x198)` clamped to the packet size; byte
`+0x197` of the reply is set to 2 when the requester's SteamID is in the `+0x170` list. Fields
include the session name (`+0x19c`), versions (`+0x29c` actual, `+0x29e` required), player counts
(`+0x2f5` max, `+0x2fb` current), flags (`+0x2f7`, `+0x329`), mission name, and a mods/addons
string (`+0x330`, truncated on a separator so it fits). Full layout: follow-up (needed for the
in-game server browser and probably for direct connect's pre-join info).

## What our server must do (summary for implementers)

- Answer HELLO with CHALLENGE (any u32); accept CONNECT with or without challenge.
- Check net magic, version 222 / build 154103 (or `requiredBuild`), password; ignore the
  anti-cheat byte; assign an ID; reply RESULT 0 reliably on the new channel.
- Decode and keep the SteamID from the Steam blob (no ticket validation happens here; the Steam
  auth ticket travels later in a game message — see `net-messages.md`).
