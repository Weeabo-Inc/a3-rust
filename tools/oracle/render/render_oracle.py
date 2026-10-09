"""Render oracle: the same shots from the real Arma 3 client and from a3-rust, compared.

Steps (each one a subcommand, `all` runs them in order):

  client   Generate an Arma3.cfg, a profile and a scripted scenario under <work>/client, start
           arma3_x64.exe once per World, wait until every screenshot of that World exists, then
           stop the game. Copies each shot to <work>/render/<shot>/arma.png.
  ours     Run apps/arma3 --screenshot per shot with the same camera, date, time, weather and
           view distance. Writes <work>/render/<shot>/ours.png.
  compare  Run `a3-tools image-diff` per shot, writes <work>/render/<shot>/{side,diff}.png and
           metrics.json, and <work>/render/report.md with the metric table.

The game is used read-only: the client gets its own -cfg and -profiles, never the user's
Documents\\Arma 3 settings. Only the game's own `screenshot` command captures images, so no
other window or the desktop is ever read.

Standard library only. Usage:

  python -I tools/oracle/render/render_oracle.py all --game-dir P:\\a3-rust\\oirignal
  python -I tools/oracle/render/render_oracle.py client --only altis_kavala_noon
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent.parent
PROFILE_NAME = "a3rust_oracle"
# 1600x900 keeps the client window inside a 1080p desktop: a window that does not fit gets
# shrunk by Windows and the game then renders (and screenshots) at the smaller size.
WIDTH, HEIGHT = 1600, 900

# Video settings of the oracle client. Post effects our renderer does not have are off (SSAO,
# DOF, motion and radial blur, sharpen, caustics, AA), so the comparison measures the base
# shading; HDR precision 16 is required by the `screenshot` command.
ARMA3_CFG = {
    "language": '"English"',
    "forcedAdapterId": -1,
    "displayMode": 0,
    "winX": 0,
    "winY": 0,
    "winWidth": WIDTH,
    "winHeight": HEIGHT,
    "winDefWidth": WIDTH,
    "winDefHeight": HEIGHT,
    "fullScreenWidth": WIDTH,
    "fullScreenHeight": HEIGHT,
    "refresh": 60,
    "renderWidth": WIDTH,
    "renderHeight": HEIGHT,
    "multiSampleCount": 1,
    "multiSampleQuality": 0,
    "particlesQuality": 2,
    "GPU_MaxFramesAhead": 1000,
    "GPU_DetectedFramesAhead": 1,
    "HDRPrecision": 16,
    "vsync": 0,
    "AToC": 0,
    "cloudsQuality": 3,
    "waterSSReflectionsQuality": 0,
    "pipQuality": 0,
    "dynamicLightsQuality": 3,
    "PPAA": 0,
    "ppSSAO": 0,
    "ppCaustics": 0,
    "ppHaze": 0,
    "tripleBuffering": 0,
    "ppBloom": 1,
    "ppRotBlur": 0,
    "ppRadialBlur": 0,
    "ppDOF": 0,
    "ppSharpen": 0,
}

# Profile video options: Ultra objects (sceneComplexity, matches --objects-quality Ultra),
# Ultra terrain (terrainGrid 3.125), high shadows, the default fovTop and a 16:9 fovLeft.
PROFILE = {
    "version": 2,
    "blood": 1,
    "anisoFilter": 4,
    "textureQuality": 3,
    "shadowQuality": 3,
    "sceneComplexity": 1800000,
    "shadowZDistance": 100,
    "viewDistance": 3000,
    "preferredObjectViewDistance": 2000,
    "terrainGrid": 3.125,
    "volumeCD": 0,
    "volumeFX": 0,
    "volumeSpeech": 0,
    "volumeVoN": 0,
    "volumeUI": 0,
    "volumeMapDucking": 1,
    "gamma": 1,
    "brightness": 1,
    "fovTop": 0.75,
    "fovLeft": round(0.75 * WIDTH / HEIGHT, 7),
    "IGUIScale": 1,
    "maxScreenShotFolderSizeMB": 2000,
}
OBJECTS_QUALITY = "Ultra"


def default_work_dir() -> Path:
    """`.work/oracle` of the main checkout (shared by every worktree), else of this repo."""
    try:
        common = subprocess.run(
            ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
            cwd=REPO,
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        return Path(common).parent / ".work" / "oracle"
    except (OSError, subprocess.CalledProcessError):
        return REPO / ".work" / "oracle"


def load_shots(path: Path, only: list[str] | None) -> list[dict]:
    data = json.loads(path.read_text(encoding="utf-8"))
    defaults = data["defaults"]
    shots = []
    for raw in data["shots"]:
        shot = {**defaults, **raw}
        if only and shot["name"] not in only:
            continue
        shots.append(shot)
    if only:
        missing = set(only) - {s["name"] for s in shots}
        if missing:
            sys.exit(f"unknown shots: {', '.join(sorted(missing))}")
    return shots


def hours_minutes(text: str) -> tuple[int, int]:
    h, m = text.split(":")
    return int(h), int(m)


# --- client ---------------------------------------------------------------------------------


def sqf_value(v) -> str:
    if isinstance(v, str):
        return "'" + v.replace("'", "''") + "'"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (list, tuple)):
        return "[" + ",".join(sqf_value(x) for x in v) + "]"
    return repr(float(v)) if isinstance(v, float) else str(v)


# The scenario, run by playScriptedMission inside an empty mission on the World. Per shot: fix
# date, weather and fog, place the camera, preload the area, settle, take the screenshot.
# forceWeatherChange stalls the game for a while, so it runs only when the overcast changes.
# Single quotes only: the code travels inside a command-line argument.
SCENARIO = """
[] spawn {
 private _shots = %SHOTS%;
 showCinemaBorder false;
 enableEnvironment [false, false];
 setWind [0, 0, true];
 0 setGusts 0;
 0 setWaves 0;
 private _cam = 'camera' camCreate [0, 0, 500];
 _cam cameraEffect ['internal', 'back'];
 {
  _x params ['_name', '_pos', '_h', '_p', '_fov', '_date', '_oc', '_fog', '_vd', '_ovd'];
  setViewDistance _vd;
  setObjectViewDistance [_ovd, 100];
  0 setRain 0;
  0 setLightnings 0;
  if (abs (overcast - _oc) > 0.01) then { 0 setOvercast _oc; forceWeatherChange; };
  0 setFog _fog;
  setDate _date;
  private _z = ((getTerrainHeightASL [_pos select 0, _pos select 1]) max 0) + (_pos select 2);
  _cam setPosASL [_pos select 0, _pos select 1, _z];
  _cam setVectorDirAndUp [[(sin _h) * (cos _p), (cos _h) * (cos _p), sin _p], [-(sin _h) * (sin _p), -(cos _h) * (sin _p), cos _p]];
  _cam camSetFov _fov;
  _cam camCommit 0;
  _cam camPreload 60;
  private _t = diag_tickTime;
  waitUntil {camPreloaded _cam || {diag_tickTime - _t > 60}};
  sleep 10;
  setDate _date;
  0 setFog _fog;
  sleep 2;
  diag_log format ['a3rust_oracle shot=%1 posASL=%2 terrainASL=%3 date=%4 overcast=%5 fog=%6 moonPhase=%7 sunOrMoon=%8 viewDistance=%9', _name, getPosASL _cam, getTerrainHeightASL [_pos select 0, _pos select 1], date, overcast, fogParams, moonPhase date, sunOrMoon, viewDistance];
  screenshot (_name + '.png');
  sleep 3;
 } forEach _shots;
 diag_log 'a3rust_oracle done';
};
"""


def scenario_code(shots: list[dict]) -> str:
    rows = []
    for s in shots:
        e, n, alt, heading, pitch = s["camera"]
        hh, mm = hours_minutes(s["time"])
        rows.append(
            [
                s["name"],
                [float(e), float(n), float(alt)],
                float(heading),
                float(pitch),
                float(s["fov"]),
                [*s["date"], hh, mm],
                float(s["overcast"]),
                [float(x) for x in s["fog"]],
                float(s["view_distance"]),
                float(s["object_view_distance"]),
            ]
        )
    code = SCENARIO.replace("%SHOTS%", sqf_value(rows))
    return " ".join(line.strip() for line in code.strip().splitlines())


def write_config(path: Path, values: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(f"{k}={v};\n" for k, v in values.items()), encoding="utf-8")


def find_screenshot(client: Path, name: str) -> Path | None:
    for p in (client / "profiles").rglob(name + ".png"):
        return p
    return None


def run_client(game_dir: Path, work: Path, shots: list[dict], timeout: float) -> None:
    client = work / "client"
    profiles = client / "profiles"
    exe = game_dir / "arma3_x64.exe"
    if not exe.exists():
        sys.exit(f"no {exe}")
    worlds: dict[str, list[dict]] = {}
    for s in shots:
        worlds.setdefault(s["world"], []).append(s)
    for world, group in worlds.items():
        write_config(client / "Arma3.cfg", ARMA3_CFG)
        write_config(profiles / "Users" / PROFILE_NAME / f"{PROFILE_NAME}.Arma3Profile", PROFILE)
        for s in group:
            old = find_screenshot(client, s["name"])
            if old:
                old.unlink()
        code = scenario_code(group)
        init = f"playScriptedMission ['{world}', {{{code}}}, missionConfigFile, true]"
        (client / f"scenario_{world}.sqf").write_text(init + "\n", encoding="utf-8")
        args = [
            str(exe),
            f"-cfg={client / 'Arma3.cfg'}",
            f"-profiles={profiles}",
            f"-name={PROFILE_NAME}",
            "-skipIntro",
            "-noSplash",
            "-noLauncher",
            "-window",
            "-nosound",
            "-world=empty",
            f"-init={init}",
        ]
        print(f"client: {world}, {len(group)} shots", flush=True)
        proc = subprocess.Popen(args, cwd=game_dir)
        start = time.monotonic()
        pending = {s["name"] for s in group}
        try:
            while pending and time.monotonic() - start < timeout:
                time.sleep(2)
                if proc.poll() is not None:
                    print(f"client exited early with code {proc.returncode}", flush=True)
                    break
                for name in sorted(pending):
                    p = find_screenshot(client, name)
                    if p and stable(p):
                        dest = work / "render" / name / "arma.png"
                        dest.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copyfile(p, dest)
                        pending.discard(name)
                        size = png_size(dest)
                        note = "" if size == (WIDTH, HEIGHT) else f", WRONG SIZE {size}"
                        print(f"  {name}: {dest} ({time.monotonic() - start:.0f} s{note})", flush=True)
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait()
        if pending:
            print(f"client: missing shots {sorted(pending)}; see the RPT in {profiles}", flush=True)
        # Freeze minidumps (forceWeatherChange stalls trip the watchdog) are large and useless.
        for dump in profiles.glob("*.mdmp"):
            dump.unlink(missing_ok=True)


def png_size(path: Path) -> tuple[int, int]:
    """Width and height from a PNG's IHDR chunk."""
    with path.open("rb") as f:
        head = f.read(24)
    return int.from_bytes(head[16:20], "big"), int.from_bytes(head[20:24], "big")


