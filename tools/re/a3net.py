"""Reference codec for the Arma 3 2.22 UDP transport (docs/re/net-transport.md, net-handshake.md).

Executable documentation: every constant and algorithm here is transcribed from arma3_x64.exe
2.22.0.154103 (function RVAs in the docs). Not yet validated against a live capture — see the
confidence notes in the docs. Standard library only.

Usage:
    python tools/re/a3net.py selftest
    python tools/re/a3net.py keys [magic]            # print derived obfuscation keys
    python tools/re/a3net.py decode <hex-packet>     # de-obfuscate + parse one UDP datagram
"""

from __future__ import annotations

import hashlib
import struct
import sys
import zlib
from dataclasses import dataclass

MAGIC = 0x25252525          # NetServer/NetClient "magic", hard-coded at both call sites
HEADER_SIZE = 24
MAX_DATAGRAM = 0x800        # receive buffer 0x7ff; header.size must equal the datagram length
DEFAULT_MAX_PACKET = 1400   # basic.cfg sockets.maxPacketSize default (DAT 0x208b968)

# header.flags bits (u16 at offset 2)
F_RELIABLE = 0x8000      # guaranteed delivery; resent until acked
F_URGENT = 0x4000        # second ("urgent") reliable lane
F_ORDERED = 0x2000       # payload must be delivered after the reliable message whose serial is in `extra`
F_NOCHANNEL = 0x1000     # sent outside a channel (connect reject); receiver skips ack processing
F_CONTROL = 0x0800       # connection-less control message, routed to the peer's default handler
F_ACK_RTT = 0x0400       # `ack` names a packet we received `extra` microseconds before sending this one
F_RTT_REQ = 0x0200       # please answer with F_ACK_RTT
F_BANDWIDTH = 0x0100     # `extra` carries the sender's bandwidth estimate
F_BW_PROBE = 0x0080      # bandwidth probe (packet pair)
F_ACK_ONLY = 0x0040      # no user payload; payload may hold an extended ack block (EXT_ACK_MAGIC)
F_USES_EXTRA = F_ORDERED | F_ACK_RTT | F_BANDWIDTH  # 0x2500: `extra` in use, ack mask is 32-bit
EXT_ACK_MAGIC = 0xB18AC212


def _f32(x: float) -> float:
    return struct.unpack("<f", struct.pack("<f", x))[0]


def _round_half_even(x: float) -> int:
    return int(round(x))  # Python round() is half-to-even, like cvtss2si with default MXCSR


def _sar(v: int, n: int) -> int:
    v &= 0xFFFFFFFF
    if v & 0x80000000:
        v -= 1 << 32
    return v >> n


def prng_seed(x: int, y: int) -> int:
    """0x30dde0: interleave low bits of x and y into the LCG seed (transcribed from the asm)."""
    ecx = (x & 1) * 2
    x = _sar(x, 1)
    ecx |= y & 1
    y = _sar(y, 1)
    for _ in range(3):
        ecx = ecx * 2 | (x & 1)
        x = _sar(x, 1)
        ecx = ecx * 2 | (y & 1)
        y = _sar(y, 1)
    ecx = ecx * 2 | (x & 1)
    edx = x & 2
    ecx = ecx * 2 | (y & 1)
    y = _sar(y, 1)
    return ((ecx << 2) | (y & 1) | edx) & 0xFFFFFFFF


def prng_bytes(a: int, b: int, n: int = 32) -> bytes:
    """0x2814a0 without extra data: seed=prng_seed(b, a); LCG 0x30e310; byte = clamp(rint(f*255-0.5), 0, 254)."""
    state = prng_seed(b, a)
    out = bytearray()
    for _ in range(n):
        state = (state * 0xC1C64E6D + 0x3039) & 0x7FFFFFFF
        f = _f32(_f32(float(state)) * _f32(4.656613e-10))
        v = _round_half_even(_f32(_f32(f * 255.0) - 0.5))
        out.append(0 if v < 0 else min(v, 0xFE))
    return bytes(out)


def _i32(v: int) -> int:
    return v & 0xFFFFFFFF


@dataclass(frozen=True)
class Keys:
    header_flags_key: bytes   # RC4 key for header bytes [2..4)
    header_tail_key: bytes    # RC4 key for header bytes [8..24)
    payload_table: bytes      # 2048-byte XOR table for the payload


