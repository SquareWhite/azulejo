#!/usr/bin/env python3
"""Runs in a herdr pane for resize-probe.sh.

On every SIGWINCH (or SIGUSR1, a redraw with no resize): log "<n> <monotonic
s> <cols> <rows>" to LOG and redraw the whole screen in one write, ending
with a token of eight copies of one letter (A–Z by n), which herdr's diffed
frames carry whole. LOG.pid holds this process's pid.
"""
import fcntl
import os
import signal
import struct
import sys
import termios
import time

log = open(sys.argv[1], "a", buffering=1)
n = 0


def size() -> tuple[int, int]:
    r, c, _, _ = struct.unpack("HHHH", fcntl.ioctl(1, termios.TIOCGWINSZ, b"\0" * 8))
    return c, r


def draw(tag: str) -> None:
    c, r = size()
    lines = ["\x1b[H\x1b[2J"]
    for i in range(r - 1):
        lines.append(("row%02d " % i + "=" * max(0, c - 6))[:c] + "\r\n")
    lines.append(tag)
    os.write(1, "".join(lines).encode())


def on_signal(*_: object) -> None:
    global n
    t = time.monotonic()
    n += 1
    c, r = size()
    log.write("%d %.6f %d %d\n" % (n, t, c, r))
    draw(chr(65 + n % 26) * 8)


signal.signal(signal.SIGWINCH, on_signal)
signal.signal(signal.SIGUSR1, on_signal)
open(sys.argv[1] + ".pid", "w").write(str(os.getpid()))
draw("-" * 8)
while True:
    signal.pause()