def stable(path: Path) -> bool:
    """True when the file exists and its size did not change for half a second."""
    try:
        a = path.stat().st_size
        time.sleep(0.5)
        return a > 0 and path.stat().st_size == a
    except OSError:
        return False


# --- ours -----------------------------------------------------------------------------------


def our_args(binary: Path, game_dir: Path, shot: dict, out: Path) -> list[str]:
    e, n, alt, heading, pitch = shot["camera"]
    y, m, d = shot["date"]
    fog = ",".join(str(float(x)) for x in shot["fog"])
    return [
        str(binary),
        "--game-dir",
        str(game_dir),
        "--world",
        shot["world"].lower(),
        "--camera",
        f"{e},{n},{alt},{heading},{pitch}",
        "--fov",
        f"{camera_fov_top(shot['fov']):.6f}",
        "--date",
        f"{y}-{m}-{d}",
        "--time",
        shot["time"],
        "--overcast",
        str(shot["overcast"]),
        "--fog",
        fog,
        "--fog-distance",
        str(shot["view_distance"]),
        "--view-distance",
        str(shot["object_view_distance"]),
        "--objects-quality",
        OBJECTS_QUALITY,
        "--width",
        str(WIDTH),
        "--height",
        str(HEIGHT),
        "--no-overlay",
        "--frames",
        "120",
        "--screenshot",
        str(out),
    ]