def derive_keys(magic: int = MAGIC) -> Keys:
    """NetPeerUDP constructor 0x352200."""
    m = magic & 0xFFFFFFFF
    m_le = struct.pack("<I", m)
    k1 = hashlib.sha1(prng_bytes(_i32(-0x50A5AA3 - m), _i32(m + 0x12CA1B)) + m_le).digest()
    k2 = hashlib.sha1(prng_bytes(_i32(m + 0x8E3B8C), _i32(-0x29CD4BC6 - m)) + m_le).digest()
    lo, hi = m & 0xFFFF, (m >> 16) & 0xFFFF
    lfsr = ((lo ^ hi) & 0x5555) ^ hi
    if lfsr == 0:
        lfsr = 0xACE1
    table = bytearray()
    for _ in range(0x800):                       # 0x345d90
        table.append((lfsr >> 1) & 0xFF)
        lfsr = ((-(lfsr & 1)) & 0xB400) ^ (lfsr >> 1)
        lfsr &= 0xFFFF
    return Keys(k1, k2, bytes(table))


def rc4_drop(key: bytes, n: int, drop: int = 0x200) -> bytes:
    """0x352fc0: RC4 keystream after discarding `drop` bytes."""
    s = list(range(256))
    j = 0
    for i in range(256):
        j = (j + s[i] + key[i % len(key)]) & 0xFF  # key bytes are signed chars; same mod 256
        s[i], s[j] = s[j], s[i]
    i = j = 0
    out = bytearray()
    for k in range(drop + n):
        i = (i + 1) & 0xFF
        j = (j + s[i]) & 0xFF
        s[i], s[j] = s[j], s[i]
        if k >= drop:
            out.append(s[(s[i] + s[j]) & 0xFF])
    return bytes(out)


def _payload_mask_start(serial: int) -> int:
    """0x352a20: start index into the 2048-byte table, derived from the plaintext serial."""
    u = serial & 0xFFFFFFFF
    u = ((_sar(u ^ 0x3D0000, 16) ^ u) * 9) & 0xFFFFFFFF
    u = ((_sar(u, 4) ^ u) * 0x27D4EB2D) & 0xFFFFFFFF
    return 0x18 + ((_sar(u, 15) ^ u) & 0x7FFF)


def xor_packet(buf: bytearray, keys: Keys) -> None:
    """Apply (or remove — XOR is symmetric) the obfuscation. Header serial must be plaintext when
    computing the payload mask, so call header-first when decoding and payload-first when encoding."""
    raise NotImplementedError  # use obfuscate()/deobfuscate()


def _xor_header(buf: bytearray, keys: Keys) -> None:
    for i, k in enumerate(rc4_drop(keys.header_flags_key, 2)):
        buf[2 + i] ^= k
    for i, k in enumerate(rc4_drop(keys.header_tail_key, 16)):
        buf[8 + i] ^= k


def _xor_payload(buf: bytearray, keys: Keys) -> None:
    size = struct.unpack_from("<H", buf, 0)[0]
    serial = struct.unpack_from("<I", buf, 8)[0]
    start = _payload_mask_start(serial)
    # loop index runs 0x18..size inclusive in the original (one byte past the end is touched only
    # if the buffer is larger; we stop at the datagram end)
    for n, idx in enumerate(range(0x18, min(size, len(buf)))):
        buf[idx] ^= keys.payload_table[(start + n) & 0x7FF]


def crc_of(buf: bytes) -> int:
    size = struct.unpack_from("<H", buf, 0)[0]
    b = bytearray(buf[:size])
    b[4:8] = b"\0\0\0\0"
    return zlib.crc32(bytes(b)) & 0xFFFFFFFF


@dataclass
class Header:
    size: int
    flags: int
    crc: int
    serial: int
    ack: int
    ack_mask: int   # 64-bit when flags & F_USES_EXTRA == 0, else 32-bit
    extra: int      # u32 at offset 20 when flags & F_USES_EXTRA, else 0


def pack(flags: int, serial: int, ack: int, ack_mask: int, extra: int, payload: bytes, keys: Keys) -> bytes:
    size = HEADER_SIZE + len(payload)
    if size > MAX_DATAGRAM:
        raise ValueError("datagram too large")
    b = bytearray(size)
    struct.pack_into("<HHII", b, 0, size, flags, 0, serial)
    struct.pack_into("<I", b, 12, ack)
    if flags & F_USES_EXTRA:
        struct.pack_into("<II", b, 16, ack_mask & 0xFFFFFFFF, extra)
    else:
        struct.pack_into("<Q", b, 16, ack_mask)
    b[24:] = payload
    struct.pack_into("<I", b, 4, crc_of(b))
    _xor_payload(b, keys)
    _xor_header(b, keys)
    return bytes(b)


