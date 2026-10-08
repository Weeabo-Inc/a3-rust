"""List, extract and disassemble Arma 3 shader caches (Dta/bin.pbo: Shaders_<model>_<stage>.shdc).

Usage:
    python tools/re/shdc.py list <file.shdc> [name-regex]
    python tools/re/shdc.py dump <file.shdc> <name-regex> <outdir>     # .dxbc + .asm per match

File layout ("BIShaderCache-id: 0007"): a header/index, then one record per compiled shader:
    u32 0xAA55AA55, u32 0, u32 key,
    cstr compiler ("D3DX11CompileShader"), cstr entry name (e.g. "PSTerrain15"),
    cstr profile ("ps_5_0"), cstr variant hash (8 hex digits),
    u32 0x11222211, u32 0x00BBBB00, u32 0x00CCCC00, u32 size, DXBC blob[size]
Disassembly uses D3DDisassemble from the Windows system d3dcompiler_47.dll (not a game file).
"""

from __future__ import annotations

import ctypes
import re
import struct
import sys
from pathlib import Path

SEP = struct.pack("<III", 0x11222211, 0x00BBBB00, 0x00CCCC00)


COMPILER = b"D3DX11CompileShader\0"


def records(data: bytes):
    pos = data.find(COMPILER)
    while pos != -1:
        try:
            p = pos + len(COMPILER)
            strs = []
            for _ in range(3):
                e = data.index(b"\0", p)
                strs.append(data[p:e].decode("latin1"))
                p = e + 1
            key = struct.unpack_from("<I", data, pos - 4)[0]
            if data[p:p + 12] == SEP:
                size = struct.unpack_from("<I", data, p + 12)[0]
                blob_off = p + 16
                if data[blob_off:blob_off + 4] == b"DXBC":
                    yield {"key": key, "compiler": "D3DX11CompileShader", "name": strs[0],
                           "profile": strs[1], "variant": strs[2], "offset": blob_off, "size": size}
        except (ValueError, struct.error):
            pass
        pos = data.find(COMPILER, pos + 4)


def disassemble(blob: bytes) -> str:
    d3d = ctypes.WinDLL("d3dcompiler_47.dll")
    out = ctypes.c_void_p()
    hr = d3d.D3DDisassemble(blob, len(blob), 0, None, ctypes.byref(out))
    if hr != 0:
        return f"; D3DDisassemble failed hr=0x{hr & 0xFFFFFFFF:08x}\n"

    # ID3DBlob vtable: 2 = Release, 3 = GetBufferPointer, 4 = GetBufferSize
    vtbl = ctypes.cast(ctypes.cast(out, ctypes.POINTER(ctypes.c_void_p))[0], ctypes.POINTER(ctypes.c_void_p))
    get_ptr = ctypes.WINFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p)(vtbl[3])
    get_size = ctypes.WINFUNCTYPE(ctypes.c_size_t, ctypes.c_void_p)(vtbl[4])
    release = ctypes.WINFUNCTYPE(ctypes.c_ulong, ctypes.c_void_p)(vtbl[2])
    text = ctypes.string_at(get_ptr(out), get_size(out)).decode("latin1").rstrip("\0")
    release(out)
    return text


def main():
    cmd, path = sys.argv[1], sys.argv[2]
    data = Path(path).read_bytes()
    rx = re.compile(sys.argv[3]) if len(sys.argv) > 3 else None
    recs = [r for r in records(data) if not rx or rx.search(r["name"])]
    if cmd == "list":
        for r in recs:
            print(f"{r['name']}\t{r['profile']}\t{r['variant']}\t0x{r['key']:08x}\t{r['size']}")
        print(f"({len(recs)} shaders)", file=sys.stderr)
    elif cmd == "dump":
        out = Path(sys.argv[4])
        out.mkdir(parents=True, exist_ok=True)
        for r in recs:
            blob = data[r["offset"]:r["offset"] + r["size"]]
            stem = f"{r['name']}_{r['variant']}"
            (out / f"{stem}.dxbc").write_bytes(blob)
            (out / f"{stem}.asm").write_text(disassemble(blob), encoding="utf-8")
        print(f"dumped {len(recs)} shaders to {out}", file=sys.stderr)


if __name__ == "__main__":
    main()
