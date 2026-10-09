"""Steam A2S query client for `apps/a3-server` (docs/re/net-a2s.md).

Speaks the standard Source query protocol: `A2S_INFO`, `A2S_PLAYER` and `A2S_RULES`, including the
challenge dance a 2020-era Steam server requires (`S2C_CHALLENGE` first, then the same request with
the challenge appended). Standard library only, so it runs anywhere the repository does.

Usage:
    python tools/re/a2s_query.py [host] [port] [--timeout 2.0] [--raw]

The query port is the game port plus one by default (2303 for 2302). `--raw` prints the first
answer of each kind as hex, which is what the PR evidence quotes.
"""

from __future__ import annotations

import argparse
import socket
import struct
import sys

HEADER = b"\xff\xff\xff\xff"
REQUEST_INFO = b"T"
REQUEST_PLAYER = b"U"
REQUEST_RULES = b"V"
REQUEST_CHALLENGE = b"W"
SOURCE_ENGINE_QUERY = b"Source Engine Query\0"
CHALLENGE_UNSET = 0xFFFFFFFF

RESPONSE_CHALLENGE = 0x41
RESPONSE_INFO = 0x49
RESPONSE_PLAYER = 0x44
RESPONSE_RULES = 0x45

# Extra Data Flag bits of an A2S_INFO answer.
EDF_PORT = 0x80
EDF_STEAM_ID = 0x10
EDF_SPECTATOR = 0x40
EDF_KEYWORDS = 0x20
EDF_GAME_ID = 0x01


class Reader:
    """A little-endian cursor over one answer."""

    def __init__(self, data: bytes) -> None:
        self.data = data
        self.pos = 0

    def u8(self) -> int:
        value = self.data[self.pos]
        self.pos += 1
        return value

    def skip(self, count: int) -> None:
        self.pos += count

    def u16(self) -> int:
        value = struct.unpack_from("<H", self.data, self.pos)[0]
        self.pos += 2
        return value

    def u32(self) -> int:
        value = struct.unpack_from("<I", self.data, self.pos)[0]
        self.pos += 4
        return value

    def i32(self) -> int:
        value = struct.unpack_from("<i", self.data, self.pos)[0]
        self.pos += 4
        return value

    def u64(self) -> int:
        value = struct.unpack_from("<Q", self.data, self.pos)[0]
        self.pos += 8
        return value

    def f32(self) -> float:
        value = struct.unpack_from("<f", self.data, self.pos)[0]
        self.pos += 4
        return value

    def cstring(self) -> str:
        end = self.data.index(b"\0", self.pos)
        text = self.data[self.pos:end].decode("utf-8", "replace")
        self.pos = end + 1
        return text

    def remaining(self) -> int:
        return len(self.data) - self.pos


def request(sock: socket.socket, server: tuple[str, int], payload: bytes, timeout: float) -> bytes:
    """Send one request, follow a challenge answer and return the data answer."""
    sock.sendto(payload, server)
    deadline = timeout
    while True:
        sock.settimeout(deadline)
        data, _ = sock.recvfrom(0x1000)
        if len(data) < 5 or data[:4] != HEADER:
            raise SystemExit(f"not an A2S answer: {data.hex()}")
        kind = data[4]
        if kind != RESPONSE_CHALLENGE:
            return data
        challenge = struct.unpack_from("<I", data, 5)[0]
        print(f"  S2C_CHALLENGE {challenge:#010x}: repeating the request with it")
        if payload[4:5] == REQUEST_INFO:
            payload = payload + struct.pack("<I", challenge)
        else:
            payload = payload[:5] + struct.pack("<I", challenge)
        sock.sendto(payload, server)
        deadline = timeout


def show_info(data: bytes, raw: bool) -> None:
    if raw:
        print(f"  raw: {data.hex()}")
    reader = Reader(data)
    reader.skip(4)
    if reader.u8() != RESPONSE_INFO:
        raise SystemExit('not an A2S_INFO answer')
    protocol = reader.u8()
    name = reader.cstring()
    map_name = reader.cstring()
    folder = reader.cstring()
    game = reader.cstring()
    app_id = reader.u16()
    players = reader.u8()
    max_players = reader.u8()
    bots = reader.u8()
    server_type = chr(reader.u8())
    environment = chr(reader.u8())
    visibility = reader.u8()
    vac = reader.u8()
    version = reader.cstring()
    edf = reader.u8()
    port = reader.u16() if edf & EDF_PORT else 0
    steam_id = reader.u64() if edf & EDF_STEAM_ID else 0
    spectator = ""
    if edf & EDF_SPECTATOR:
        spectator = f"{reader.u16()}/{reader.cstring()}"
    keywords = reader.cstring() if edf & EDF_KEYWORDS else ""
    game_id = reader.u64() if edf & EDF_GAME_ID else 0
    print(f"  A2S_INFO protocol={protocol} name={name!r} map={map_name!r} folder={folder!r}")
    print(f"           game={game!r} appid={app_id} gameid={game_id} steamid={steam_id}")
    print(
        f"           players={players}/{max_players} bots={bots} type={server_type} "
        f"os={environment} password={visibility} vac={vac} version={version!r}"
    )
    print(f"           port={port} spectator={spectator!r} edf={edf:#04x} leftover={reader.remaining()}")
    print(f"           keywords={keywords!r}")


