#!/usr/bin/env python3
"""Oracle probe: a config whose `#include` target does not exist (#348).

`a3\\missions_f_oldman\\missions\\repro_objectsimulationloadgame.tanoa\\description.ext` includes
two files that exist nowhere in the 2.22 install:

    \\a3\\Missions_F_Oldman\\Systems\\UI\\Sleeping\\RscTestControlTypes.inc
    \\a3\\Missions_F_Oldman\\Systems\\UI\\Sleeping\\RscRestUI.inc

The question is what the original does about it: fail the config, or log the missing file and load
the rest? This probe answers it on `arma3server_x64.exe`, and is run by hand (it writes missions,
runs the server, reads the RPTs); it is not part of the `.probes` differential corpus, because the
answer changes what our preprocessor does, not what a script sees.

    usage: python tools/oracle/probes/missing_include.py [--game-dir <install>]

It runs three one-mission servers, one after the other, each with a different `description.ext`:

| case | `description.ext` | what it settles |
|---|---|---|
| `control` | `class Header`, `class A3ROBefore { value = 11; }`, `class A3ROAfter { value = 22; }` | that the driver reads `missionConfigFile` at all |
| `include` | the same three, with a missing `#include` between the two classes | the #348 case |
| `syntax` | the same start, with an unterminated class | whether a config failure of another kind is fatal too |

The driver also loads the shipped `description.ext` of the Mission from #348 with `loadConfig`, and
each case records whether the engine started the mission.

The `initServer.sqf` driver logs one `A3RO` line per question; the RPTs are left at
`<work>/oracle/probes/missing_include/<case>.rpt`. Nothing is committed except this script and the
lines quoted in `docs/re/config.md`.
"""

from __future__ import annotations

import argparse
import os
import secrets
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))

import oracle  # noqa: E402  (the sibling tool; this probe reuses its mission/server scaffolding)

WORLD = "VR"
MISSION_PREFIX = "a3ro_probe"

HEADER = 'class Header\n{\n\tgameType = "Sandbox";\n};\n'
BEFORE = "class A3ROBefore\n{\n\tvalue = 11;\n};\n"
AFTER = "class A3ROAfter\n{\n\tvalue = 22;\n};\n"

# The shipped Mission from #348, verbatim; its own two missing includes are inside it.
SHIPPED_DESCRIPTION = (
    "\\a3\\Missions_F_Oldman\\Missions\\REPRO_objectSimulationLoadGame.Tanoa\\description.ext"
)

CASES = {
    "control": HEADER + BEFORE + AFTER,
    "include": (
        HEADER
        + BEFORE
        + '// a missing #include, as in the Mission from #348\n'
        + '#include "\\a3ro_probe\\does_not_exist.inc"\n'
        + '#include "a3ro_missing_local.inc"\n'
        + AFTER
    ),
    "syntax": HEADER + BEFORE + "class A3ROBroken\n{\n\tvalue = 33;\n",
}

INIT_SERVER = """// Written by tools/oracle/probes/missing_include.py (#348).
A3RO_report = {
    diag_log "A3RO_BEGIN";
    diag_log format ["A3RO header=%1 before=%2 after=%3",
        isClass (missionConfigFile >> "Header"),
        getNumber (missionConfigFile >> "A3ROBefore" >> "value"),
        getNumber (missionConfigFile >> "A3ROAfter" >> "value")];
    private _shipped = loadConfig "__SHIPPED__";
    diag_log format ["A3RO shipped_null=%1 shipped_class=%2 shipped_idd=%3",
        isNull _shipped,
        isClass (_shipped >> "RscRecruitSyndicats"),
        getNumber (_shipped >> "RscRecruitSyndicats" >> "idd")];
    diag_log "A3RO_END";
};
[] spawn { sleep 3; call A3RO_report; sleep 2; "__PASSWORD__" serverCommand "#shutdown"; };
"""

# RPT lines worth quoting as evidence.
INTERESTING = (
    "a3ro",
    "include file",
    "cannot include",
    "preprocessor failed",
    "starting mission",
    "mission file:",
    "mission world:",
    "description.ext::header",
)


def write_mission(folder: Path, case: str, command_password: str) -> None:
    folder.mkdir(parents=True)
    (folder / "mission.sqm").write_text(oracle.MISSION_SQM, encoding="utf-8", newline="\r\n")
    (folder / "description.ext").write_text(CASES[case], encoding="utf-8", newline="\r\n")
    (folder / "initServer.sqf").write_text(
        INIT_SERVER.replace("__SHIPPED__", SHIPPED_DESCRIPTION).replace(
            "__PASSWORD__", command_password
        ),
        encoding="utf-8",
    )


