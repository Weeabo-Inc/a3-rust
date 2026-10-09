"""Server oracle: differential SQF testing against the original dedicated server.

Runs a corpus of SQF probes on the original `arma3server_x64.exe` (the oracle) and on our engine
(`a3-tools sqf exec`), then diffs the results probe by probe. Only the Python standard library is
used, like the rest of `tools/`.

Both sides run the *same* generated SQF driver. The driver spawns each probe as its own scheduled
script (a script error ends only that probe), and writes each result to the log with
`diag_log text`, in this line protocol:

    A3RO_BEGIN|<id>
    A3RO|<id>|<typeName>|<str value>      one line, value without CR/LF and <= 1000 chars
    A3RE|<id>|<typeName>|<escaped value>  value with CR/LF: \\ -> \\\\, LF -> \\n, CR -> \\r
    A3RV|<id>|<chunk>                     long values: chunks, then
    A3RL|<id>|<typeName>|<chunk count>
    A3RN|<id>|<sign><mantissa>p<exp>      scalars: the exact float32, mantissa = 23-bit fraction
    A3RO_END|<id>|<1 if the probe ran to the end, else 0>
    A3RO_DONE

A nil result is `A3RO|<id>|-|nil`. Anything the engine logs between BEGIN and END that is not a
protocol line (script errors) is kept as evidence.

Commands:
    run       oracle + ours + diff + report (the usual entry point)
    oracle    run the probes on arma3server_x64.exe only
    ours      run the probes on our engine only
    diff      diff the last oracle and ours results
    report    regenerate docs/fidelity/sqf-oracle.md from the last diff
    list      list the probe corpus

Everything the server touches lives under `.work/oracle/server/` (config, profile, RPT). The test
mission is copied to `<A3_ROOT>/MPMissions/a3rust_oracle.<world>/` for the run and removed after.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import os
import re
import secrets
import shutil
import socket
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
PROBES_DIR = HERE / "probes"
MISSION_PREFIX = "a3rust_oracle"
SERVER_NAME = "a3rust_oracle"
DEFAULT_WORLD = "VR"
STALE_MISSION_SECONDS = 4 * 3600

# Protocol line prefixes.
_LINE = re.compile(r"^(A3RO_BEGIN|A3RO_END|A3RO_DONE|A3RO_TIMEOUT|A3RO|A3RE|A3RV|A3RL|A3RN)(\|.*)?$")
# RPT lines start with a wall-clock stamp: " 4:23:11 text" or "04:23:11.123 text".
_RPT_STAMP = re.compile(r"^\s*\d{1,2}:\d{2}:\d{2}(?:\.\d+)?\s(.*)$")


# --------------------------------------------------------------------------------------------
# Probe corpus


@dataclass
class Probe:
    id: str
    code: str
    file: str
    world: str | None  # None: world-independent


def parse_probe_file(text: str, file: str = "<text>") -> list[Probe]:
    """Parses a `.probes` file.

    Format: an optional `@world <name>` directive line; then probes, each a `## <id>` header line
    followed by its SQF code lines. Lines starting with `//` are comments and dropped. Code is
    passed to `compile`, so it must not use preprocessor directives.
    """
    probes: list[Probe] = []
    world: str | None = None
    current: tuple[str, list[str]] | None = None

    def flush() -> None:
        if current is None:
            return
        pid, lines = current
        code = "\n".join(lines).strip()
        if not code:
            raise ValueError(f"{file}: probe {pid} has no code")
        probes.append(Probe(pid, code, file, world))

    for raw in text.splitlines():
        line = raw.rstrip()
        stripped = line.strip()
        if stripped.startswith("//"):
            continue
        if stripped.startswith("@world "):
            if current is not None or probes:
                raise ValueError(f"{file}: @world must come before the first probe")
            world = stripped.split(None, 1)[1].strip() or None
            continue
        if stripped.startswith("## "):
            flush()
            pid = stripped[3:].strip()
            if not re.fullmatch(r"[A-Za-z0-9_.\-]+", pid):
                raise ValueError(f"{file}: bad probe id {pid!r}")
            current = (pid, [])
            continue
        if current is None:
            if stripped:
                raise ValueError(f"{file}: code outside a probe: {stripped!r}")
            continue
        current[1].append(line)
    flush()
    return probes


def load_corpus(directory: Path = PROBES_DIR) -> list[Probe]:
    probes: list[Probe] = []
    seen: dict[str, str] = {}
    for path in sorted(directory.glob("*.probes")):
        for probe in parse_probe_file(path.read_text(encoding="utf-8"), path.name):
            if probe.id in seen:
                raise ValueError(f"duplicate probe id {probe.id} in {path.name} and {seen[probe.id]}")
            seen[probe.id] = path.name
            probes.append(probe)
    return probes


# --------------------------------------------------------------------------------------------
# The SQF driver (identical on both sides)


def sqf_string(text: str) -> str:
    return '"' + text.replace('"', '""') + '"'


DRIVER = r"""
// Generated by tools/oracle/oracle.py. Runs the probes and logs their results (see oracle.py).
A3RO_fnc_num = {
    private _v = _this;
    if !(finite _v) exitWith { str _v };
    if (_v == 0) exitWith { "0" };
    private _s = "+";
    if (_v < 0) then { _s = "-"; _v = 0 - _v };
    private _e = 0;
    while { _v >= 2 } do { _v = _v / 2; _e = _e + 1 };
    while { _v < 1 } do { _v = _v * 2; _e = _e - 1 };
    _s + (((_v - 1) * 8388608) toFixed 0) + "p" + (_e toFixed 0)
};
A3RO_fnc_emit = {
    params ["_a3ro_id", "_a3ro_type", "_a3ro_s"];
    private _lf = toString [10];
    private _cr = toString [13];
    private _tag = "A3RO|";
    if ((_a3ro_s find _lf) >= 0 || { (_a3ro_s find _cr) >= 0 }) then {
        private _o = [];
        {
            if (_x == 10) then { _o append [92, 110] } else {
                if (_x == 13) then { _o append [92, 114] } else {
                    if (_x == 92) then { _o append [92, 92] } else { _o pushBack _x };
                };
            };
        } forEach (toArray _a3ro_s);
        _a3ro_s = toString _o;
        _tag = "A3RE|";
    };
    if (count _a3ro_s > 1000) then {
        private _n = 0;
        private _i = 0;
        while { _i < count _a3ro_s } do {
            diag_log text ("A3RV|" + _a3ro_id + "|" + (_a3ro_s select [_i, 1000]));
            _i = _i + 1000;
            _n = _n + 1;
        };
        diag_log text ("A3RL|" + _a3ro_id + "|" + _a3ro_type + "|" + str _n);
    } else {
        diag_log text (_tag + _a3ro_id + "|" + _a3ro_type + "|" + _a3ro_s);
    };
};
A3RO_fnc_probe = {
    params ["_a3ro_id", "_a3ro_src"];
    private _a3ro_code = compile _a3ro_src;
    private _a3ro_r = call _a3ro_code;
    if (isNil "_a3ro_r") then {
        [_a3ro_id, "-", "nil"] call A3RO_fnc_emit;
    } else {
        [_a3ro_id, typeName _a3ro_r, str _a3ro_r] call A3RO_fnc_emit;
        if (typeName _a3ro_r == "SCALAR") then {
            diag_log text ("A3RN|" + _a3ro_id + "|" + (_a3ro_r call A3RO_fnc_num));
        };
    };
    A3RO_completed = true;
};
A3RO_handle = [] spawn {
    {
        _x params ["_a3ro_id", "_a3ro_src"];
        diag_log text ("A3RO_BEGIN|" + _a3ro_id);
        A3RO_completed = false;
        private _h = [_a3ro_id, _a3ro_src] spawn A3RO_fnc_probe;
        private _n = 0;
        waitUntil { _n = _n + 1; scriptDone _h || { _n > 600 } };
        if !(scriptDone _h) then { terminate _h; diag_log text ("A3RO_TIMEOUT|" + _a3ro_id) };
        diag_log text ("A3RO_END|" + _a3ro_id + "|" + (if (A3RO_completed) then { "1" } else { "0" }));
    } forEach A3RO_probes;
    diag_log text "A3RO_DONE";
    if !(isNil "A3RO_fnc_finish") then { call A3RO_fnc_finish };
};
"""


def driver_script(probes: list[Probe]) -> str:
    items = ",\n".join(f"    [{sqf_string(p.id)}, {sqf_string(p.code)}]" for p in probes)
    return f"A3RO_probes = [\n{items}\n];\n{DRIVER}"


# --------------------------------------------------------------------------------------------
# Parsing results


@dataclass
class Result:
    id: str
    begun: bool = False
    ended: bool = False
    completed: bool = False
    timed_out: bool = False
    type: str | None = None
    value: str | None = None
    number: str | None = None
    evidence: list[str] = field(default_factory=list)

    @property
    def status(self) -> str:
        if not self.begun:
            return "missing"
        if self.type is None or not self.completed:
            return "error"
        return "ok"

    def to_json(self) -> dict:
        return {
            "id": self.id,
            "status": self.status,
            "type": self.type,
            "value": self.value,
            "number": self.number,
            "timed_out": self.timed_out,
            "evidence": self.evidence[:20],
        }


def _unescape(text: str) -> str:
    out = []
    i = 0
    while i < len(text):
        c = text[i]
        if c == "\\" and i + 1 < len(text):
            out.append({"n": "\n", "r": "\r", "\\": "\\"}.get(text[i + 1], text[i + 1]))
            i += 2
        else:
            out.append(c)
            i += 1
    return "".join(out)


def parse_log(lines: list[str]) -> tuple[dict[str, Result], bool]:
    """Parses driver output (log lines without RPT stamps). Returns results and whether DONE."""
    results: dict[str, Result] = {}
    current: Result | None = None
    chunks: dict[str, list[str]] = {}
    done = False
    for line in lines:
        m = _LINE.match(line)
        if not m:
            if current is not None and line.strip():
                current.evidence.append(line.rstrip())
            continue
        tag = m.group(1)
        rest = (m.group(2) or "")[1:]
        if tag == "A3RO_DONE":
            done = True
            current = None
            continue
        pid, _, payload = rest.partition("|")
        result = results.setdefault(pid, Result(pid))
        if tag == "A3RO_BEGIN":
            result.begun = True
            current = result
        elif tag == "A3RO_END":
            result.ended = True
            result.completed = payload.strip() == "1"
            current = None
        elif tag == "A3RO_TIMEOUT":
            result.timed_out = True
        elif tag in ("A3RO", "A3RE"):
            type_name, _, value = payload.partition("|")
            result.type = type_name
            result.value = _unescape(value) if tag == "A3RE" else value
        elif tag == "A3RV":
            chunks.setdefault(pid, []).append(payload)
        elif tag == "A3RL":
            type_name, _, _count = payload.partition("|")
            result.type = type_name
            joined = "".join(chunks.pop(pid, []))
            result.value = _unescape(joined) if "\\" in joined and "\n" not in joined else joined
        elif tag == "A3RN":
            result.number = payload
    return results, done


def strip_rpt(text: str) -> list[str]:
    """RPT text to log lines: the time stamp removed; continuation lines kept as they are."""
    out = []
    for raw in text.splitlines():
        m = _RPT_STAMP.match(raw)
        out.append(m.group(1) if m else raw)
    return out


# --------------------------------------------------------------------------------------------
# Diff


CATEGORIES = ("match", "mismatch", "precision", "ours-error", "oracle-error", "both-error", "missing")


def classify(oracle: Result | None, ours: Result | None) -> str:
    if oracle is None or oracle.status == "missing" or ours is None or ours.status == "missing":
        return "missing"
    o_err = oracle.status == "error"
    u_err = ours.status == "error"
    if o_err and u_err:
        return "both-error"
    if o_err:
        return "oracle-error"
    if u_err:
        return "ours-error"
    if oracle.type != ours.type or oracle.value != ours.value:
        return "mismatch"
    if oracle.number != ours.number:
        return "precision"
    return "match"


def diff(probes: list[Probe], oracle: dict[str, Result], ours: dict[str, Result]) -> list[dict]:
    rows = []
    for probe in probes:
        o = oracle.get(probe.id)
        u = ours.get(probe.id)
        rows.append(
            {
                "id": probe.id,
                "file": probe.file,
                "world": probe.world,
                "code": probe.code,
                "category": classify(o, u),
                "oracle": o.to_json() if o else None,
                "ours": u.to_json() if u else None,
            }
        )
    return rows


def area_of(probe_id: str) -> str:
    return probe_id.split(".", 1)[0]


def summary_markdown(rows: list[dict], meta: dict, issues: list | None = None) -> str:
    """The committed summary: counts per area and category, and the mismatching probe ids."""
    counts: dict[str, dict[str, int]] = {}
    for row in rows:
        area = counts.setdefault(area_of(row["id"]), {c: 0 for c in CATEGORIES})
        area[row["category"]] += 1
    total = {c: sum(a[c] for a in counts.values()) for c in CATEGORIES}
    comparable = len(rows) - total["oracle-error"] - total["missing"]
    rate = 100.0 * total["match"] / comparable if comparable else 0.0

    out = [
        "# SQF server oracle",
        "",
        "Generated by `python tools/oracle/oracle.py report` from the last oracle run; do not edit.",
        "The oracle is the original `arma3server_x64.exe` "
        f"({meta.get('game_build', '2.22.0.154103')}); ours is `a3-tools sqf exec`. "
        "Probes live in `tools/oracle/probes/`; see `tools/oracle/oracle.py` for the protocol.",
        "",
        "Reproduce (Windows, game install in `A3_ROOT`): `cargo build --release -p a3-tools`, then",
        "`python tools/oracle/oracle.py run` (add `--filter <regex>` for a subset). Raw results go to",
        "`.work/oracle/results/` (`oracle.json`, `ours.json`, `diff.json`).",
        "",
        f"Run: {meta.get('date', '?')}, ours at commit `{meta.get('commit', '?')}`.",
        "",
        f"**{total['match']} of {comparable} comparable probes match ({rate:.1f}%)**; "
        f"{len(rows)} probes in total.",
        "",
        "Categories: `match` same type and `str`, and the same float32 for numbers; `precision` same",
        "`str` but a different float32; `mismatch` different type or `str`; `ours-error` the",
        "original returns a value, we raise a script error (or abort); `oracle-error` the probe",
        "errors on the original (the probe is wrong or the command is absent; not compared);",
        "`both-error` both raise; `missing` no result from one side.",
        "",
        "| Area | " + " | ".join(CATEGORIES) + " |",
        "|---|" + "---:|" * len(CATEGORIES),
    ]
    for area in sorted(counts):
        out.append(f"| {area} | " + " | ".join(str(counts[area][c]) for c in CATEGORIES) + " |")
    out.append("| **total** | " + " | ".join(f"**{total[c]}**" for c in CATEGORIES) + " |")
    out.append("")
    failing = [r for r in rows if r["category"] in ("mismatch", "precision", "ours-error")]
    issues = issues or []
    if failing:
        out += ["## Failing probes", "", "| Probe | Category | Issue |", "|---|---|---|"]
        for row in failing:
            issue = next((f"#{n}" for rx, n, _ in issues if rx.search(row["id"])), "")
            out.append(f"| `{row['id']}` | {row['category']} | {issue} |")
        out.append("")
    return "\n".join(out)


def load_issues(path: Path = HERE / "known-issues.tsv") -> list[tuple[re.Pattern, int, str]]:
    """`known-issues.tsv`: `<probe id regex> TAB <issue number> TAB <title>` per line."""
    issues = []
    if not path.is_file():
        return issues
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        rx, number, *title = line.split("\t")
        issues.append((re.compile(rx), int(number.lstrip("#")), title[0] if title else ""))
    return issues


# --------------------------------------------------------------------------------------------
# Running the oracle


def game_dir(args) -> Path:
    root = args.game_dir or os.environ.get("A3_ROOT")
    if not root:
        sys.exit("set A3_ROOT or pass --game-dir")
    path = Path(root)
    if not (path / "arma3server_x64.exe").is_file():
        sys.exit(f"no arma3server_x64.exe in {path}")
    return path


def work_dir(args) -> Path:
    # Absolute: the server runs with the game folder as its working directory.
    return Path(args.work_dir).resolve() if args.work_dir else REPO_WORK


def _find_repo_work() -> Path:
    # The shared `.work` of the main checkout, also from a worktree under `.claude/worktrees/`.
    for parent in [REPO, *REPO.parents]:
        if (parent / ".work").is_dir() and (parent / "Cargo.toml").is_file():
            return parent / ".work"
    return REPO / ".work"


REPO_WORK = _find_repo_work()


def free_port() -> int:
    # Arma uses port, port+1 (Steam query) and port+2/+3/+4; find a run of five free ones.
    for _ in range(50):
        base = 23000 + secrets.randbelow(4000) * 5  # below the dynamic port range (49152+)
        ok = True
        for offset in range(5):
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
                try:
                    s.bind(("0.0.0.0", base + offset))
                except OSError:
                    ok = False
                    break
        if ok:
            return base
    return 2402


MISSION_SQM = """version=54;
class EditorData
{
\tmoveGridStep=1;
\tangleGridStep=0.2617994;
\tscaleGridStep=1;
\tautoGroupingDist=10;
\ttoggles=1;
};
binarizationWanted=0;
addons[]={};
class AddonsMetaData
{
\tclass List
\t{
\t\titems=0;
\t};
};
randomSeed=1234567;
class ScenarioData
{
\tauthor="a3rust oracle";
};
class Mission
{
\tclass Intel
\t{
\t\ttimeOfChanges=1800.0002;
\t\tstartWeather=0;
\t\tstartWind=0;
\t\tstartWaves=0;
\t\tforecastWeather=0;
\t\tforecastWind=0;
\t\tforecastWaves=0;
\t\tforecastLightnings=0;
\t\tyear=2035;
\t\tmonth=6;
\t\tday=24;
\t\thour=12;
\t\tminute=0;
\t\tstartFogDecay=0.014;
\t\tforecastFogDecay=0.014;
\t};
};
"""

DESCRIPTION_EXT = """class Header
{
\tgameType = "Sandbox";
\tminPlayers = 1;
\tmaxPlayers = 1;
};
respawn = 0;
"""


def write_mission(folder: Path, probes: list[Probe], command_password: str) -> None:
    folder.mkdir(parents=True)
    (folder / "mission.sqm").write_text(MISSION_SQM, encoding="utf-8", newline="\r\n")
    (folder / "description.ext").write_text(DESCRIPTION_EXT, encoding="utf-8", newline="\r\n")
    finish = (
        "A3RO_fnc_finish = { [] spawn { sleep 1; "
        + sqf_string(command_password)
        + ' serverCommand "#shutdown"; }; };\n'
    )
    (folder / "initServer.sqf").write_text(finish + driver_script(probes), encoding="utf-8")


def server_cfg(mission: str, password: str, admin: str, command: str) -> str:
    return f"""// Generated by tools/oracle/oracle.py for one oracle run. LAN loopback only, not listed.
