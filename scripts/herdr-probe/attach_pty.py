#!/usr/bin/env python3
"""Run `herdr terminal attach` under a pseudo-terminal and capture what it writes.

Usage: attach_pty.py TERMINAL_ID|tui SECONDS OUT_FILE [--takeover] [--cols N] [--rows N]
       [--send-after S HEXBYTES]... [--resize-after S COLS ROWS]...
Env: HERDR_SESSION (required).

Stands in for a terminal emulator (libghostty/VTE) hosting the attach client:
records every byte the client emits (OUT_FILE), can type bytes into it and
resize the PTY (SIGWINCH) at given times. TERMINAL_ID "tui" runs the full
session client (`herdr --session S`, what a phone over SSH runs) instead. Prints a JSON summary with
timestamps of output bursts.
"""
import fcntl
import json
import os
import pty
import select
import signal
import struct
import sys
import termios
import time


def set_size(fd: int, cols: int, rows: int) -> None:
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def main() -> None:
    term_id, seconds, out_file, *rest = sys.argv[1:]
    seconds = float(seconds)
    cols, rows, takeover = 100, 30, False
    sends, resizes = [], []
    i = 0
    while i < len(rest):
        a = rest[i]
        if a == "--takeover":
            takeover = True; i += 1
        elif a == "--cols":
            cols = int(rest[i + 1]); i += 2
        elif a == "--rows":
            rows = int(rest[i + 1]); i += 2
        elif a == "--send-after":
            sends.append((float(rest[i + 1]), bytes.fromhex(rest[i + 2]))); i += 3
        elif a == "--resize-after":
            resizes.append((float(rest[i + 1]), int(rest[i + 2]), int(rest[i + 3]))); i += 4
        else:
            raise SystemExit(f"bad arg {a}")
    argv = ["herdr", "--session", os.environ["HERDR_SESSION"]]
    if term_id != "tui":
        argv += ["terminal", "attach", term_id]
    if takeover:
        argv.append("--takeover")
    env = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}
    env["TERM"] = "xterm-ghostty" if os.environ.get("AZJ_TERM_GHOSTTY") else "xterm-256color"
    pid, fd = pty.fork()
    if pid == 0:
        os.execvpe(argv[0], argv, env)
    set_size(fd, cols, rows)
    os.kill(pid, signal.SIGWINCH)
    start = time.time()
    bursts = []
    sends.sort(); resizes.sort()
    with open(out_file, "wb") as out:
        while time.time() - start < seconds:
            now = time.time() - start
            while sends and sends[0][0] <= now:
                os.write(fd, sends.pop(0)[1])
            while resizes and resizes[0][0] <= now:
                _, c, r = resizes.pop(0)
                set_size(fd, c, r)
                os.kill(pid, signal.SIGWINCH)
            r, _, _ = select.select([fd], [], [], 0.02)
            if r:
                try:
                    data = os.read(fd, 65536)
                except OSError:
                    break
                if not data:
                    break
                out.write(data)
                bursts.append((round(now, 4), len(data)))
    exited = None
    try:
        wpid, status = os.waitpid(pid, os.WNOHANG)
        exited = status if wpid else None
    except ChildProcessError:
        exited = "reaped"
    if exited is None:
        os.kill(pid, signal.SIGTERM)
        time.sleep(0.2)
        try:
            os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            pass
    print(json.dumps({"bursts": bursts[:200], "burst_count": len(bursts), "client_exited_status": exited}))


if __name__ == "__main__":
    main()