def run_case(args, case: str, base: Path) -> str:
    """Runs one server with a one-mission `Missions` list. Returns its RPT text."""
    root = oracle.game_dir(args)
    profiles = base / case / "profiles"
    if profiles.exists():
        shutil.rmtree(profiles)
    profiles.mkdir(parents=True)

    mission = f"{MISSION_PREFIX}_{case}_{os.getpid()}.{WORLD}"
    mission_dir = root / "MPMissions" / mission
    for stale in (root / "MPMissions").glob(f"{MISSION_PREFIX}_*.*"):
        if stale.is_dir() and time.time() - stale.stat().st_mtime > oracle.STALE_MISSION_SECONDS:
            shutil.rmtree(stale, ignore_errors=True)

    password, admin, command = (secrets.token_hex(12) for _ in range(3))
    server_cfg = base / case / "server.cfg"
    server_cfg.write_text(oracle.server_cfg(mission, password, admin, command), encoding="utf-8")
    (base / case / "basic.cfg").write_text(oracle.BASIC_CFG, encoding="utf-8")
    port = oracle.free_port()
    cmd = [
        str(root / "arma3server_x64.exe"),
        f"-port={port}",
        f"-config={server_cfg}",
        f"-cfg={base / case / 'basic.cfg'}",
        f"-profiles={profiles}",
        f"-name={oracle.SERVER_NAME}",
        "-nosound",
        "-world=empty",
        "-autoInit",
        "-limitFPS=500",
    ]

    proc = None
    rpt = ""
    try:
        write_mission(mission_dir, case, command)
        print(f"probe: {case}: mission {mission} on port {port}", file=sys.stderr)
        proc = subprocess.Popen(
            cmd,
            cwd=str(root),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            time.sleep(1.0)
            rpt = oracle._read_rpt(profiles)
            if "A3RO_END" in rpt or proc.poll() is not None:
                break
        if "A3RO_END" in rpt:
            try:
                proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                pass
    finally:
        if proc is not None and proc.poll() is None:
            proc.kill()
            proc.wait(timeout=30)
        rpt = oracle._read_rpt(profiles) or rpt
        if mission_dir.exists() and not args.keep_mission:
            shutil.rmtree(mission_dir, ignore_errors=True)
    (base / case / "last.rpt").write_text(rpt, encoding="utf-8")
    print(f"probe: {case}: RPT ({len(rpt)} bytes) -> {base / case / 'last.rpt'}", file=sys.stderr)
    return rpt


def fields(rpt: str) -> dict[str, str]:
    """The `A3RO <key>=<value>` pairs the driver logged, flattened into one dict."""
    out: dict[str, str] = {}
    for line in rpt.splitlines():
        if "A3RO " not in line:
            continue
        for item in line.split("A3RO ", 1)[1].split('"')[0].split():
            if "=" in item:
                key, value = item.split("=", 1)
                out[key] = value
    return out


def report(case: str, rpt: str) -> None:
    print(f"--- {case}: RPT lines ---")
    for line in rpt.splitlines():
        low = line.lower()
        if any(pattern in low for pattern in INTERESTING):
            # The RPT has a `➥` (U+27A5) that a cp1252 console cannot print.
            print(line.rstrip().encode("cp1252", "replace").decode("cp1252"))
    values = fields(rpt)
    if not values:
        print(f"--- {case}: INCONCLUSIVE, the driver never logged; see the RPT.")
        return
    print(f"--- {case}: driver ---")
    print(
        "    mission description.ext:"
        f" header={values.get('header')} before={values.get('before')} after={values.get('after')}"
    )
    print(
        "    loadConfig the shipped description.ext:"
        f" null={values.get('shipped_null')} class={values.get('shipped_class')}"
        f" idd={values.get('shipped_idd')}"
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--game-dir", help="game install (default: $A3_ROOT)")
    parser.add_argument("--work-dir", help="scratch directory (default: the repo's .work)")
    parser.add_argument("--timeout", type=float, default=900.0, help="seconds per case")
    parser.add_argument("--keep-mission", action="store_true", help="do not delete the missions")
    parser.add_argument(
        "--case", action="append", choices=sorted(CASES), help="run only this case (repeatable)"
    )
    args = parser.parse_args(argv)

    base = oracle.work_dir(args) / "oracle" / "probes" / "missing_include"
    for case in args.case or sorted(CASES):
        report(case, run_case(args, case, base))
    return 0


if __name__ == "__main__":
    sys.exit(main())
