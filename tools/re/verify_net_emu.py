"""Cross-check tools/re/a3net.py against the original code, executed under Unicorn (tools/re/emu.py).

    P:/a3-rust/.work/venv/Scripts/python.exe tools/re/verify_net_emu.py P:/a3-rust/oirignal/arma3_x64.exe

Every check calls the real function at its RVA (2.22.0.154103) and compares with the Python
transcription. Prints one line per check and exits non-zero on any mismatch.
"""

from __future__ import annotations

import os
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import a3net  # noqa: E402
from emu import Emu  # noqa: E402
from unicorn import UC_HOOK_CODE  # noqa: E402
from unicorn.x86_const import UC_X86_REG_R8, UC_X86_REG_RDX, UC_X86_REG_RIP, UC_X86_REG_RSP  # noqa: E402

results: list[tuple[str, bool]] = []


def check(name: str, ok: bool) -> None:
    results.append((name, ok))
    print(f"{'ok  ' if ok else 'FAIL'} {name}")


def main() -> int:
    e = Emu(sys.argv[1] if len(sys.argv) > 1 else "P:/a3-rust/oirignal/arma3_x64.exe")
    m = a3net.MAGIC

    # transport: PRNG seed / LCG / byte generator (hook CryptHashData wrapper 0x281150)
    captured: list[bytes] = []

    def hook(uc, addr, size, data):
        rdx, r8 = uc.reg_read(UC_X86_REG_RDX), uc.reg_read(UC_X86_REG_R8)
        captured.append(bytes(uc.mem_read(rdx, r8)))
        rsp = uc.reg_read(UC_X86_REG_RSP)
        uc.reg_write(UC_X86_REG_RIP, struct.unpack("<Q", uc.mem_read(rsp, 8))[0])
        uc.reg_write(UC_X86_REG_RSP, rsp + 8)

    h = e.uc.hook_add(UC_HOOK_CODE, hook, begin=e.base + 0x281150, end=e.base + 0x281150)
    for a, b in [((-0x50A5AA3 - m) & 0xFFFFFFFF, (m + 0x12CA1B) & 0xFFFFFFFF),
                 ((m + 0x8E3B8C) & 0xFFFFFFFF, (-0x29CD4BC6 - m) & 0xFFFFFFFF)]:
        captured.clear()
        e.call(0x2814A0, e.alloc(0x40), a, b, 0, 0)
        check(f"transport prng bytes ({a:#x},{b:#x}) 0x2814a0", captured[-1] == a3net.prng_bytes(a, b))
    e.uc.hook_del(h)

    keys = a3net.derive_keys()
    lo, hi = m & 0xFFFF, (m >> 16) & 0xFFFF
    tab = e.alloc(0x800)
    e.call(0x345D90, tab, ((lo ^ hi) & 0x5555) ^ hi)
    check("transport payload LFSR table 0x345d90", e.read(tab, 0x800) == keys.payload_table)

    key = os.urandom(20)
    d = e.alloc(16)
    e.call(0x352FC0, d, 16, e.alloc(20, key), 20, 0x200)
    check("RC4-drop512 0x352fc0", e.read(d, 16) == a3net.rc4_drop(key, 16))

    peer = e.alloc(0x200)
    e.write(peer + 0x60, struct.pack("<Q", tab))
    pkt = bytearray(os.urandom(80))
    struct.pack_into("<H", pkt, 0, 80)
    dst = e.alloc(96, bytes(pkt))
    e.call(0x352A20, peer, dst, dst)
    py = bytearray(pkt)
    a3net._xor_payload(py, keys)
    check("transport payload XOR 0x352a20", e.read(dst, 80) == bytes(py))

    # message layer: key derivation, CRC16, AES-CBC
    for tok in (bytes(16), os.urandom(32), os.urandom(32)):
        tp, o1, o2, ctx = e.alloc(len(tok), tok), e.alloc(16), e.alloc(16), e.alloc(0x200)
        e.call(0x7B4C40, 0, tp, len(tok), o1)
        e.call(0x7B4D00, 0, tp, len(tok), o2)
        e.call(0x272ED0, ctx, o1, o2, 0)
        c = e.read(ctx, 0x30)
        emu_key = bytes(x ^ y for x, y in zip(c[0x10:0x20], c[0x20:0x30]))
        check(f"msg key/iv from {len(tok)}-byte token", (emu_key, c[:16]) == a3net.msg_key_from_token(tok))
        body = os.urandom(48)
        buf = e.alloc(48, body)
        e.call(0x270B80, e.alloc(0x200, e.read(ctx, 0x200)), buf, buf, 48)
        k, iv = a3net.msg_key_from_token(tok)
        rk, prev, out = a3net._expand(k), iv, bytearray()
        for i in range(0, 48, 16):
            prev = a3net._enc_block(rk, bytes(x ^ y for x, y in zip(body[i:i + 16], prev)))
            out += prev
        check(f"msg AES-128-CBC encrypt 0x270b80 ({len(tok)}-byte token)", e.read(buf, 48) == bytes(out))
    data = os.urandom(77)
    dp = e.alloc(77, data)
    crc = e.call(0x281CA0, 0xFFFF, dp, 77) & 0xFFFF
    check("CRC16 0x281ca0", crc == a3net.crc16_ccitt(data))

    bad = [n for n, ok in results if not ok]
    print(f"{len(results) - len(bad)}/{len(results)} checks passed")
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
