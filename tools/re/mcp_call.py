"""Call any MCP server from `.mcp.json` over stdio, from a plain shell.

Sub-agents cannot attach MCP servers mid-session; this script lets them use the same servers.

Usage:
    python tools/re/mcp_call.py <server> --list                 # list tool names
    python tools/re/mcp_call.py <server> --list --verbose       # names + descriptions
    python tools/re/mcp_call.py <server> <tool> [json-args]     # call a tool, print result
    python tools/re/mcp_call.py <server> <tool> key=value ...   # args as key=value pairs

Examples:
    python tools/re/mcp_call.py ghidra-mcp --list
    python tools/re/mcp_call.py ghidra-mcp list_strings filter=PhysX limit=5
    python tools/re/mcp_call.py rea providers

Only the Python standard library is used. The server definition (command, args, env) is read
from `.mcp.json` at the repository root (or `--config`).
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class StdioClient:
    def __init__(self, spec: dict, timeout: float):
        env = os.environ.copy()
        env.update(spec.get("env", {}))
        self.timeout = timeout
        self.proc = subprocess.Popen(
            [spec["command"], *spec.get("args", [])],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env=env, cwd=spec.get("cwd") or str(ROOT),
        )
        self.next_id = 0
        self.stderr: list[bytes] = []
        threading.Thread(target=self._drain, daemon=True).start()

    def _drain(self):
        for line in self.proc.stderr:
            self.stderr.append(line)

    def send(self, msg: dict):
        self.proc.stdin.write((json.dumps(msg) + "\n").encode())
        self.proc.stdin.flush()

    def request(self, method: str, params: dict | None = None):
        self.next_id += 1
        rid = self.next_id
        self.send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params or {}})
        result: dict = {}

        def read():
            for raw in self.proc.stdout:
                raw = raw.strip()
                if not raw:
                    continue
                try:
                    msg = json.loads(raw)
                except json.JSONDecodeError:
                    continue
                if msg.get("id") == rid and ("result" in msg or "error" in msg):
                    result["msg"] = msg
                    return

        t = threading.Thread(target=read, daemon=True)
        t.start()
        t.join(self.timeout)
        if "msg" not in result:
            err = b"".join(self.stderr[-20:]).decode(errors="replace")
            raise TimeoutError(f"no response to {method} within {self.timeout}s\n{err}")
        msg = result["msg"]
        if "error" in msg:
            raise RuntimeError(json.dumps(msg["error"]))
        return msg["result"]

    def initialize(self):
        self.request("initialize", {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "a3-rust-mcp-call", "version": "0.1"},
        })
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(5)
        except Exception:
            self.proc.kill()


def parse_args(rest: list[str]) -> dict:
    if not rest:
        return {}
    if len(rest) == 1 and rest[0].lstrip().startswith("{"):
        return json.loads(rest[0])
    out = {}
    for kv in rest:
        k, _, v = kv.partition("=")
        try:
            out[k] = json.loads(v)
        except json.JSONDecodeError:
            out[k] = v
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("server")
    ap.add_argument("tool", nargs="?")
    ap.add_argument("args", nargs="*")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--verbose", action="store_true")
    ap.add_argument("--config", default=str(ROOT / ".mcp.json"))
    ap.add_argument("--timeout", type=float, default=600)
    a = ap.parse_args()

    servers = json.loads(Path(a.config).read_text(encoding="utf-8"))["mcpServers"]
    if a.server not in servers:
        sys.exit(f"unknown server {a.server!r}; known: {', '.join(servers)}")
    c = StdioClient(servers[a.server], a.timeout)
    try:
        c.initialize()
        if a.list or not a.tool:
            tools, cursor = [], None
            while True:
                r = c.request("tools/list", {"cursor": cursor} if cursor else {})
                tools += r.get("tools", [])
                cursor = r.get("nextCursor")
                if not cursor:
                    break
            for t in tools:
                if a.verbose:
                    print(f"{t['name']}: {(t.get('description') or '').splitlines()[0] if t.get('description') else ''}")
                    print(f"    args: {', '.join((t.get('inputSchema') or {}).get('properties', {}).keys())}")
                else:
                    print(t["name"])
            print(f"({len(tools)} tools)", file=sys.stderr)
            return
        r = c.request("tools/call", {"name": a.tool, "arguments": parse_args(a.args)})
        for item in r.get("content", []):
            if item.get("type") == "text":
                print(item["text"])
            else:
                print(json.dumps(item))
        if r.get("structuredContent") and not r.get("content"):
            print(json.dumps(r["structuredContent"], indent=1))
        if r.get("isError"):
            sys.exit(1)
    finally:
        c.close()


if __name__ == "__main__":
    main()
