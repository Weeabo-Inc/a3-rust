"""Extract the SQF script command table from arma3_x64.exe (build 2.22.0.154103).

Usage:
    python tools/re/sqf_commands.py <arma3_x64.exe> [--tsv out.tsv] [--types out.tsv]

How it works (see docs/re/sqf-command-table.md):
  The engine registers every script command in large static-initialiser functions that build
  descriptor objects on the stack and call one of three constructors:

    nular   0x2c86c0  (rcx=this, rdx=ret type, r8=name, r9=handler, stack: description, ...)
    unary   0x2c8050  (rcx=this, rdx=ret type, r8=name, r9=handler, [rsp+20h]=right type, ...)
    binary  0x2c8a20  (rcx=this, rdx=ret type, r8=name, r9d=priority, [rsp+20h]=handler,
                       [rsp+28h]=left type, [rsp+30h]=right type, ...)

  Type objects (GameType) are static globals built by:
    0x2c9060  GameType(rcx=this, rdx=name, ...)    basic type with a name such as "SCALAR"
    0x2c9030  GameType(rcx=this, rdx=other)        copy
    0x2cbf20  GameType::operator|(rcx=a, rdx=result, r8=b)   union a|b

  The script disassembles every function listed in .pdata, tracks `lea reg,[rip+x]`,
  register moves and stores to [rsp+N], and records the argument values at each constructor
  call. Addresses are RVAs (image base 0x140000000).
"""

from __future__ import annotations

import argparse
import re
import struct
import sys
from collections import Counter

import capstone
import pefile

NULAR, UNARY, BINARY = 0x2C86C0, 0x2C8050, 0x2C8A20
TYPE_NAMED, TYPE_COPY, TYPE_UNION = 0x2C9060, 0x2C9030, 0x2CBF20

# stack slot layout per constructor: offset -> field
NULAR_SLOTS = {0x20: "description", 0x28: "example", 0x30: "example_result", 0x38: "since",
               0x40: "changed", 0x48: "category"}
UNARY_SLOTS = {0x20: "right_type", 0x28: "right_name", 0x30: "description", 0x38: "example",
               0x40: "example_result", 0x48: "since", 0x50: "changed", 0x58: "category"}
BINARY_SLOTS = {0x20: "handler", 0x28: "left_type", 0x30: "right_type", 0x38: "left_name",
                0x40: "right_name", 0x48: "description", 0x50: "example", 0x58: "example_result",
                0x60: "since", 0x68: "changed", 0x70: "category"}

REGS64 = {"rax", "rbx", "rcx", "rdx", "rsi", "rdi", "rbp", "r8", "r9", "r10", "r11", "r12", "r13",
          "r14", "r15"}
REG32_TO_64 = {"eax": "rax", "ebx": "rbx", "ecx": "rcx", "edx": "rdx", "esi": "rsi", "edi": "rdi",
               "ebp": "rbp", **{f"r{i}d": f"r{i}" for i in range(8, 16)}}

RIP_RE = re.compile(r"\[rip ([+-]) (0x[0-9a-f]+)\]")
RSP_RE = re.compile(r"^qword ptr \[rsp \+ (0x[0-9a-f]+)\], (\w+)$")
RSP_DW_RE = re.compile(r"^dword ptr \[rsp \+ (0x[0-9a-f]+)\], (\w+)$")


