"""Dump the network message format table of arma3_x64.exe 2.22.0.154103 by emulation.

Runs the engine's own registration function (RVA 0xbc35a0, fills the format table at 0x21c4fb0
and the count at 0x21ccab4) under Unicorn (tools/re/emu.py), then reads every
NetworkMessageFormat: message type id = table index.

Usage (needs the .work venv: unicorn, pefile):
    P:/a3-rust/.work/venv/Scripts/python.exe tools/re/net_formats.py <arma3_x64.exe> [--tsv out.tsv]

TSV columns: id, format_rva, item_count, items. Each item is `type:compression`, then
`>N` for nested-message items (N = nested message type id) and `/e<errType>:<coef>` when the item
has error-type info (used for update priority). Data-type and compression numbers are explained
in docs/re/net-messages.md.
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from emu import Emu  # noqa: E402

REGISTER_RVA = 0xBC35A0
TABLE_RVA = 0x21C4FB0
COUNT_RVA = 0x21CCAB4

TYPE_NAMES = {
    0: "variant", 1: "optmsg", 2: "bool", 3: "int", 4: "int64", 5: "float", 6: "string",
    7: "rawdata", 8: "u32", 9: "vector", 10: "matrix", 11: "boolarr", 12: "intarr",
    13: "floatarr", 14: "stringarr", 15: "idstrarr", 16: "strfloatarr", 17: "msg", 18: "msgarr",
    19: "optmsgarr", 20: "netid", 21: "netidarr", 22: "arr22", 23: "blob36", 24: "arr16",
    25: "locstr", 26: "locstrarr", 27: "byte",
}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("exe")
    ap.add_argument("--tsv")
    a = ap.parse_args()
    e = Emu(a.exe)
    e.call(REGISTER_RVA)
    n = e.u32(e.base + COUNT_RVA)
    out = open(a.tsv, "w", encoding="utf-8", newline="\n") if a.tsv else sys.stdout
    out.write("id\tformat_rva\titem_count\titems\n")
    for i in range(n):
        f = e.u64(e.base + TABLE_RVA + 8 * i)
        if not f:
            out.write(f"{i}\t\t0\t\n")
            continue
        items_p, cnt = e.u64(f + 8), e.u32(f + 0x10)
        err_p, err_cnt = e.u64(f + 0x20), e.u32(f + 0x28)
        parts = []
        for k in range(cnt):
            t, c, dflt = struct.unpack("<IIQ", e.read(items_p + 0x18 * k, 16))
            s = f"{TYPE_NAMES.get(t, t)}:{c}"
            if t in (1, 0x11, 0x12, 0x13) and dflt:
                s += f">{e.u32(dflt)}"   # nested message type id
            if k < err_cnt:
                et, coef = struct.unpack("<If", e.read(err_p + 8 * k, 8))
                if et or coef:
                    s += f"/e{et}:{coef:g}"
            parts.append(s)
        rva = f - e.base if f >= e.base else f
        out.write(f"{i}\t0x{rva:x}\t{cnt}\t{' '.join(parts)}\n")
    if a.tsv:
        out.close()
        print(f"{n} message types -> {a.tsv}", file=sys.stderr)


if __name__ == "__main__":
    main()
