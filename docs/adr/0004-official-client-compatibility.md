---
status: accepted
---

# Our Dedicated server is wire-compatible with the official Arma 3 2.22 client

Official Arma 3 clients at build 2.22.0.154103 must be able to join our Dedicated server via
Direct connect. The server speaks the original network protocol byte for byte, so the multiplayer
side of the project is judged against the real game client, not against our own client.

## Context

Phase 6 originally aimed for "two of our clients play on our server". That target lets us invent
a protocol, but it proves nothing about fidelity and leaves our server useless to the existing
player base until our client is complete. An official client joining our server is a much
stricter and more useful test: every message, every ID and every timing assumption of the
original must hold.

## Decision

Our Dedicated server is wire-compatible with the official 2.22.0.154103 client.

In scope (server side):

- Answer Steam A2S queries — A2S_INFO, A2S_RULES and A2S_PLAYER, including challenge handling —
  directly on the query port, without the Steamworks SDK.
- Accept the client's Steam auth ticket without validating it.
- No BattlEye.
- No signature verification: behave as the original does with `verifySignatures=0`.
- Authentication by Server password and Admin password (`#login`) only.

Explicitly out of scope:

- The Steamworks SDK (and any Steam game-server registration or master-server listing through it).
- Validating Steam auth tickets.
- BattlEye.
- Bisign/Bikey checks of client addons during join.
- Our client joining official servers. This is **not** a goal unless a later decision states it.

## Considered options

- **Own protocol between our client and our server**: rejected. Easier, but it cannot be checked
  against the original and does not let real players use our server.
- **Use the Steamworks SDK for A2S and ticket validation**: rejected. A closed native dependency
  with its own licence terms and a running Steam requirement; A2S is a small documented UDP
  protocol we can answer ourselves, and ticket validation is not needed for direct connect.

## Consequences

- **(a) Byte-compatible protocol, reverse engineered first.** The network protocol (transport,
  framing and reliability, handshake, version check, message catalogue, A2S answers) is reverse
  engineered from `arma3_x64.exe` and `arma3server_x64.exe` and documented in `docs/re/` before
  it is implemented. This makes network protocol RE a Phase 0 priority.
- **(b) Phase 4 mirrors the original network object model.** The World and Entity design in
  Phase 4 uses the original's Network object IDs, Owner and Locality, its split of which machine
  simulates what, and its object create/update/delete and ownership-transfer message flow. It is
  designed that way from the start, so that Phase 6 does not have to retrofit it.
- **(c) The server supplies what the client expects.** The same Mission transfer of the mission
  PBO, matching addon/mod lists as the client checks them, and the version/build check.
- **(d) Conformance tests from captures.** Protocol tests replay recorded captures of real
  official client <-> official server traffic. Captures are stored locally in `.work/` and are not
  committed if they contain anything sensitive (Steam IDs, tickets, IP addresses, player names);
  small synthetic fixtures derived from them are committed.
- **(e) Risks.**
  - The protocol may be encrypted or obfuscated in parts; if so, the scheme must be reverse
    engineered too, and it may change in any game update.
  - Version lock: the client and server version must match. We target exactly 2.22.0.154103; a
    new official build may change the protocol and needs a fresh RE pass and a decision whether
    to follow it.
  - Rules payload: the official server packs binary mod/DLC information into A2S_RULES; the
    client's server browser and direct connect depend on its exact encoding.
