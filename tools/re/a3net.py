"""Reference codec for the Arma 3 2.22 UDP transport (docs/re/net-transport.md, net-handshake.md).

Executable documentation: every constant and algorithm here is transcribed from arma3_x64.exe
2.22.0.154103 (function RVAs in the docs). Not yet validated against a live capture — see the
confidence notes in the docs. Standard library only.

Usage:
    python tools/re/a3net.py selftest
    python tools/re/a3net.py keys [magic]            # print derived obfuscation keys
    python tools/re/a3net.py decode <hex-packet>     # de-obfuscate + parse one UDP datagram
    python tools/re/a3net.py packet <flags> <serial> <ack> <ack_mask> <extra> [payload-hex]
                                                     # build one obfuscated datagram (test fixture)
    python tools/re/a3net.py offset <serial>...      # payload XOR table start for a plaintext serial
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


# ---- message layer: AES-128-CBC "DContext" (docs/re/net-messages.md) ---------------------

_SBOX = bytes.fromhex(
    "637c777bf26b6fc53001672bfed7ab76ca82c97dfa5947f0add4a2af9ca472c0b7fd9326363ff7cc34a5e5f171d8311504c723c31896059a071280e2eb27b27509832c1a1b6e5aa0523bd6b329e32f8453d100ed20fcb15b6acbbe394a4c58cfd0efaafb434d338545f9027f503c9fa851a3408f929d38f5bcb6da2110fff3d2cd0c13ec5f974417c4a77e3d645d197360814fdc222a908846eeb814de5e0bdbe0323a0a4906245cc2d3ac629195e479e7c8376d8dd54ea96c56f4ea657aae08ba78252e1ca6b4c6e8dd741f4bbd8b8a703eb5664803f60e613557b986c11d9ee1f8981169d98e949b1e87e9ce5528df8ca1890dbfe6426841992d0fb054bb16")
_INV_SBOX = bytes(_SBOX.index(i) for i in range(256))


def _xt(a: int) -> int:
    return ((a << 1) ^ 0x1B) & 0xFF if a & 0x80 else a << 1


def _mul(a: int, b: int) -> int:
    r = 0
    while b:
        if b & 1:
            r ^= a
        a, b = _xt(a), b >> 1
    return r


def _expand(key: bytes) -> list[bytes]:
    w = [list(key[i:i + 4]) for i in range(0, 16, 4)]
    rcon = 1
    for i in range(4, 44):
        t = list(w[i - 1])
        if i % 4 == 0:
            t = [_SBOX[b] for b in t[1:] + t[:1]]
            t[0] ^= rcon
            rcon = _xt(rcon)
        w.append([a ^ b for a, b in zip(w[i - 4], t)])
    return [bytes(sum(w[r * 4:r * 4 + 4], [])) for r in range(11)]


def _enc_block(rk: list[bytes], b: bytes) -> bytes:
    s = [x ^ k for x, k in zip(b, rk[0])]
    for r in range(1, 11):
        s = [_SBOX[x] for x in s]
        s = [s[(i + 4 * (i % 4)) % 16] for i in range(16)]  # ShiftRows (column-major)
        if r < 10:
            s = sum(([_mul(c[0], 2) ^ _mul(c[1], 3) ^ c[2] ^ c[3], c[0] ^ _mul(c[1], 2) ^ _mul(c[2], 3) ^ c[3],
                      c[0] ^ c[1] ^ _mul(c[2], 2) ^ _mul(c[3], 3), _mul(c[0], 3) ^ c[1] ^ c[2] ^ _mul(c[3], 2)]
                     for c in (s[j:j + 4] for j in range(0, 16, 4))), [])
        s = [x ^ k for x, k in zip(s, rk[r])]
    return bytes(s)


def _dec_block(rk: list[bytes], b: bytes) -> bytes:
    s = [x ^ k for x, k in zip(b, rk[10])]
    for r in range(9, -1, -1):
        s = [s[(i - 4 * (i % 4)) % 16] for i in range(16)]  # InvShiftRows
        s = [_INV_SBOX[x] for x in s]
        s = [x ^ k for x, k in zip(s, rk[r])]
        if r > 0:
            s = sum(([_mul(c[0], 14) ^ _mul(c[1], 11) ^ _mul(c[2], 13) ^ _mul(c[3], 9),
                      _mul(c[0], 9) ^ _mul(c[1], 14) ^ _mul(c[2], 11) ^ _mul(c[3], 13),
                      _mul(c[0], 13) ^ _mul(c[1], 9) ^ _mul(c[2], 14) ^ _mul(c[3], 11),
                      _mul(c[0], 11) ^ _mul(c[1], 13) ^ _mul(c[2], 9) ^ _mul(c[3], 14)]
                     for c in (s[j:j + 4] for j in range(0, 16, 4))), [])
    return bytes(s)


def _b(v: int) -> int:
    return v & 0xFF


def msg_key_from_token(token: bytes) -> tuple[bytes, bytes]:
    """(AES key, IV) for a key token (0x7b4c40 / 0x7b4d00 / 0x272ed0; verified by emulation).
    Only the low byte of CRC-32(token) matters."""
    c = zlib.crc32(token) & 0xFF
    k = bytearray()
    for i in range(16):
        v = _b(c * 0x16 - i * 0x1D - 0x21)
        for j in range(16):
            v = _b(v + (i * -0x69 + 0x1D) * j + c * 2 + i)
        k.append(v)
    b1 = _b(c * 2)
    iv = bytearray()
    for i in range(16):
        t = ((_b(i * 10) ^ _b(b1 + 0x4F)) + (_b(i * 0x13) ^ _b(b1 + 0x9E)) + (_b(i * 0x1C) ^ _b(b1 - 0x13))
             + (_b(i * 0x25) ^ _b(b1 + 0x3C)) + (_b(i * 0x2E) ^ _b(b1 + 0x8B)) + (_b(i * 0x37) ^ _b(b1 - 0x26))
             + (_b(i << 6) ^ _b(b1 + 0x29)) + (_b(i * 0x49) ^ _b(b1 + 0x78)) + (_b(i * 0x52) ^ _b(b1 - 0x39))
             + (_b(i * 0x5B) ^ _b(b1 + 0x16)) + (_b(i * 0x64) ^ _b(b1 + 0x65)) + (_b(i * 0x6D) ^ _b(b1 + 0xB4))
             + (_b(i * 0x76) ^ _b(b1 + 3)) + (_b(i * 0x7F) ^ _b(b1 + 0x52)) + (_b(i * -0x78) ^ _b(b1 + 0xA1))
             + (i ^ b1) + i * -0x71 - 0x43 + c * 0x16)
        iv.append(_b(t))
    g = bytes(_b(((i * i * 7 + 0x4B) * i - 0x20) * i) for i in range(16))
    return bytes(a ^ b for a, b in zip(k, g)), bytes(iv)


DEFAULT_MSG_KEY = msg_key_from_token(bytes(16))  # key 651287ec..., IV b32a11a8...


def crc16_ccitt(data: bytes) -> int:
    """Hashes::CRC16 (0x281bb0/0x281ca0): CRC-16/CCITT-FALSE, poly 0x1021, init 0xFFFF."""
    crc = 0xFFFF
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
    return crc


def msg_encrypt(payload: bytes, key_iv: tuple[bytes, bytes] = DEFAULT_MSG_KEY, junk: int = 0,
                q: int = 0) -> bytes:
    """Encoder 0x7b4940: payload|crc16|check, pad to 16, AES-CBC, then `junk` bytes and info byte."""
    key, iv = key_iv
    body = bytearray(payload) + struct.pack("<H", crc16_ccitt(payload)) + b"\0"
    check_pos = len(body) - 1
    pad = (-len(body)) % 16
    body += b"\0" * pad
    if junk:
        info = (pad << 4) | junk
        check = (junk << 4) | pad
    else:
        info = pad if pad else (q & 0xF) | 0x80
        check = ((info >> 4) | (info << 4)) & 0xFF
    body[check_pos] = check
    rk, prev, out = _expand(key), iv, bytearray()
    for i in range(0, len(body), 16):
        prev = _enc_block(rk, bytes(a ^ b for a, b in zip(body[i:i + 16], prev)))
        out += prev
    return bytes(out) + bytes(junk) + bytes([info])


def msg_decrypt(data: bytes, key_iv: tuple[bytes, bytes] = DEFAULT_MSG_KEY) -> bytes:
    """Decoder 0x7b4790. Raises ValueError on any check failure."""
    key, iv = key_iv
    n = len(data)
    r = (n & 0xF) - 1
    if r < 0:
        raise ValueError("length is a multiple of 16")
    info = data[-1]
    if r >= 1:
        pad = info >> 4
        if info & 0xF != r:
            raise ValueError("info byte mismatch")
        want_check = ((info >> 4) | (info << 4)) & 0xFF
    else:
        pad = info & 0xF if info < 0x80 else 0
        want_check = None
    clen = n - (n & 0xF)
    rk, prev, plain = _expand(key), iv, bytearray()
    for i in range(0, clen, 16):
        blk = data[i:i + 16]
        plain += bytes(a ^ b for a, b in zip(_dec_block(rk, blk), prev))
        prev = blk
    end = clen - pad
    if end <= 3:
        raise ValueError("too short")
    crc_lo, crc_hi, check = plain[end - 3], plain[end - 2], plain[end - 1]
    if want_check is not None:
        ok = check == want_check
    elif pad == 0:
        ok = check & 0xF == 8
    else:
        ok = check >> 4 == pad
    if not ok:
        raise ValueError("check byte mismatch")
    payload = bytes(plain[:end - 3])
    if crc16_ccitt(payload) != crc_lo | crc_hi << 8:
        raise ValueError("crc16 mismatch")
    return payload


def read_varint(buf: bytes, pos: int) -> tuple[int, int]:
    """Unsigned LEB128 (7 bits per byte, bit 7 = more), as in 0xc793e0 compression 1."""
    v = shift = 0
    while True:
        b = buf[pos]
        pos += 1
        v |= (b & 0x7F) << shift
        if b < 0x80:
            return v & 0xFFFFFFFF, pos
        shift += 7


def write_varint(v: int) -> bytes:
    out = bytearray()
    v &= 0xFFFFFFFF
    while True:
        if v < 0x80:
            out.append(v)
            return bytes(out)
        out.append((v & 0x7F) | 0x80)
        v >>= 7


def selftest() -> None:
    p = b"\x01\x02\x03 network message body"
    for junk in (0, 5, 12):
        assert msg_decrypt(msg_encrypt(p, junk=junk)) == p
    assert msg_decrypt(msg_encrypt(b"x" * 13)) == b"x" * 13  # pad == 0 case
    assert DEFAULT_MSG_KEY[0].hex() == "651287ec61c6fba0fdfacf04993e8308"
    assert crc16_ccitt(b"123456789") == 0x29B1
    assert read_varint(write_varint(300), 0) == (300, 2)
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
    elif sys.argv[1] == "packet":
        # One full datagram from the documented layout, for cross-checking another implementation.
        flags, serial, ack = (int(sys.argv[i], 0) for i in (2, 3, 4))
        mask, extra = (int(sys.argv[i], 0) for i in (5, 6))
        payload = bytes.fromhex(sys.argv[7]) if len(sys.argv) > 7 else b""
        print(pack(flags, serial, ack, mask, extra, payload, derive_keys()).hex())
    elif sys.argv[1] == "offset":
        for arg in sys.argv[2:]:
            print(f"{int(arg, 0):#010x} {_payload_mask_start(int(arg, 0)):#06x}")


if __name__ == "__main__":
    main()
