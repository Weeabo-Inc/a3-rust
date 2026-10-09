"""Minimal client for the transport-level connect handshake (docs/re/net-handshake.md).

Builds HELLO and CONNECT with the reference codec in `tools/re/a3net.py` (an independent
implementation of the same document) and prints what a server answers, so a real handshake can be
observed against `apps/a3-server` without the game client. Standard library only.

Usage:
    python tools/re/a3net_probe.py [host] [port] [--name NAME] [--password PW] [--build N]

Exit status is 0 when the server accepted the connect (RESULT code 0), 1 otherwise.
"""

from __future__ import annotations

import argparse
import os
import socket
import struct
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import a3net  # noqa: E402  (path set up above)

STEAM_ID = 76561197960287930
CLIENT_SERIAL = 1000
RESULT_NAMES = {
    0: "accepted",
    1: "bad password",
    2: "version/build mismatch",
    3: "server error",
    4: "full / no id",
    5: "anti-cheat rejected",
    6: "check-string mismatch",
}


def describe(header: a3net.Header, payload: bytes) -> str:
    flags = "|".join(
        name
        for bit, name in (
            (a3net.F_RELIABLE, "RELIABLE"),
            (a3net.F_NOCHANNEL, "NOCHANNEL"),
            (a3net.F_CONTROL, "CONTROL"),
            (0x0001, "control-msg"),
        )
        if header.flags & bit
    )
    return f"flags={header.flags:#06x} ({flags}) serial={header.serial} len={len(payload)}"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("host", nargs="?", default="127.0.0.1")
    parser.add_argument("port", nargs="?", type=int, default=2302)
    parser.add_argument("--name", default="a3net probe")
    parser.add_argument("--password", default="")
    parser.add_argument("--build", type=int, default=a3net.BUILD)
    parser.add_argument("--timeout", type=float, default=3.0)
    args = parser.parse_args()

    keys = a3net.derive_keys()
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.settimeout(args.timeout)
    server = (args.host, args.port)
    serial = CLIENT_SERIAL

    hello = struct.pack("<III", a3net.CTRL_HELLO, a3net.MAGIC, a3net.VERSION)
    sock.sendto(a3net.pack(a3net.F_CONTROL | 1, serial, 0, 0, 0, hello, keys), server)
    serial += 1
    data, _ = sock.recvfrom(a3net.MAX_DATAGRAM)
    header, payload = a3net.unpack(data, keys)
    magic = struct.unpack_from("<I", payload, 0)[0]
    if magic != a3net.CTRL_CHALLENGE:
        print(f"unexpected answer to HELLO: {magic:#010x}", file=sys.stderr)
        return 1
    challenge = struct.unpack_from("<I", payload, 4)[0]
    print(f"HELLO     -> {a3net.CTRL_HELLO:#010x} magic {a3net.MAGIC:#010x} version {a3net.VERSION}")
    print(f"CHALLENGE <- {describe(header, payload)} challenge={challenge:#010x}")

    connect = a3net.connect_request(
        args.name, args.password, "", STEAM_ID, 0x1000, challenge=challenge, actual=a3net.VERSION
    )
    # `--build` lets the probe show a version rejection as well as an accepted join.
    connect = connect[:0x91] + struct.pack("<I", args.build) + connect[0x95:]
    sock.sendto(a3net.pack(a3net.F_CONTROL | 1, serial, 0, 0, 0, connect, keys), server)
    serial += 1
    print(f"CONNECT   -> {len(connect)} bytes, challenge {challenge:#010x}, build {args.build}")

    deadline = time.time() + args.timeout
    while time.time() < deadline:
        data, _ = sock.recvfrom(a3net.MAX_DATAGRAM)
        header, payload = a3net.unpack(data, keys)
        message = struct.unpack_from("<I", payload, 0)[0]
        if message == a3net.CTRL_RESULT:
            code, player_id = struct.unpack_from("<II", payload, 4)
            print(
                f"RESULT    <- {describe(header, payload)} code={code} "
                f"({RESULT_NAMES.get(code, '?')}) player id={player_id}"
            )
            # Acknowledge the reliable RESULT the way the client's channel would, then leave.
            sock.sendto(a3net.pack(0, serial, header.serial, 0, 0, b"", keys), server)
            return 0 if code == 0 else 1
        print(f"ignoring an unexpected answer: {message:#010x}")
    print("no RESULT within the timeout", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