hostname = "{SERVER_NAME}";
password = "{password}";
passwordAdmin = "{admin}";
serverCommandPassword = "{command}";
maxPlayers = 1;
persistent = 1;
loopback = true;
upnp = false;
BattlEye = 0;
verifySignatures = 0;
kickDuplicate = 1;
voteThreshold = 2;
disableVoN = 1;
logFile = "";
timeStampFormat = "short";
statisticsEnabled = 0;
class Missions
{{
    class Oracle
    {{
        template = "{mission}";
        difficulty = "Regular";
    }};
}};
"""


BASIC_CFG = """// Generated by tools/oracle/oracle.py.
MaxMsgSend = 128;
MaxSizeGuaranteed = 512;
MaxSizeNonguaranteed = 256;
MinBandwidth = 131072;
MaxBandwidth = 10000000000;
MinErrorToSend = 0.001;
MinErrorToSendNear = 0.01;
MaxCustomFileSize = 0;
"""


def run_oracle(args, probes: list[Probe], world: str) -> tuple[dict[str, Result], dict]:
    root = game_dir(args)
    base = work_dir(args) / "oracle" / "server"
    profiles = base / "profiles"
    if profiles.exists():
        shutil.rmtree(profiles)
    profiles.mkdir(parents=True)
    base.mkdir(parents=True, exist_ok=True)

    # One mission folder per run (the process id in the name), so concurrent runs from several
    # worktrees do not delete each other's mission.
    mission = f"{MISSION_PREFIX}_{os.getpid()}.{world}"
    mission_dir = root / "MPMissions" / mission
    # Leftovers of an interrupted run: only our own, clearly named mission folders that are
    # older than any run could take.
    for stale in (root / "MPMissions").glob(f"{MISSION_PREFIX}*.*"):
        if stale.is_dir() and time.time() - stale.stat().st_mtime > STALE_MISSION_SECONDS:
            shutil.rmtree(stale, ignore_errors=True)
    password, admin, command = (secrets.token_hex(12) for _ in range(3))
    (base / "server.cfg").write_text(server_cfg(mission, password, admin, command), encoding="utf-8")
    (base / "basic.cfg").write_text(BASIC_CFG, encoding="utf-8")
    port = free_port()
    exe = root / "arma3server_x64.exe"
    cmd = [
        str(exe),
        f"-port={port}",
        f"-config={base / 'server.cfg'}",
        f"-cfg={base / 'basic.cfg'}",
        f"-profiles={profiles}",
        f"-name={SERVER_NAME}",
        "-nosound",
        "-world=empty",
        "-autoInit",
        "-limitFPS=500",
    ]
    meta = {"world": world, "port": port, "probes": len(probes)}
    started = time.monotonic()
    proc = None
    done = False
    rpt_text = ""
    try:
        write_mission(mission_dir, probes, command)
        print(f"oracle: {len(probes)} probes on {world}, port {port}", file=sys.stderr)
        proc = subprocess.Popen(
            cmd,
            cwd=str(root),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        deadline = started + args.timeout
        while time.monotonic() < deadline:
            time.sleep(1.0)
            rpt_text = _read_rpt(profiles)
            if "A3RO_DONE" in rpt_text:
                done = True
                break
            if proc.poll() is not None:
                break
        if done:
            try:
                proc.wait(timeout=20)
            except subprocess.TimeoutExpired:
                pass
    finally:
        if proc is not None and proc.poll() is None:
            proc.kill()
            proc.wait(timeout=30)
        rpt_text = _read_rpt(profiles) or rpt_text
        if mission_dir.exists():
            shutil.rmtree(mission_dir, ignore_errors=True)
    meta["seconds"] = round(time.monotonic() - started, 1)
    meta["done"] = done
    meta["exit_code"] = proc.returncode if proc else None
    (base / "last.rpt").write_text(rpt_text, encoding="utf-8")
    results, finished = parse_log(strip_rpt(rpt_text))
    meta["done"] = meta["done"] or finished
    if not results:
        print(f"oracle: no probe output; see {base / 'last.rpt'}", file=sys.stderr)
    return results, meta


def _read_rpt(profiles: Path) -> str:
    rpts = sorted(profiles.rglob("*.rpt"), key=lambda p: p.stat().st_mtime)
    if not rpts:
        return ""
    return rpts[-1].read_text(encoding="utf-8", errors="replace")


def run_ours(args, probes: list[Probe], world: str | None) -> tuple[dict[str, Result], dict]:
    root = game_dir(args)
    base = work_dir(args) / "oracle" / "ours"
    base.mkdir(parents=True, exist_ok=True)
    script = base / "a3ro_main.sqf"
    script.write_text(driver_script(probes), encoding="utf-8")
    tool = args.a3_tools or _default_a3_tools()
    cmd = [str(tool), "sqf", "--game-dir", str(root), "exec", str(script), "--init-functions",
           "--errors-in-log", "--frames", str(max(5000, len(probes) * 20))]
    if world:
        cmd += ["--world", world]
    print(f"ours: {len(probes)} probes" + (f" on {world}" if world else ""), file=sys.stderr)
    started = time.monotonic()
    try:
        proc = subprocess.run(cmd, capture_output=True, timeout=args.timeout, stdin=subprocess.DEVNULL)
        stdout = proc.stdout.decode("utf-8", errors="replace")
        stderr = proc.stderr.decode("utf-8", errors="replace")
        code = proc.returncode
    except subprocess.TimeoutExpired as e:
        stdout = (e.stdout or b"").decode("utf-8", errors="replace")
        stderr = (e.stderr or b"").decode("utf-8", errors="replace")
        code = None
    (base / "last.out").write_text(stdout, encoding="utf-8")
    (base / "last.err").write_text(stderr, encoding="utf-8")
    results, done = parse_log(stdout.splitlines())
    meta = {"seconds": round(time.monotonic() - started, 1), "done": done, "exit_code": code}
    if not results:
        print(f"ours: no probe output; see {base}", file=sys.stderr)
    return results, meta


def _default_a3_tools() -> Path:
    exe = "a3-tools.exe" if os.name == "nt" else "a3-tools"
    for profile in ("release", "debug"):
        path = REPO / "target" / profile / exe
        if path.is_file():
            return path
    sys.exit("build a3-tools first: cargo build --release -p a3-tools (or pass --a3-tools)")


# --------------------------------------------------------------------------------------------
# CLI


def results_dir(args) -> Path:
    path = work_dir(args) / "oracle" / "results"
    path.mkdir(parents=True, exist_ok=True)
    return path


def select(args) -> list[Probe]:
    probes = load_corpus(Path(args.probes_dir) if args.probes_dir else PROBES_DIR)
    if args.filter:
        rx = re.compile(args.filter)
        probes = [p for p in probes if rx.search(p.id)]
    return probes


def by_world(probes: list[Probe]) -> dict[str | None, list[Probe]]:
    groups: dict[str | None, list[Probe]] = {}
    for p in probes:
        groups.setdefault(p.world, []).append(p)
    return groups


def save(args, name: str, results: dict[str, Result], meta: dict) -> None:
    data = {"meta": meta, "results": {k: v.to_json() for k, v in results.items()}}
    (results_dir(args) / f"{name}.json").write_text(json.dumps(data, indent=1), encoding="utf-8")


def load_results(args, name: str) -> tuple[dict[str, Result], dict]:
    data = json.loads((results_dir(args) / f"{name}.json").read_text(encoding="utf-8"))
    results = {}
    for k, v in data["results"].items():
        r = Result(k)
        r.begun = v["status"] != "missing"
        r.completed = v["status"] == "ok"
        r.ended = r.begun
        r.type, r.value, r.number = v["type"], v["value"], v["number"]
        r.timed_out = v.get("timed_out", False)
        r.evidence = v.get("evidence", [])
        results[k] = r
    return results, data["meta"]


def cmd_oracle(args) -> None:
    results: dict[str, Result] = {}
    metas = {}
    # The server always runs a mission on some world; world-independent probes share VR's run.
    groups: dict[str, list[Probe]] = {}
    for p in select(args):
        groups.setdefault(p.world or DEFAULT_WORLD, []).append(p)
    for world, probes in groups.items():
        r, m = run_oracle(args, probes, world)
        results.update(r)
        metas[world] = m
        print(f"oracle: {len(r)} results, done={m['done']} in {m['seconds']} s", file=sys.stderr)
    save(args, "oracle", results, {"runs": metas, "date": _now()})


def cmd_ours(args) -> None:
    results: dict[str, Result] = {}
    metas = {}
    for world, probes in by_world(select(args)).items():
        r, m = run_ours(args, probes, world)
        results.update(r)
        metas[world or "-"] = m
        print(f"ours: {len(r)} results, done={m['done']} in {m['seconds']} s", file=sys.stderr)
    save(args, "ours", results, {"runs": metas, "date": _now()})


def cmd_diff(args) -> None:
    probes = select(args)
    oracle, _ = load_results(args, "oracle")
    ours, _ = load_results(args, "ours")
    rows = diff(probes, oracle, ours)
    meta = {"date": _now(), "commit": _git_commit(), "probes": len(rows)}
    out = results_dir(args) / "diff.json"
    out.write_text(json.dumps({"meta": meta, "rows": rows}, indent=1), encoding="utf-8")
    counts = {c: sum(1 for r in rows if r["category"] == c) for c in CATEGORIES}
    print(" ".join(f"{c}={n}" for c, n in counts.items()))
    for row in rows:
        if row["category"] in ("mismatch", "precision", "ours-error") or args.verbose:
            o, u = row["oracle"] or {}, row["ours"] or {}
            print(f"{row['category']:12} {row['id']}")
            print(f"    oracle: {o.get('type')} {o.get('value')!r} {o.get('number') or ''}")
            print(f"    ours:   {u.get('type')} {u.get('value')!r} {u.get('number') or ''}")
            for line in (u.get("evidence") or [])[:2]:
                print(f"      ours! {line}")
    print(f"wrote {out}")


def cmd_report(args) -> None:
    data = json.loads((results_dir(args) / "diff.json").read_text(encoding="utf-8"))
    md = summary_markdown(data["rows"], data["meta"], load_issues())
    out = REPO / "docs" / "fidelity" / "sqf-oracle.md"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(md + "\n", encoding="utf-8")
    print(f"wrote {out}")


def cmd_run(args) -> None:
    if not args.skip_oracle:
        cmd_oracle(args)
    cmd_ours(args)
    cmd_diff(args)
    cmd_report(args)


def cmd_list(args) -> None:
    for p in select(args):
        print(f"{p.id:40} {p.world or '-':8} {p.code.splitlines()[0][:60]}")


def _now() -> str:
    return _dt.datetime.now().strftime("%Y-%m-%d")


def _git_commit() -> str:
    try:
        return subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"], cwd=REPO, capture_output=True, text=True
        ).stdout.strip()
    except OSError:
        return "?"


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=["run", "oracle", "ours", "diff", "report", "list"])
    parser.add_argument("--game-dir", help="game install (default: A3_ROOT)")
    parser.add_argument("--work-dir", help="scratch folder (default: the repo's .work)")
    parser.add_argument("--a3-tools", help="our a3-tools binary (default: target/release)")
    parser.add_argument("--filter", help="regex on probe ids")
    parser.add_argument("--probes-dir", help="probe corpus folder (default: tools/oracle/probes)")
    parser.add_argument("--timeout", type=float, default=900.0, help="seconds per engine run")
    parser.add_argument("--skip-oracle", action="store_true", help="run: reuse the last oracle results")
    parser.add_argument("-v", "--verbose", action="store_true")
    args = parser.parse_args(argv)
    {
        "run": cmd_run,
        "oracle": cmd_oracle,
        "ours": cmd_ours,
        "diff": cmd_diff,
        "report": cmd_report,
        "list": cmd_list,
    }[args.command](args)


if __name__ == "__main__":
    main()
