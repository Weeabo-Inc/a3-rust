# Network transport (UDP packet layer)

Arma 3 2.22.0.154103. Source: `arma3_x64.exe` (the client contains the full server code too);
addresses are **RVAs** (VA = RVA + 0x140000000). Reference implementation of everything on this
page: `tools/re/a3net.py` (`python tools/re/a3net.py selftest | keys | decode <hex>`).

**Confidence:** high for layouts, flags and algorithms (read directly from the code paths named
below). **Not yet validated against a live capture** — the first capture will either confirm the
derived keys byte-for-byte or show a transcription error (see "Validation" at the end).

## Layering

```
UDP datagram  ─ NetPeerUDP (0x352200 ctor, recv thread 0x354e10 "udpNet", send 0x354930)
  └ 24-byte header + payload, CRC32, obfuscation             ← this page
     └ NetChannelBasic (vtable 0x1aa9a18): serials, acks, RTT, reliable lanes, resend
        └ NetMessage = one datagram payload; large guaranteed messages are split
           └ NetTranspClient/NetTranspServer (NetClient 0x7b5830, NetServer 0x7b9650):
              control handshake (net-handshake.md), then "user messages"
              └ NetworkClient/NetworkServer: game messages (net-messages.md)
```

There is no TCP. Ports: the game port (default **2302**) carries everything on this page. Steam
queries (A2S) are answered by the Steam game-server library on the query port (game port + 1 by
default; log line `"Initializing Steam server - Game Port: %d, Steam Query Port: %d"`), see
`net-a2s.md`. BattlEye uses its own port and is out of scope.

## Datagram layout

All integers little-endian. One datagram = one NetMessage.

| Offset | Type | Field | Notes |
|---|---|---|---|
| 0 | u16 | `size` | total datagram length including header. **Not obfuscated.** Receiver drops the datagram unless `size == bytes received`, `size >= 24` and `size <= 0x800`. |
| 2 | u16 | `flags` | see below. Obfuscated. |
| 4 | u32 | `crc` | CRC-32 (IEEE/zlib polynomial 0xEDB88320, init 0xFFFFFFFF, final xor) of bytes `[0, size)` of the **plaintext** datagram with this field set to 0. Not obfuscated. |
| 8 | u32 | `serial` | per-channel packet sequence number, +1 per datagram sent (incl. resends, which get a new serial) |
| 12 | u32 | `ack` | highest serial received from the peer |
| 16 | u64 | `ack_mask` | bit i set = serial `ack - 63 + i` received, when `flags & 0x2500 == 0` |
| 16 | u32 | `ack_mask` | 32-bit mask (`ack - 31 + i`) when `flags & 0x2500 != 0` … |
| 20 | u32 | `extra` | … and then this field carries per-flag data (0x2000 predecessor serial, 0x0400 delay µs, 0x0100 bandwidth) |
| 24 | bytes | payload | `size - 24` bytes |

Bytes `[8, 24)` are obfuscated as a block; bytes `[24, size)` with a separate keystream.

Receive path (`0x354e10`): `recvfrom` into a 0x7ff-byte buffer → size checks → `vtable+0xb8`
(`0x352ad0`, header de-obfuscation) → `vtable+0xc0` (`0x352a20`, payload de-obfuscation) → CRC
check (`0x286e10/0x286f40/0x287180`, class `Hashes::CRC32SSE`: table at 0x1a95320 is the standard
CRC-32 table, the SSE path is PCLMUL folding of the same CRC) → peer lookup by FNV-1a-64 of the
source `sockaddr` → `F_CONTROL` packets go to the default handler (`vtable+0x40`), others to the
channel's `vtable+0x90` (`0x34e8e0`). Datagrams failing the size check are passed to an optional
raw handler (`peer+0x110`, set by the creator) — used for non-protocol traffic.

Send path: build plaintext, compute CRC with field zeroed, store it, obfuscate payload (needs the
plaintext serial), obfuscate header.

## Obfuscation (not encryption)

All keys are derived from one 32-bit `magic`, which is the constant **0x25252525** passed to
`NetServer::Init` (call at 0xc88ff6) and to the client connect (0xc39da5). Everything is
therefore fixed and identical for every server and client of this build.

`NetPeerUDP` constructor `0x352200`, with `m = 0x25252525`:

1. **Header keys** (SHA-1 via CryptoAPI `CALG_SHA1`, helpers `0x2810b0/0x281150/0x281180`):
   - `key1 = SHA1(R(A1, B1) ‖ le32(m))`, `A1 = -0x050A5AA3 - m`, `B1 = m + 0x0012CA1B`
   - `key2 = SHA1(R(A2, B2) ‖ le32(m))`, `A2 = m + 0x008E3B8C`, `B2 = -0x29CD4BC6 - m`
   - `R(A, B)` (`0x2814a0`) = 32 bytes: seed `s = interleave(B, A)` (`0x30dde0`, a bit interleave of
     the low 6 bits, transcribed in `a3net.prng_seed`), then 32× `s = (s * 0xC1C64E6D + 0x3039) &
     0x7FFFFFFF` (`0x30e310`), `f = float32(s) * 2^-31`, `byte = clamp(cvtss2si(f*255.0f - 0.5f), 0, 254)`
     (single-precision SSE math, round-half-even).
2. **Header masks**: `0x352ad0` runs RC4 with `key1`, discards 512 bytes (`0x352fc0` = RC4-drop512),
   and XORs header bytes `[2,4)`; same with `key2` over bytes `[8,24)`. RC4 is re-keyed for every
   datagram, so these are **constant XOR masks**:
   - flags mask `9b 6d`
   - bytes 8..23 mask `bf 27 61 a1 c3 e5 d4 73 7f 19 b5 99 5d c0 20 d0`

   (values printed by `python tools/re/a3net.py keys`; to be confirmed by capture).
3. **Payload table** (`0x345d90`): 2048 bytes from a 16-bit Galois LFSR, polynomial mask 0xB400,
   seed `((m_lo ^ m_hi) & 0x5555) ^ m_hi` (0xACE1 if that is 0); `table[i] = (lfsr >> 1) & 0xFF`
   taken *before* each step `lfsr = (lfsr >> 1) ^ (-(lfsr & 1) & 0xB400)`.
4. **Payload XOR** (`0x352a20`), only when the table exists: with `u = serial` (plaintext),
   `u = ((sar(u ^ 0x3D0000, 16) ^ u) * 9)`, `u = ((sar(u, 4) ^ u) * 0x27D4EB2D)`,
   `start = 0x18 + ((sar(u, 15) ^ u) & 0x7FFF)`; byte `24+n` ^= `table[(start + n) & 0x7FF]`.
   (32-bit wrap-around arithmetic, `sar` = arithmetic shift.) The loop runs to `size` inclusive,
   touching one byte past the datagram inside the 2 KiB buffer; irrelevant on the wire.

## Flags (u16 at offset 2)

| Bit | Name (ours) | Meaning | Evidence |
|---|---|---|---|
| 0x8000 | RELIABLE | guaranteed: kept in the send window and resent until acked; counted in reliable stats | `0x34e8e0`, `0x34c450` |
| 0x4000 | URGENT | second reliable lane with its own ordering chain; queued ahead of normal traffic | `0x34c450` (queue 0xd0), `0x3520b0` |
| 0x2000 | ORDERED | `extra` = serial of the previous reliable message of the same lane; deliver after it | `0x3520b0` sets `0xa000` and `extra` |
| 0x1000 | NOCHANNEL | sent without a channel (connect reject); receiver skips ack bookkeeping | `0x34e8e0`, `0xb7efa0` |
| 0x0800 | CONTROL | connection-less control message; routed to the peer's default handler | `0x354e10`, handshake |
| 0x0400 | ACK_RTT | `ack` names a datagram the sender received `extra` µs before sending this one → RTT sample | `0x34da60` ("uRTT") |
| 0x0200 | RTT_REQ | asks the peer to answer with an ACK_RTT datagram | `0x350140`, `0x34bda0` |
| 0x0100 | BANDWIDTH | `extra` = sender's bandwidth estimate (receiver smooths 0.85/0.15) | `0x34da60` |
| 0x0080 | BW_PROBE | bandwidth probe; receiver measures inter-arrival time | `0x34da60` |
| 0x0040 | ACK_ONLY | no user data; payload may hold an extended ack block (below) | `0x34d600`, `0x34fd50` |
| 0x0020 | FRAGMENT | part of a split guaranteed message | `0x7b7600` |
| 0x0010 | LAST_FRAGMENT | final part (set together with 0x0020) | `0x7b7600` |
| 0x0001 | (control) | set on every handshake control message (`0x801`, `0x8001`, `0x1001`) | `0x7b6160`, `0xb7efa0` |

