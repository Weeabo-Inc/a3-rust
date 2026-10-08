# RE tooling

How every agent queries the original binaries from a shell. Large artifacts live under
`P:\a3-rust\.work\` (git-ignored, shared by all agents); only scripts and docs are committed.

## What runs where

| Piece | Location | Status |
|---|---|---|
| Ghidra 12.1.4 (JDK 27) | `P:\a3-rust\.resources\ghidra_12.1.4_PUBLIC` | works |
| Ghidra project `a3` (arma3_x64.exe, full auto-analysis incl. MSVC RTTI + demangler, 1216 s) | `P:\a3-rust\.work\ghidra\a3.gpr`, opened via the junction `P:\a3-ghidra\a3.gpr` | done |
| Ghidra project `a3server` (arma3server_x64.exe) | `P:\a3-ghidra\a3server.gpr` | see `docs/re/net-*.md` |
| **GhidraMCP headless server** (bethington/ghidra-mcp 7.0.0, 190 endpoints) | clone + Maven build in `.work\tools\ghidra-mcp`; HTTP on `127.0.0.1:8089` | **works — main backend** |
| GhidraMCP GUI extension | unzipped into `ghidra_12.1.4_PUBLIC\Ghidra\Extensions\GhidraMCP` | installed, optional |
| rea 6.0.0 (morluto/rea, npm) | `.work\tools\rea-npm` | works for small binaries only (see below) |
| ida-pro-mcp | `.work\venvs\ida-pro-mcp` | **does not work with IDA Free 9.4** (see below) |

Ghidra rejects project paths that contain an element starting with `.` (`Path element starting
with '.' is not permitted`), so the project in `.work\ghidra` is always opened through the
directory junction `P:\a3-ghidra` → `P:\a3-rust\.work\ghidra`.

## Start / stop the shared query server

```powershell
powershell -File tools/re/ghidra-mcp-server.ps1 start    # opens a3.gpr, loads arma3_x64.exe (~5-8 min)
powershell -File tools/re/ghidra-mcp-server.ps1 status
powershell -File tools/re/ghidra-mcp-server.ps1 stop     # saves all programs first
```

- One JVM (`com.xebyte.headless.GhidraMCPHeadlessServer`, `-Xmx12g`) holds the project lock. Do
  not run `analyzeHeadless` on project `a3` while it runs; stop it first.
- PID file `.work\ghidra-mcp-server.pid`, logs `.work\logs\ghidra-mcp-server.{out,err}.log`.
- `GHIDRA_MCP_ALLOW_SCRIPTS=1` is set, so `/run_script_inline` (Java snippets) works. Loopback only.
- Renames and comments are written into the shared project and survive restarts
  (`a3re.py rename/label/comment` save immediately).
- Another program in the same project: pass `--program NAME` to `a3re.py`.

## Shell CLI: `tools/re/a3re.py` (stdlib only)

```sh
python tools/re/a3re.py status
python tools/re/a3re.py decompile 0x8a6fc0          # RVA or VA (0x1408a6fc0) or a symbol name
python tools/re/a3re.py disasm 0x8a6fc0
python tools/re/a3re.py func-at 0x2e2380            # containing function, callers, callees
python tools/re/a3re.py xrefs 0x1b543d8             # references to an address (here a string)
python tools/re/a3re.py callees FUN_1408ab890
python tools/re/a3re.py strings "PhysX3 SDK" --limit 20
python tools/re/a3re.py read 0x1b543d8 32
python tools/re/a3re.py rtti '^(Man|Car|Tank)$'      # offline, from docs/re/rtti-classes.tsv
python tools/re/a3re.py vtable Man --limit 40       # slot -> function name
python tools/re/a3re.py sqf '^setDamage$'           # offline, from docs/re/sqf-commands.tsv
python tools/re/a3re.py rename 0x8a6fc0 SQF_diag_tickTime
python tools/re/a3re.py label 0x1b543d8 s_diag_tickTime
python tools/re/a3re.py comment 0x8a6fc0 "nular diag_tickTime: ms timer * 0.001"
python tools/re/a3re.py call search_instructions mnemonic=cmp operand_pattern=0x56657273
python tools/re/a3re.py call list_class_members class_name=Man
```

Any of the 190 GhidraMCP endpoints is reachable with `a3re.py call <endpoint> k=v ...` (add
`--post` for POST endpoints) or plain `curl "http://127.0.0.1:8089/<endpoint>?program=arma3_x64.exe&..."`.
List them: `curl http://127.0.0.1:8089/mcp/schema`. Rename warnings about PascalCase come from
the server's naming conventions; they do not block the rename.

## Offline extractors (no Ghidra needed; need `pip install pefile capstone`)