def camera_fov_top(fov: float) -> float:
    """RV fovTop of a `camSetFov fov` camera.

    Measured with this oracle at 16:9 and 4:3: the client's image matches ours at
    fovTop = fov * 0.75 at both aspects, i.e. the camera's fov scales the profile's fovTop
    (0.75 here) and fovLeft follows the aspect. See docs/fidelity/render-oracle.md.
    """
    return fov * PROFILE["fovTop"]

def run_ours(binary: Path, game_dir: Path, work: Path, shots: list[dict]) -> None:
    if not binary.exists():
        sys.exit(f"no {binary}; build it with cargo build --release -p arma3")
    for s in shots:
        out = work / "render" / s["name"] / "ours.png"
        out.parent.mkdir(parents=True, exist_ok=True)
        args = our_args(binary, game_dir, s, out)
        print(f"ours: {s['name']}", flush=True)
        log = out.with_name("ours.log")
        with log.open("w", encoding="utf-8") as f:
            r = subprocess.run(args, stdout=f, stderr=subprocess.STDOUT)
        if r.returncode != 0:
            print(f"  failed ({r.returncode}), see {log}", flush=True)


# --- compare --------------------------------------------------------------------------------

COLUMNS = [
    ("mae_linear", "MAE lin"),
    ("ssim", "SSIM"),
    ("mean_lum_arma", "lum Arma"),
    ("mean_lum_ours", "lum ours"),
    ("lum_ratio", "lum ratio"),
    ("histogram_distance", "hist dist"),
]


