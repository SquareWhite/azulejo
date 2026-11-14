#!/usr/bin/env python3
"""Check which escape sequences survive herdr's direct-attach frame stream.

Usage: passthrough_probe.py PANE_ID [MODE]   (MODE = observe | control, default observe)
Env: HERDR_SESSION (required, e.g. azj-probe).

Starts `herdr terminal session <MODE>` on the pane, runs each passthrough.sh case
inside the pane via `herdr pane run`, and reports whether the expected bytes
appeared in the frames that followed. Also records herdr events per case.
"""
import base64
import json
import os
import socket
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
SESSION = os.environ["HERDR_SESSION"]

CASES = [
    ("truecolor", [b"38;2;255;100;0"]),
    ("color256", [b"38;5;196"]),
    ("undercurl", [b"4:3", b"58:2", b"58;2"]),
    ("osc8", [b"https://example.com/azj"]),
    ("osc52", [b"]52;"]),
    ("kittygfx", [b"\x1b_G"]),
    ("osc9", [b"]9;"]),
    ("osc777", [b"]777;"]),
    ("osc99", [b"]99;"]),
    ("bell", [b"\x07"]),
    ("title", [b"AZJ-TITLE"]),
    ("osc7", [b"]7;"]),
    ("cursor_bar", [b"[5 q"]),
    # Input modes: switched on and off inside one command (see passthrough.sh).
    ("bpaste_on pause bpaste_off", [b"?2004h"]),
    ("kkp_on pause kkp_off", [b">1u", b"=1u", b"=1;"]),
    ("mouse_on pause mouse_off", [b"?1000h", b"?1006h"]),
    ("appcursor pause appcursor_off", [b"?1h"]),
    ("focus_on pause focus_off", [b"?1004h"]),
]


def clean_env() -> dict:
    return {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}


def herdr(*args: str) -> str:
    return subprocess.run(["herdr", "--session", SESSION, *args], capture_output=True, text=True, env=clean_env()).stdout


def socket_path() -> str:
    out = subprocess.run(["herdr", "--session", SESSION, "status"], capture_output=True, text=True, env=clean_env()).stdout
    for line in out.splitlines():
        if line.strip().startswith("socket:"):
            return line.split(":", 1)[1].strip()
    raise SystemExit("no socket in herdr status")


def main() -> None:
    pane = sys.argv[1]
    mode = sys.argv[2] if len(sys.argv) > 2 else "observe"
    argv = ["herdr", "--session", SESSION, "terminal", "session", mode, pane, "--cols", "100", "--rows", "30"]
    if mode == "control":
        argv.insert(-4, "--takeover")
    proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=clean_env())
    frames: list[tuple[float, bytes]] = []
    events: list[tuple[float, dict]] = []

    def read_frames() -> None:
        for line in proc.stdout:
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            if rec.get("type") == "terminal.frame":
                frames.append((time.time(), base64.b64decode(rec["bytes"])))

    def read_events() -> None:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(socket_path())
        subs = [{"type": t} for t in ("pane.updated", "pane.exited", "workspace.updated", "workspace.metadata_updated")]
        subs.append({"type": "pane.agent_status_changed", "pane_id": pane})
        s.sendall((json.dumps({"id": "pt", "method": "events.subscribe", "params": {"subscriptions": subs}}) + "\n").encode())
        buf = b""
        while True:
            chunk = s.recv(65536)
            if not chunk:
                return
            buf += chunk
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                events.append((time.time(), json.loads(line)))

    threading.Thread(target=read_frames, daemon=True).start()
    threading.Thread(target=read_events, daemon=True).start()
    time.sleep(1.0)
    script = os.path.join(HERE, "passthrough.sh")
    results = []
    for name, needles in CASES:
        t0 = time.time()
        herdr("pane", "run", pane, f"sh {script} {name}")
        time.sleep(0.8 + 1.0 * name.count("pause"))
        t1 = time.time()
        data = b"".join(b for t, b in frames if t0 <= t < t1)
        evs = [e for t, e in events if t0 <= t < t1 and "event" in e]
        hit = [n.decode(errors="replace") for n in needles if n in data]
        ev_summary = sorted({e["event"] for e in evs})
        changed = {}
        for e in evs:
            pane_data = e.get("data", {}).get("pane", {})
            for k in ("terminal_title", "cwd", "foreground_cwd", "agent_status"):
                if k in pane_data:
                    changed[k] = pane_data[k]
        results.append({"case": name, "passes": bool(hit), "matched": hit, "frame_bytes": len(data),
                        "events": ev_summary, "pane_fields": changed})
    proc.terminate()
    err = proc.stderr.read().decode(errors="replace").strip()
    for r in results:
        print(json.dumps(r))
    if err:
        print(json.dumps({"stderr": err}))


if __name__ == "__main__":
    main()