class Pe:
    def __init__(self, path: str):
        self.pe = pefile.PE(path, fast_load=True)
        self.pe.parse_data_directories(directories=[pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_EXCEPTION"]])
        self.mem = self.pe.get_memory_mapped_image()
        self.base = self.pe.OPTIONAL_HEADER.ImageBase

    def functions(self):
        seen = set()
        for e in self.pe.DIRECTORY_ENTRY_EXCEPTION:
            b, end = e.struct.BeginAddress, e.struct.EndAddress
            if b not in seen:
                seen.add(b)
                yield b, end

    def cstr(self, rva):
        if not rva or not (0 < rva < len(self.mem)):
            return None
        end = self.mem.find(b"\0", rva, rva + 4096)
        raw = self.mem[rva:end]
        try:
            s = raw.decode("utf-8")
        except UnicodeDecodeError:
            return None
        if raw and not all(c >= 32 or c in (9, 10, 13) for c in raw):
            return None
        return s


def sweep(pe: Pe):
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    calls = []  # (kind, site, regs, slots)
    targets = {NULAR, UNARY, BINARY, TYPE_NAMED, TYPE_COPY, TYPE_UNION}
    # pre-filter: only functions containing a rel32 call to one of the targets
    text = pe.mem
    for begin, end in pe.functions():
        chunk = text[begin:end]
        hit = False
        idx = chunk.find(b"\xe8")
        while idx != -1:
            if idx + 5 <= len(chunk):
                rel = struct.unpack_from("<i", chunk, idx + 1)[0]
                if begin + idx + 5 + rel in targets:
                    hit = True
                    break
            idx = chunk.find(b"\xe8", idx + 1)
        if not hit:
            continue
        regs: dict[str, int | None] = {}
        slots: dict[int, int | None] = {}
        for addr, size, mnem, ops in md.disasm_lite(bytes(chunk), begin):
            if mnem == "lea":
                dst, _, src = ops.partition(", ")
                m = RIP_RE.search(src)
                if m and dst in REGS64:
                    d = int(m.group(2), 16) * (1 if m.group(1) == "+" else -1)
                    regs[dst] = addr + size + d
                elif dst in REGS64:
                    regs[dst] = None
            elif mnem == "mov":
                m = RSP_RE.match(ops)
                if m:
                    off, src = int(m.group(1), 16), m.group(2)
                    if src in REGS64:
                        slots[off] = regs.get(src)
                    else:
                        try:
                            slots[off] = int(src, 16) if src.startswith("0x") else int(src)
                        except ValueError:
                            slots[off] = None
                    continue
                m = RSP_DW_RE.match(ops)
                if m:
                    off, src = int(m.group(1), 16), m.group(2)
                    try:
                        slots[off] = int(src, 16) if src.startswith("0x") else int(src)
                    except ValueError:
                        slots[off] = None
                    continue
                dst, _, src = ops.partition(", ")
                dst64 = REG32_TO_64.get(dst, dst)
                if dst64 in REGS64:
                    src64 = REG32_TO_64.get(src, src)
                    if src64 in REGS64:
                        regs[dst64] = regs.get(src64)
                    else:
                        try:
                            regs[dst64] = int(src, 16) if src.startswith("0x") else int(src)
                        except ValueError:
                            regs[dst64] = None
            elif mnem == "xor":
                a, _, b = ops.partition(", ")
                if a == b:
                    regs[REG32_TO_64.get(a, a)] = 0
            elif mnem == "call":
                try:
                    tgt = int(ops, 16)  # disassembled at RVA addresses
                except ValueError:
                    tgt = None
                if tgt in targets:
                    calls.append((tgt, addr, dict(regs), dict(slots)))
                for r in ("rax", "rcx", "rdx", "r8", "r9", "r10", "r11"):
                    regs.pop(r, None)
    return calls


def build_types(pe: Pe, calls):
    named, copies, unions = {}, {}, {}
    for kind, _site, regs, _slots in calls:
        this = regs.get("rcx")
        if kind == TYPE_NAMED:
            named[this] = pe.cstr(regs.get("rdx")) or f"?{regs.get('rdx')}"
        elif kind == TYPE_COPY:
            copies[this] = regs.get("rdx")
        elif kind == TYPE_UNION:
            # member operator|: rcx = left operand, rdx = hidden result pointer, r8 = right operand
            unions[regs.get("rdx")] = (this, regs.get("r8"))

    cache: dict[int, str] = {}

    def resolve(t, depth=0):
        if t is None:
            return "?"
        if t in cache:
            return cache[t]
        if depth > 20:
            return f"?{t:#x}"
        if t in named:
            r = named[t]
        elif t in copies:
            r = resolve(copies[t], depth + 1)
        elif t in unions:
            a, b = unions[t]
            parts = resolve(a, depth + 1).split("|") + resolve(b, depth + 1).split("|")
            r = "|".join(sorted(set(parts), key=parts.index))
        else:
            r = f"?{t:#x}"
        cache[t] = r
        return r

    return resolve, named, copies, unions


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("exe")
    ap.add_argument("--tsv", help="command table output (default stdout)")
    ap.add_argument("--types", help="type table output")
    ap.add_argument("--long", action="store_true", help="include description/example columns")
    a = ap.parse_args()

    pe = Pe(a.exe)
    calls = sweep(pe)
    resolve, named, copies, unions = build_types(pe, calls)

    rows = []
    for kind, site, regs, slots in calls:
        if kind not in (NULAR, UNARY, BINARY):
            continue
        layout = {NULAR: NULAR_SLOTS, UNARY: UNARY_SLOTS, BINARY: BINARY_SLOTS}[kind]
        f = {name: slots.get(off) for off, name in layout.items()}
        rec = {
            "name": pe.cstr(regs.get("r8")) or "?",
            "kind": {NULAR: "nular", UNARY: "unary", BINARY: "binary"}[kind],
            "left_type": resolve(f.get("left_type")) if kind == BINARY else "",
            "right_type": resolve(f.get("right_type")) if kind != NULAR else "",
            "return_type": resolve(regs.get("rdx")),
            "handler": f.get("handler") if kind == BINARY else regs.get("r9"),
            "priority": regs.get("r9") if kind == BINARY else "",
            "site": site,
        }
        for k in ("left_name", "right_name", "description", "example", "example_result", "since",
                  "changed", "category"):
            v = f.get(k)
            rec[k] = (pe.cstr(v) if isinstance(v, int) and v > 0x1000 else "") or ""
        rows.append(rec)

    cols = ["name", "kind", "left_type", "right_type", "return_type", "handler", "priority",
            "category", "since", "left_name", "right_name", "site"]
    if a.long:
        cols += ["description", "example", "example_result", "changed"]
    out = open(a.tsv, "w", encoding="utf-8", newline="\n") if a.tsv else sys.stdout
    out.write("\t".join(cols) + "\n")
    rows.sort(key=lambda r: (r["name"].lower(), r["kind"], r["left_type"], r["right_type"]))
    for r in rows:
        vals = []
        for c in cols:
            v = r[c]
            if c in ("handler", "site") and isinstance(v, int):
                v = f"0x{v:x}"
            vals.append(str(v).replace("\t", " ").replace("\r", " ").replace("\n", "\\n"))
        out.write("\t".join(vals) + "\n")
    if a.tsv:
        out.close()
    if a.types:
        with open(a.types, "w", encoding="utf-8", newline="\n") as t:
            t.write("type_rva\tkind\tresolved\n")
            for addr, n in sorted(named.items(), key=lambda x: x[0] or 0):
                t.write(f"0x{addr or 0:x}\tnamed\t{n}\n")
            for addr in sorted(set(copies) | set(unions), key=lambda x: x or 0):
                t.write(f"0x{addr or 0:x}\t{'copy' if addr in copies else 'union'}\t{resolve(addr)}\n")
    c = Counter(r["kind"] for r in rows)
    print(f"{len(rows)} command overloads: {dict(c)}; {len({r['name'].lower() for r in rows})} distinct names; "
          f"{len(named)} named types", file=sys.stderr)


if __name__ == "__main__":
    main()
