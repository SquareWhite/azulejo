#!/usr/bin/env python3
"""Takes a herdr pane over from another client, the way a phone or a second
Azulejo would: `herdr terminal attach <id> --takeover` in its own PTY, kept
attached for a while, then detached. Used by focus.azs (2.11) to put a tile
into its read-only "taken over" state without any OS input.

    takeover.py HERDR SESSION PANE SECONDS
"""
import fcntl
import json
import os
import pty
import select
import struct
import subprocess
import sys
import termios
import time

herdr, session, pane, seconds = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4])
env = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}
got = subprocess.run([herdr, "--session", session, "pane", "get", pane],
                     capture_output=True, text=True, env=env, check=True)
terminal = json.loads(got.stdout)["result"]["pane"]["terminal_id"]

pid, fd = pty.fork()
if pid == 0:
    # herdr's attach refuses a zero-sized PTY (ADR 0006).
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    os.environ.clear()
    os.environ.update(env)
    os.environ["TERM"] = "xterm-256color"
    os.execvp(herdr, [herdr, "--session", session, "terminal", "attach", terminal, "--takeover"])

# Keep reading so the attach never blocks on a full PTY.
deadline = time.time() + seconds
while time.time() < deadline:
    ready, _, _ = select.select([fd], [], [], 0.1)
    if ready:
        try:
            os.read(fd, 65536)
        except OSError:
            break
# The attach client ignores SIGTERM in raw mode: give it a moment, then KILL.
for sig in (15, 9):
    try:
        os.kill(pid, sig)
    except OSError:
        break
    for _ in range(10):
        try:
            done, _ = os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            done = pid
        if done == pid:
            sys.exit(0)
        time.sleep(0.1)