| Script | Output |
|---|---|
| `tools/re/rtti_dump.py <exe> --tsv out.tsv` | MSVC RTTI classes, vtables, bases (3 s) |
| `tools/re/msvc_demangle.py <mangled>...` | demangler for RTTI type names |
| `tools/re/sqf_commands.py <exe> --tsv out.tsv [--types t.tsv] [--long]` | script command table (10 s) |
| `tools/re/data_inventory.py` | `docs/re/data-inventory.md` from `A3_ROOT` |
| `tools/re/a3net.py selftest\|keys\|decode <hex>` | reference codec for the UDP transport, connect handshake and message encryption (`net-*.md`) |
| `tools/re/emu.py` | Unicorn harness: call functions of arma3_x64.exe with fake allocator/imports/TLS (needs the `.work\venv`) |
| `tools/re/verify_net_emu.py <exe>` | runs the original net crypto under emulation and compares with `a3net.py` |
| `tools/re/net_formats.py <exe> --tsv out` | runs the message-format registration under emulation → `docs/re/net-message-formats.tsv` |
| `tools/re/shdc.py list\|dump <file.shdc> [regex] [outdir]` | lists the compiled shaders in a shader cache (`Shaders_5_0_PS.shdc` etc. from `Dta\bin.pbo`) and dumps DXBC + disassembly (Windows `d3dcompiler_47.dll`; stdlib only). Used for `render-*.md` |

## MCP servers for interactive sessions (`.mcp.json`)

`.mcp.json` at the repo root registers three stdio servers. Sub-agents cannot attach MCP servers
mid-session; use `tools/re/mcp_call.py` to call any of them from a shell:

```sh
python tools/re/mcp_call.py ghidra-mcp --list
python tools/re/mcp_call.py ghidra-mcp force_decompile function=0x1408a6fc0
python tools/re/mcp_call.py ghidra-mcp search_strings search_term=BattlEye limit=5
python tools/re/mcp_call.py rea --list --verbose
```

| Server | Works? | Needs |
|---|---|---|
| `ghidra-mcp` (bridge `python -m bridge_mcp_ghidra`, venv `.work\tools\ghidra-mcp\.venv`) | yes | the headless server above on :8089. Without it the bridge only offers its 8 static tools. |
| `rea` (`node .work\tools\rea-npm\node_modules\rea-agents\scripts\rea.mjs mcp`, Ghidra provider) | yes, for small binaries | nothing running. rea imports and auto-analyses the target in a fresh temporary Ghidra project per session, with a 330 s startup deadline and no persistent annotations. On arma3_x64.exe it fails: `"Analysis took too long"`, `timeout_ms: 330000` (log `.work\logs\rea-arma3.json`). Useful for DLLs and small tools. |
| `ida-pro-mcp` (`python -m ida_pro_mcp.server`, venv `.work\venvs\ida-pro-mcp`) | **no** | IDA Pro. IDA Free 9.4 ships neither IDAPython (no `plugins\idapython*.dll`, no `python\`) nor idalib (`import idapro` → `Cannot load IDA library file idalib.dll`). The installer refuses because it finds `idafree_*.hexlic`: `IDA Free does not support plugins and cannot be used`. `tools/list` returns `Failed to complete request to IDA Pro … connection refused`. Kept registered so it works once an IDA Pro licence exists. |

Also evaluated: LaurieWired/GhidraMCP (needs the Ghidra GUI open), clearbluejar/pyghidra-mcp
(headless, but indexes decompiled code into chromadb on open, which is too slow for a 39 MB exe,
and has no RTTI/script endpoints). bethington/ghidra-mcp was chosen because its headless server
keeps one analysed project open, persists edits and exposes scripting.

## Rebuild from scratch

```powershell
# 1. analysis (≈20 min, 12 GB heap)
New-Item -ItemType Junction -Path P:\a3-ghidra -Target P:\a3-rust\.work\ghidra
$env:GHIDRA_HEADLESS_MAXMEM='12G'
P:\a3-rust\.resources\ghidra_12.1.4_PUBLIC\support\analyzeHeadless.bat P:\a3-ghidra a3 `
  -import P:\a3-rust\oirignal\arma3_x64.exe -max-cpu 8 -log P:\a3-rust\.work\logs\ghidra-import.log
# 2. GhidraMCP (Maven + JDK)
git clone https://github.com/bethington/ghidra-mcp P:\a3-rust\.work\tools\ghidra-mcp
cd P:\a3-rust\.work\tools\ghidra-mcp
python -m tools.setup ensure-prereqs --ghidra-path P:\a3-rust\.resources\ghidra_12.1.4_PUBLIC
python -m tools.setup build                      # GUI extension zip -> unzip into Ghidra\Extensions
mvn clean package -P headless -DskipTests        # headless server jar
uv sync --no-dev                                 # bridge venv (.venv)
# 3. rea
npm install --prefix P:\a3-rust\.work\tools\rea-npm rea-agents
# 4. ida-pro-mcp (installs, but needs IDA Pro)
python -m venv P:\a3-rust\.work\venvs\ida-pro-mcp
P:\a3-rust\.work\venvs\ida-pro-mcp\Scripts\python.exe -m pip install P:\a3-rust\.work\tools\ida-pro-mcp
```