`flags & 0x2500 != 0` ⇒ `ack_mask` is 32-bit and `extra` is meaningful.

### Extended ack block (ACK_ONLY payload, `0x34d600` / writer `0x34fd50`)

`u32 0xB18AC212, u32 bandwidth, u32 hi_serial, u32 lo_serial, u32 masks[]` — each mask
acknowledges 32 serials starting at `lo_serial + 32*k`. Accepted only if payload ≥ 20 bytes.

## Channel behaviour (NetChannelBasic)

- **Serial window**: the receiver keeps a 1024-entry ring (`channel+0xa20`). A datagram is
  "refused" (logged `*** NetChannel (%u) refused a packet: serialID=%u (%u...%u)`) when its serial
  is outside `[lowest_kept, highest_received + 0x100]` or already seen. While the channel is
  still in its initial state, the first serial in `2..1000000` is adopted as the start.
- **Acks**: every outgoing datagram carries `ack`/`ack_mask` (`0x3503d0` fills them; 64-bit mask
  when there is no `extra`). The sender marks acked datagrams (`inputAcknowledgement`, `0x34d600`)
  and resends lost reliable payloads under a new serial (`"Ch(%u):sRetry"`).
- **Queues** (`0x34c450`): urgent (0xd0), reliable (0xa8), unreliable (0x100). Unreliable messages
  carry an 800 ms send deadline (`0x3521d0(msg, 800000)`) and are dropped if late.
- **RTT**: on RTT_REQ the receiver piggybacks ACK_RTT on its next datagram with `ack` = that
  serial and `extra` = µs between receipt and send. Bandwidth control uses many tunables
  (`ackTimeoutA/B`, `ackRedundancy`, `initBandwidth`, `minBandwidth`, `maxBandwidth`,
  `minLatencyUpdate`, `outWindow`, `ackWindow`, ...), read from `basic.cfg` class `sockets`
  (`0x7bf710`); defaults are in .data around 0x208b968.
- **Sizes**: `basic.cfg` `sockets.maxPacketSize`, default **1400** (DAT 0x208b968); the receive
  buffer is 2047 bytes and the hard limit is 0x800. The channel's max payload (`vtable+0x28`) is
  the per-message split threshold.

## Fragmentation (`NetTransp*::SendMsg` 0x7b7600, server variant 0x7bd780)

- Non-guaranteed message larger than the max payload → error `"trying to send a too large
  non-guaranteed message"`; never split.
- Guaranteed message larger than the max payload → split into consecutive parts of max-payload
  bytes. Every part has `0x8020` (`| 0x4000` if urgent), the last part additionally `0x0010`;
  each part is ORDERED after the previous one (`extra` = previous part's serial).
- Receiver (`0x7bd0b0`, `ProcessCommonMessages`): datagrams with `0x0020` are merged per source
  address (FNV-1a of the sockaddr) until the part with `0x0010` arrives, then the joined payload
  is processed as one message.

## Timeouts / keepalive

- Connect: client resends its handshake every 2 s and gives up after 8 s (`0x7b6160`).
- Server pending-challenge entries expire after 8 s (`0x7b9790(list, 8000000)`).
- Player disconnect: server.cfg `disconnectTimeout` (`"Kicking player name='%s', ID='%d'.
  DisconnectTimeout too high (%llu/%u ms)"`, `0x7bc1c0`). The exact keepalive message (if any
  beyond normal acks/RTT) is an open question; channels send ACK_ONLY datagrams when idle.

## Validation (next step)

1. Capture a real client ↔ `arma3server_x64.exe` session on localhost (Wireshark/pcap; captures
   stay in `.work/`).
2. Check: `size` field plain; `crc` verifies after `a3net.unpack`; flags of the first client
   datagram decode to `0x0801`; its payload starts with `64 15 ba bb 25 25 25 25 de 00 00 00`.
3. If the header masks are wrong, the payload magic and CRC will not verify — compare masks by
   XORing a known-plaintext control datagram.
