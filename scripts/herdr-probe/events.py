#!/usr/bin/env python3
"""Subscribe to herdr events and print each one with a receive timestamp.

Usage: events.py SOCKET_PATH [SECONDS] [TYPE[@PANE_ID] ...]
Prints one JSON object per line: {"t": <unix seconds>, "event": <raw line>}.
"""
import json
import socket
import sys
import time

DEFAULT_TYPES = [
    "workspace.created", "workspace.closed", "tab.created", "tab.closed",
    "pane.created", "pane.closed", "pane.updated", "pane.exited",
    "pane.focused", "pane.moved", "pane.agent_detected", "layout.updated",
]


def parse(spec: str) -> dict:
    # "type" or "type@pane_id" (pane.agent_status_changed and pane.scroll_changed need a pane).
    kind, _, pane = spec.partition("@")
    return {"type": kind, "pane_id": pane} if pane else {"type": kind}


def main() -> None:
    path = sys.argv[1]
    seconds = float(sys.argv[2]) if len(sys.argv) > 2 else 10.0
    types = sys.argv[3:] or DEFAULT_TYPES
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(path)
    req = {
        "id": "probe:sub",
        "method": "events.subscribe",
        "params": {"subscriptions": [parse(t) for t in types]},
    }
    sock.sendall((json.dumps(req) + "\n").encode())
    sock.settimeout(0.2)
    deadline = time.time() + seconds
    buf = b""
    while time.time() < deadline:
        try:
            chunk = sock.recv(65536)
        except socket.timeout:
            continue
        if not chunk:
            break
        buf += chunk
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            print(json.dumps({"t": time.time(), "event": line.decode(errors="replace")}), flush=True)


if __name__ == "__main__":
    main()