def run_compare(tools: Path, work: Path, shots: list[dict]) -> None:
    if not tools.exists():
        sys.exit(f"no {tools}; build it with cargo build --release -p a3-tools")
    rows = []
    for s in shots:
        d = work / "render" / s["name"]
        arma, ours = d / "arma.png", d / "ours.png"
        if not (arma.exists() and ours.exists()):
            print(f"compare: {s['name']}: missing {'arma.png' if not arma.exists() else 'ours.png'}")
            continue
        r = subprocess.run(
            [str(tools), "image-diff", str(arma), str(ours), "--out", str(d), "--json"],
            capture_output=True,
            text=True,
        )
        if r.returncode != 0:
            print(f"compare: {s['name']}: {r.stderr.strip()}")
            continue
        metrics = json.loads(r.stdout)
        (d / "metrics.json").write_text(json.dumps(metrics, indent=2), encoding="utf-8")
        rows.append((s, metrics))
        print(f"compare: {s['name']}: " + ", ".join(f"{k} {metrics[k]:.3f}" for k, _ in COLUMNS))
    lines = [
        "| shot | " + " | ".join(h for _, h in COLUMNS) + " | MAE R/G/B (linear) |",
        "|---|" + "---|" * (len(COLUMNS) + 1),
    ]
    for s, m in rows:
        rgb = "/".join(f"{x:.3f}" for x in m["mae_linear_rgb"])
        lines.append(
            f"| {s['name']} | " + " | ".join(f"{m[k]:.3f}" for k, _ in COLUMNS) + f" | {rgb} |"
        )
    report = work / "render" / "report.md"
    report.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"report: {report}")


def main() -> None:
    global WIDTH, HEIGHT
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("step", choices=["client", "ours", "compare", "all"])
    ap.add_argument("--game-dir", type=Path, default=os.environ.get("A3_ROOT"))
    ap.add_argument("--work", type=Path, default=None, help="default: <main checkout>/.work/oracle")
    ap.add_argument("--shots", type=Path, default=HERE / "shots.json")
    ap.add_argument("--only", nargs="*", help="shot names to run")
    ap.add_argument("--timeout", type=float, default=900, help="seconds per client run")
    ap.add_argument("--bin", type=Path, default=REPO / "target" / "release", help="our binaries")
    ap.add_argument("--size", default=f"{WIDTH}x{HEIGHT}", help="render size WxH")
    args = ap.parse_args()
    WIDTH, HEIGHT = (int(v) for v in args.size.lower().split("x"))
    ARMA3_CFG.update(
        winWidth=WIDTH, winHeight=HEIGHT, winDefWidth=WIDTH, winDefHeight=HEIGHT,
        renderWidth=WIDTH, renderHeight=HEIGHT,
    )
    PROFILE["fovLeft"] = round(0.75 * WIDTH / HEIGHT, 7)
    if args.game_dir is None:
        sys.exit("set A3_ROOT or pass --game-dir")
    work = (args.work or default_work_dir()).resolve()
    shots = load_shots(args.shots, args.only)
    exe = ".exe" if os.name == "nt" else ""
    if args.step in ("client", "all"):
        run_client(args.game_dir, work, shots, args.timeout)
    if args.step in ("ours", "all"):
        run_ours(args.bin / f"arma3{exe}", args.game_dir, work, shots)
    if args.step in ("compare", "all"):
        run_compare(args.bin / f"a3-tools{exe}", work, shots)


if __name__ == "__main__":
    main()
