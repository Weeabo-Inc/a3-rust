"""Shell CLI for reverse-engineering queries against the original Arma 3 binaries.

Backend: the shared GhidraMCP headless server (bethington/ghidra-mcp) on http://127.0.0.1:8089,
started with `tools/re/ghidra-mcp-server.ps1 start`. Offline commands (rtti, sqf) read the
committed TSVs in docs/re/ and need no server. Only the Python standard library is used.

Addresses: a full VA (0x1402e2380) or an RVA (0x2e2380); values below the image base get
0x140000000 added. Names are Ghidra symbol names (FUN_..., or names agents assigned).

Commands:
    decompile <addr|name>          pseudo-C of the containing function
    disasm <addr|name>             disassembly of the function
    func-at <addr>                 function containing addr: name, range, callers, callees
    xrefs <addr|name> [--limit N]  references to an address or function
    callees <addr|name>            functions called by a function
    strings <text> [--limit N]     defined strings containing text (case-insensitive)
    read <addr> [len]              hex dump of memory
    rtti <class-regex>             RTTI classes (offline: docs/re/rtti-classes.tsv)
    vtable <class>                 vtable slots of a class with function names
    sqf <name-regex>               script commands (offline: docs/re/sqf-commands.tsv)
    rename <addr> <new-name>       rename the function at addr (saved to the shared project)
    label <addr> <name>            name a data/code address (global symbol)
    comment <addr> <text>          set a plate comment at addr
    save                           save the program in the shared project
    call <endpoint> [k=v ...]      raw GhidraMCP endpoint (GET; add --post for POST)
    status                         server health

Options: --program NAME (default arma3_x64.exe), --json (raw JSON output), --url URL.
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DOCS = ROOT / "docs" / "re"
BASE = 0x140000000
DEFAULT_URL = os.environ.get("A3_GHIDRA_MCP_URL", "http://127.0.0.1:8089")


class Server:
    def __init__(self, url: str, program: str):
        self.url = url.rstrip("/")
        self.program = program

    def get(self, path: str, **params):
        params = {k: v for k, v in params.items() if v is not None}
        params.setdefault("program", self.program)
        q = urllib.parse.urlencode(params)
        return self._open(urllib.request.Request(f"{self.url}/{path.lstrip('/')}?{q}"))

    def post(self, path: str, **body):
        body = {k: v for k, v in body.items() if v is not None}
        body.setdefault("program", self.program)
        req = urllib.request.Request(f"{self.url}/{path.lstrip('/')}", data=json.dumps(body).encode(),
                                     headers={"Content-Type": "application/json"}, method="POST")
        return self._open(req)

    def _open(self, req):
        try:
            with urllib.request.urlopen(req, timeout=600) as r:
                raw = r.read().decode("utf-8", errors="replace")
        except urllib.error.HTTPError as e:
            raw = e.read().decode("utf-8", errors="replace")
        except urllib.error.URLError as e:
            sys.exit(f"GhidraMCP server not reachable at {self.url} ({e.reason}).\n"
                     f"Start it: powershell -File tools/re/ghidra-mcp-server.ps1 start")
        try:
            return json.loads(raw)
        except json.JSONDecodeError:
            return {"text": raw}


def norm_addr(s: str) -> str:
    """Return a VA hex string for an address argument, or the argument unchanged if a name."""
    t = s.lower().removeprefix("rva:")
    if re.fullmatch(r"(0x)?[0-9a-f]+", t) and (t.startswith("0x") or len(t) >= 6):
        v = int(t, 16)
        if v < BASE:
            v += BASE
        return f"0x{v:x}"
    return s


def emit(obj, as_json: bool, key: str | None = None):
    if as_json or key is None:
        print(json.dumps(obj, indent=1))
        return
    v = obj.get(key) if isinstance(obj, dict) else None
    if v is None:
        print(json.dumps(obj, indent=1))
    else:
        print(v.replace("\r\n", "\n") if isinstance(v, str) else json.dumps(v, indent=1))


def read_tsv(name: str):
    p = DOCS / name
    if not p.exists():
        sys.exit(f"{p} missing")
    with open(p, encoding="utf-8", newline="") as f:
        return list(csv.DictReader(f, delimiter="\t", quoting=csv.QUOTE_NONE))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd")
    ap.add_argument("args", nargs="*")
    ap.add_argument("--program", default=os.environ.get("A3_RE_PROGRAM", "arma3_x64.exe"))
    ap.add_argument("--url", default=DEFAULT_URL)
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--limit", type=int, default=50)
    ap.add_argument("--post", action="store_true")
    a = ap.parse_args()
    s = Server(a.url, a.program)
    c, args = a.cmd, a.args

    if c == "status":
        emit(s.get("check_connection"), True)
    elif c == "decompile":
        emit(s.get("force_decompile", function=norm_addr(args[0])), a.json, "decompiled")
    elif c == "disasm":
        r = s.get("disassemble_function", address=norm_addr(args[0]))
        if a.json or "instructions" not in r:
            emit(r, True)
        else:
            for i in r["instructions"]:
                print(f"{i.get('address')}: {i.get('instruction') or i.get('text') or i}")
    elif c == "func-at":
        r = s.get("get_functions", function=norm_addr(args[0]))
        if a.json:
            emit(r, True)
        else:
            for k in ("name", "address", "body_end", "size", "signature", "caller_count", "callee_count"):
                print(f"{k}: {r.get(k)}")
            print("callers:", ", ".join(f"{x['name']}@{x['address']}" for x in r.get("callers", [])[:20]))
            print("callees:", ", ".join(f"{x['name']}@{x['address']}" for x in r.get("callees", [])[:40]))
    elif c == "xrefs":
        r = s.get("get_xrefs_to", address=norm_addr(args[0]), limit=a.limit)
        if a.json or "references" not in r:
            emit(r, True)
        else:
            for x in r["references"]:
                print(f"{x.get('from_address')}\t{x.get('type')}\t{x.get('from_function', '')}")
            print(f"({r.get('total')} total)", file=sys.stderr)
    elif c == "callees":
        r = s.get("get_functions", function=norm_addr(args[0]))
        for x in r.get("callees", []):
            print(f"{x['address']}\t{x['name']}")
    elif c == "strings":
        r = s.get("search_strings", search_term=" ".join(args), limit=a.limit)
        if a.json or "matches" not in r:
            emit(r, True)
        else:
            for m in r["matches"]:
                print(f"{m['address']}\t{m['value']}")
            print(f"({r.get('total')} total)", file=sys.stderr)
    elif c == "read":
        n = int(args[1], 0) if len(args) > 1 else 64
        emit(s.get("read_memory", address=norm_addr(args[0]), length=n), a.json, "hex" if not a.json else None)
    elif c == "rtti":
        rx = re.compile(args[0] if args else ".", re.I)
        for row in read_tsv("rtti-classes.tsv"):
            if rx.search(row["class"]):
                print(f"{row['class']}\tvtables={row['vtable_rvas']}\tbases={row['direct_bases']}")
    elif c == "vtable":
        rows = [r for r in read_tsv("rtti-classes.tsv") if r["class"] == args[0]]
        if not rows or not rows[0]["vtable_rvas"]:
            sys.exit(f"no vtable for {args[0]!r} (try: re.py rtti '{args[0]}')")
        first = rows[0]["vtable_rvas"].split(",")[0]
        slots = int(rows[0]["vtable_slots"].split(",")[0])
        vt = int(first.split("@")[0], 16)
        r = s.get("read_memory", address=f"0x{BASE + vt:x}", length=8 * slots)
        hexs = (r.get("hex") or r.get("data") or "").replace(" ", "")
        if not hexs:
            emit(r, True)
            return
        raw = bytes.fromhex(hexs)
        print(f"{args[0]} primary vtable at 0x{BASE + vt:x} (rva 0x{vt:x}), {slots} slots"
              + (f" (showing {a.limit}; use --limit)" if slots > a.limit else ""))
        ptrs = [int.from_bytes(raw[8 * i:8 * i + 8], "little") for i in range(min(slots, a.limit))]
        names = {}
        for chunk in range(0, len(ptrs), 100):
            part = ptrs[chunk:chunk + 100]
            r = s.get("get_functions", functions=",".join(f"0x{p:x}" for p in part), fields="entry_point")
            for k, v in (r.get("functions") or {}).items():
                names[int(k, 16)] = v.get("name", "?")
        for i, ptr in enumerate(ptrs):
            print(f"  [{i:3}] +0x{8 * i:03x}  0x{ptr:x}  {names.get(ptr, '?')}")
    elif c == "sqf":
        rx = re.compile(args[0] if args else ".", re.I)
        for row in read_tsv("sqf-commands.tsv"):
            if rx.search(row["name"]):
                sig = {"nular": f"{row['name']}",
                       "unary": f"{row['name']} {row['right_type']}",
                       "binary": f"{row['left_type']} {row['name']} {row['right_type']}"}[row["kind"]]
                print(f"{sig} -> {row['return_type']}\thandler={row['handler']}\t{row['kind']}")
    elif c == "rename":
        emit(s.post("rename_function", function_address=norm_addr(args[0]), new_name=args[1]), True)
        s.get("save_program")
    elif c == "label":
        emit(s.post("rename_symbol", address=norm_addr(args[0]), new_name=args[1]), True)
        s.get("save_program")
    elif c == "comment":
        emit(s.post("set_comment", address=norm_addr(args[0]), comment=" ".join(args[1:]), type="plate"), True)
        s.get("save_program")
    elif c == "save":
        emit(s.get("save_program"), True)
    elif c == "call":
        kv = dict(x.split("=", 1) for x in args[1:])
        emit(s.post(args[0], **kv) if a.post else s.get(args[0], **kv), True)
    else:
        sys.exit(f"unknown command {c!r}; see --help")


if __name__ == "__main__":
    main()