def unpack(datagram: bytes, keys: Keys) -> tuple[Header, bytes]:
    if len(datagram) < HEADER_SIZE or len(datagram) > MAX_DATAGRAM:
        raise ValueError("not a transport datagram (size)")
    if struct.unpack_from("<H", datagram, 0)[0] != len(datagram):
        raise ValueError("not a transport datagram (size field)")
    b = bytearray(datagram)
    _xor_header(b, keys)
    _xor_payload(b, keys)
    size, flags, crc, serial, ack = struct.unpack_from("<HHIII", b, 0)
    if flags & F_USES_EXTRA:
        mask, extra = struct.unpack_from("<II", b, 16)
    else:
        mask, extra = struct.unpack_from("<Q", b, 16)[0], 0
    if crc_of(b) != crc:
        raise ValueError(f"crc mismatch {crc:08x} != {crc_of(b):08x}")
    return Header(size, flags, crc, serial, ack, mask, extra), bytes(b[24:])


# ---- connect handshake (docs/re/net-handshake.md) ---------------------------------------------

CTRL_HELLO = 0xBBBA1564        # client -> server, 12 bytes
CTRL_CHALLENGE = 0xA5A48965    # server -> client, 8 bytes
CTRL_CONNECT = 0xCCCA1E12      # client -> server, 156 bytes (no challenge)
CTRL_CONNECT_CH = 0xCCCA5E62   # client -> server, 160 bytes (with challenge)
CTRL_RESULT = 0xAAA51A7E       # server -> client, 12 bytes
CTRL_QUERY = 0xEEE191AE        # client -> server, 17 bytes: session info request
CTRL_QUERY_REPLY = 0xFFF1E8AC  # server -> client, session info (variable)
BUILD = 154103
VERSION = 222

STEAM_BLOB_KEY = hashlib.sha1(b"8CFB1217-A5BC-465F-AB31-5DCB4AE7F58A").digest()


def steam_blob_encode(steam_id: int) -> bytes:
    """0x9cb360: '[' + SteamID64 LE, RC4-drop512 with SHA1(GUID)."""
    plain = b"[" + struct.pack("<Q", steam_id)
    return bytes(p ^ k for p, k in zip(plain, rc4_drop(STEAM_BLOB_KEY, 9)))


def steam_blob_decode(blob: bytes) -> int | None:
    """0x9cb2a0: returns the SteamID64 or None if the marker byte is wrong."""
    plain = bytes(p ^ k for p, k in zip(blob[:9], rc4_drop(STEAM_BLOB_KEY, 9)))
    return struct.unpack("<Q", plain[1:9])[0] if plain[0] == 0x5B else None


def connect_request(name: str, password: str, mod_string: str, steam_id: int, player_id: int,
                    challenge: int | None = None, flags95: int = 0, flags96: int = 0,
                    be_state: int = 0, actual: int = VERSION, required: int = VERSION) -> bytes:
    """Client -> server CONNECT (0x7b6160). Packed little-endian, no padding."""
    def s40(s: str) -> bytes:
        return s.encode("utf-8")[:39].ljust(40, b"\0")
    body = struct.pack("<II", CTRL_CONNECT if challenge is None else CTRL_CONNECT_CH, MAGIC)
    body += s40(name) + s40(password) + s40(mod_string) + steam_blob_encode(steam_id)
    body += struct.pack("<IIIBBiB", actual, required, BUILD, flags95, flags96, player_id, be_state)
    assert len(body) == 0x9C
    if challenge is not None:
        body += struct.pack("<I", challenge)
    return body


def selftest() -> None:
    keys = derive_keys()
    pkt = pack(F_RELIABLE | F_RTT_REQ, 1000, 7, 0xFFFF, 0, b"hello arma", keys)
    h, p = unpack(pkt, keys)
    assert p == b"hello arma" and h.serial == 1000 and h.flags == F_RELIABLE | F_RTT_REQ
    pkt2 = pack(F_ORDERED | F_RELIABLE, 5, 4, 0x3, 2, b"x" * 100, keys)
    h2, p2 = unpack(pkt2, keys)
    assert h2.extra == 2 and h2.ack_mask == 3 and p2 == b"x" * 100
    sid = 76561197960287930
    assert steam_blob_decode(steam_blob_encode(sid)) == sid
    assert len(connect_request("player", "", "", sid, 123, challenge=42)) == 0xA0
    print("selftest ok")


def main():
    if len(sys.argv) < 2 or sys.argv[1] == "selftest":
        selftest()
    elif sys.argv[1] == "keys":
        k = derive_keys(int(sys.argv[2], 0) if len(sys.argv) > 2 else MAGIC)
        print("header[2:4] key :", k.header_flags_key.hex())
        print("header[8:24] key:", k.header_tail_key.hex())
        print("payload table   :", k.payload_table[:32].hex(), "...")
        print("header[2:4] mask:", rc4_drop(k.header_flags_key, 2).hex())
        print("header[8:24]mask:", rc4_drop(k.header_tail_key, 16).hex())
    elif sys.argv[1] == "decode":
        h, p = unpack(bytes.fromhex(sys.argv[2]), derive_keys())
        print(h)
        print(p.hex())


if __name__ == "__main__":
    main()
