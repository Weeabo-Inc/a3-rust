"""Extract the MSVC x64 RTTI class hierarchy from a PE file, without Ghidra.

Usage:
    python tools/re/rtti_dump.py <exe> [--tsv out.tsv]

Walks every CompleteObjectLocator (COL) in the image, follows it to the
TypeDescriptor and ClassHierarchyDescriptor, and finds the vtable(s) that point
at each COL. Type descriptors without a COL (classes that have RTTI only for
`typeid`/exceptions) are listed with an empty vtable column.

Output TSV columns (addresses are RVAs; add the image base 0x140000000 for VAs):
    class  mangled  vtable_rvas  vtable_slots  direct_bases  all_bases  flags
"""

from __future__ import annotations

import argparse
import re
import struct
import sys
from collections import defaultdict
from pathlib import Path

import pefile

sys.path.insert(0, str(Path(__file__).resolve().parent))
from msvc_demangle import demangle_type_name  # noqa: E402


class Image:
    def __init__(self, path: str):
        self.pe = pefile.PE(path, fast_load=True)
        self.base = self.pe.OPTIONAL_HEADER.ImageBase
        self.mem = self.pe.get_memory_mapped_image()
        self.sections = {
            s.Name.rstrip(b"\0").decode(): (s.VirtualAddress, s.VirtualAddress + max(s.Misc_VirtualSize, s.SizeOfRawData))
            for s in self.pe.sections
        }

    def u32(self, rva: int) -> int:
        return struct.unpack_from("<I", self.mem, rva)[0]

    def i32(self, rva: int) -> int:
        return struct.unpack_from("<i", self.mem, rva)[0]

    def u64(self, rva: int) -> int:
        return struct.unpack_from("<Q", self.mem, rva)[0]

    def cstr(self, rva: int, limit: int = 4096) -> str:
        end = self.mem.find(b"\0", rva, rva + limit)
        return self.mem[rva:end].decode("latin1")

    def in_section(self, rva: int, name: str) -> bool:
        lo, hi = self.sections.get(name, (0, 0))
        return lo <= rva < hi


def scan(img: Image):
    rdata_lo, rdata_hi = img.sections[".rdata"]
    data_lo, data_hi = img.sections[".data"]
    text_lo, text_hi = img.sections[".text"]

    # Type descriptors live in .data: {pVFTable, spare, ".?AV...\0"}
    tds: dict[int, str] = {}
    for m in re.finditer(rb"\.\?A[VUW][^\0]{1,2000}\0", img.mem[data_lo:data_hi]):
        name_rva = data_lo + m.start()
        td_rva = name_rva - 16
        if td_rva % 8:
            continue
        tds[td_rva] = m.group()[:-1].decode("latin1")

    def bcd(rva: int):
        td, ncontained, mdisp, pdisp, vdisp, attrs, _chd = struct.unpack_from("<IIiiiII", img.mem, rva)
        return td, ncontained, mdisp, pdisp, vdisp, attrs

    def hierarchy(chd_rva: int):
        _sig, attrs, nbases, arr = struct.unpack_from("<IIII", img.mem, chd_rva)
        bases = [bcd(img.u32(arr + 4 * i)) for i in range(nbases)]
        return attrs, bases

    # Complete object locators in .rdata: sig=1 and pSelf == own RVA
    cols: dict[int, tuple] = {}
    mv = memoryview(img.mem)
    for rva in range(rdata_lo, rdata_hi - 24, 4):
        if img.u32(rva) != 1:
            continue
        sig, off, cdoff, td, chd, self_rva = struct.unpack_from("<IIIIII", mv, rva)
        if self_rva != rva or td not in tds:
            continue
        cols[rva] = (off, cdoff, td, chd)

    # vtables: a qword in .rdata equal to base+COL; vtable starts right after it
    col_vas = {img.base + c: c for c in cols}
    vtables: dict[int, list[tuple[int, int, int]]] = defaultdict(list)  # td -> [(vt_rva, offset, slots)]
    for rva in range(rdata_lo, rdata_hi - 8, 8):
        q = img.u64(rva)
        c = col_vas.get(q)
        if c is None:
            continue
        vt = rva + 8
        slots = 0
        while vt + 8 * slots < rdata_hi:
            p = img.u64(vt + 8 * slots) - img.base
            if not (text_lo <= p < text_hi):
                break
            slots += 1
        off, _cd, td, _chd = cols[c]
        vtables[td].append((vt, off, slots))

    classes = {}
    for col_rva, (off, cdoff, td, chd) in cols.items():
        if td in classes and off != 0:
            continue
        attrs, bases = hierarchy(chd)
        names = [tds.get(b[0], f"?td@{b[0]:x}") for b in bases]
        # direct bases: walk the flattened depth-first array
        direct = []
        i = 1
        while i < len(bases):
            direct.append(names[i])
            i += 1 + bases[i][1]
        classes[td] = {
            "mangled": tds[td],
            "attrs": attrs,
            "direct": direct,
            "all": names[1:],
        }
    for td, name in tds.items():
        classes.setdefault(td, {"mangled": name, "attrs": None, "direct": [], "all": []})
    return classes, vtables, tds


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("exe")
    ap.add_argument("--tsv", help="write TSV here (default: stdout)")
    a = ap.parse_args()

    img = Image(a.exe)
    classes, vtables, tds = scan(img)
    out = open(a.tsv, "w", encoding="utf-8", newline="\n") if a.tsv else sys.stdout
    out.write("class\tmangled\tvtable_rvas\tvtable_slots\tdirect_bases\tall_bases\tflags\n")
    rows = []
    for td, c in classes.items():
        vts = sorted(vtables.get(td, []), key=lambda v: v[1])
        flags = []
        if c["attrs"] is not None:
            if c["attrs"] & 1:
                flags.append("MI")
            if c["attrs"] & 2:
                flags.append("VI")
        else:
            flags.append("no-col")
        rows.append((
            demangle_type_name(c["mangled"]),
            c["mangled"],
            ",".join(f"0x{v[0]:x}" + (f"@{v[1]}" if v[1] else "") for v in vts),
            ",".join(str(v[2]) for v in vts),
            ";".join(demangle_type_name(n) for n in c["direct"]),
            ";".join(demangle_type_name(n) for n in c["all"]),
            ",".join(flags),
        ))
    rows.sort(key=lambda r: r[0].lower())
    for r in rows:
        out.write("\t".join(r) + "\n")
    if a.tsv:
        out.close()
        print(f"{len(rows)} classes, {sum(1 for r in rows if r[2])} with vtables", file=sys.stderr)


if __name__ == "__main__":
    main()