def show_players(data: bytes, raw: bool) -> None:
    if raw:
        print(f"  raw: {data.hex()}")
    reader = Reader(data)
    reader.skip(4)
    if reader.u8() != RESPONSE_PLAYER:
        raise SystemExit('not an A2S_PLAYER answer')
    count = reader.u8()
    print(f"  A2S_PLAYER {count} player(s)")
    for _ in range(count):
        index = reader.u8()
        name = reader.cstring()
        score = reader.i32()
        duration = reader.f32()
        print(f"           #{index} {name!r} score={score} duration={duration:.0f}s")
    print(f"           leftover={reader.remaining()}")


def show_rules(data: bytes, raw: bool) -> None:
    if raw:
        print(f"  raw: {data.hex()}")
    reader = Reader(data)
    reader.skip(4)
    if reader.u8() != RESPONSE_RULES:
        raise SystemExit('not an A2S_RULES answer')
    count = reader.u16()
    rules = []
    for _ in range(count):
        rules.append((reader.cstring(), reader.cstring()))
    print(f"  A2S_RULES {count} rule(s), leftover={reader.remaining()}")
    chunks = {}
    for key, value in rules:
        if len(key) == 2:
            # The key is two raw bytes: the 1-based chunk index and the chunk count.
            index, total = ord(key[0]), ord(key[1])
            chunks[index] = value
            print(f"           chunk {index}/{total} ({len(value)} bytes)")
        else:
            print(f"           {key!r} = {value!r}")
    if chunks:
        joined = b"".join(chunks[index].encode("latin-1") for index in sorted(chunks))
        block = unescape(joined)
        print(f"           binary block: {block.hex()}")
        describe_block(block)


def unescape(data: bytes) -> bytes:
    """Inverse of the engine's rules escaping: 01 01 -> 01, 01 02 -> 00, 01 03 -> FF."""
    out = bytearray()
    index = 0
    while index < len(data):
        byte = data[index]
        index += 1
        if byte != 0x01:
            out.append(byte)
            continue
        code = data[index]
        index += 1
        out.append({0x01: 0x01, 0x02: 0x00, 0x03: 0xFF}[code])
    return bytes(out)


def describe_block(block: bytes) -> None:
    """Decode the mod/DLC block of docs/re/net-a2s.md, as far as its version is known."""
    if not block:
        return
    reader = Reader(block)
    version = reader.u8()
    flags = reader.u8()
    dlc_mask = reader.u16()
    difficulty = reader.u8()
    flag = reader.u8()
    print(
        f"           block v{version} flags={flags:#04x} dlc_mask={dlc_mask:#06x} "
        f"difficulty={difficulty} flag={flag}"
    )
    for bit in range(13):
        if dlc_mask & (1 << bit):
            print(f"           dlc hash {bit}: {reader.u32():#010x}")
    if flags & 0x01:
        print("           mod list dropped (overflow)")
    else:
        mod_count = reader.u8()
        print(f"           {mod_count} mod(s)")
        for _ in range(mod_count):
            mod_hash = reader.u32()
            info = reader.u8()
            length = info & 0x0F
            workshop_id = int.from_bytes(reader.data[reader.pos : reader.pos + length], "little")
            reader.pos += length
            name = reader.data[reader.pos + 1 : reader.pos + 1 + reader.data[reader.pos]].decode(
                "utf-8", "replace"
            )
            reader.pos += 1 + reader.data[reader.pos]
            print(f"           mod hash={mod_hash:#010x} id={workshop_id} dlc={bool(info & 0x10)} {name!r}")
    if flags & 0x03 == 0:
        sig_count = reader.u8()
        print(f"           {sig_count} signature key(s)")
        for _ in range(sig_count):
            length = reader.u8()
            print(f"           {reader.data[reader.pos : reader.pos + length].decode('utf-8', 'replace')!r}")
            reader.pos += length


def main() -> int:
    # Player names are arbitrary text; a console that cannot encode one must not abort the query.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("host", nargs="?", default="127.0.0.1")
    parser.add_argument("port", nargs="?", type=int, default=2303)
    parser.add_argument("--timeout", type=float, default=2.0)
    parser.add_argument("--raw", action="store_true", help="print the first answer of each kind as hex")
    args = parser.parse_args()

    server = (args.host, args.port)
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    print(f"querying {args.host}:{args.port}")

    info = request(sock, server, HEADER + REQUEST_INFO + SOURCE_ENGINE_QUERY, args.timeout)
    if info[4] != RESPONSE_INFO:
        raise SystemExit(f"unexpected answer to A2S_INFO: {info[4]:#04x}")
    show_info(info, args.raw)

    players = request(
        sock, server, HEADER + REQUEST_PLAYER + struct.pack("<I", CHALLENGE_UNSET), args.timeout
    )
    if players[4] != RESPONSE_PLAYER:
        raise SystemExit(f"unexpected answer to A2S_PLAYER: {players[4]:#04x}")
    show_players(players, args.raw)

    rules = request(
        sock, server, HEADER + REQUEST_RULES + struct.pack("<I", CHALLENGE_UNSET), args.timeout
    )
    if rules[4] != RESPONSE_RULES:
        raise SystemExit(f"unexpected answer to A2S_RULES: {rules[4]:#04x}")
    show_rules(rules, args.raw)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

