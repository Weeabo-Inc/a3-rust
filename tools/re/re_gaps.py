#!/usr/bin/env python3
"""List the open reverse-engineering items in docs/re/*.md.

An item is either
- a line that carries a confidence marker (medium/low confidence, assumed, stand-in, unknown,
  not decoded, not traced, open question, ...), or
- a bullet under an "Open questions" / "Open points" / "Open items" heading.

Struck-through bullets (`~~...~~`) count as resolved and are skipped. The output feeds the
ledger `docs/fidelity/re-gaps.md`; the script does not edit it.

    python tools/re/re_gaps.py                 # every item, grouped by doc
    python tools/re/re_gaps.py --summary       # item count per doc
    python tools/re/re_gaps.py --doc render-   # only docs whose name contains the text
    python tools/re/re_gaps.py --include-net   # also the net-*.md docs (out of scope by default)

Stdlib only.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

MARKER = re.compile(
    r"medium confidence|low confidence|confidence: *(?:medium|low)|\((?:medium|low)\b"
    r"|\b(?:medium|low)\)|_\((?:uncertain|assumed|guess)[^)]*\)_|\bassum(?:ed|ption)\b"
    r"|stand-in|\bguess|not (?:yet )?(?:decoded|traced|confirmed|verified|known|identified)"
    r"|\bunknown\b|\buncertain\b|\bunverified\b|\bTODO\b|open question",
    re.IGNORECASE,
)
OPEN_HEADING = re.compile(r"^#+\s*(?:\d+\.\s*)?open (?:questions|points|items|issues)", re.I)
HEADING = re.compile(r"^(#+)\s*(.*)")
BULLET = re.compile(r"^\s*(?:[-*]|\d+\.)\s+")

# Generated inventories and the deferred network track.
SKIP = {"data-inventory.md", "rtti-classes.md", "sqf-command-table.md", "TOOLING.md"}


def scan(path: Path):
    """Yield (line_no, section, text) for every open item in one doc."""
    section = ""
    in_open = False
    for no, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.rstrip()
        h = HEADING.match(line)
        if h:
            section = h.group(2).strip()
            in_open = bool(OPEN_HEADING.match(line))
            continue
        if not line.strip() or line.lstrip().startswith("```"):
            continue
        resolved = "~~" in line and line.strip().lstrip("-* ").startswith("~~")
        if resolved:
            continue
        if (in_open and BULLET.match(line)) or MARKER.search(line):
            yield no, section, line.strip()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--root", default=Path(__file__).resolve().parents[2] / "docs" / "re", type=Path)
    ap.add_argument("--summary", action="store_true", help="count per doc only")
    ap.add_argument("--doc", default="", help="filter docs by substring")
    ap.add_argument("--include-net", action="store_true", help="include net-*.md")
    ap.add_argument("--width", type=int, default=160, help="truncate item text (0 = no limit)")
    args = ap.parse_args()
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")

    total = 0
    for path in sorted(args.root.glob("*.md")):
        name = path.name
        if name in SKIP or args.doc not in name:
            continue
        if name.startswith("net-") and not args.include_net:
            continue
        items = list(scan(path))
        if not items:
            continue
        total += len(items)
        if args.summary:
            print(f"{len(items):4}  {name}")
            continue
        print(f"## {name} ({len(items)})")
        for no, section, text in items:
            if args.width and len(text) > args.width:
                text = text[: args.width - 3] + "..."
            print(f"  {no:5}  [{section[:40]}]  {text}")
        print()
    print(f"{total} open items", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
