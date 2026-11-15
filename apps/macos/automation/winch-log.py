#!/usr/bin/env python3
"""Runs in a herdr pane for reflow-lag.sh: logs every SIGWINCH and redraws.

  winch-log.py DIR    appends "<monotonic s> <cols> <rows>" to DIR/winch-<pane>.log

On every SIGWINCH it logs the time (time.monotonic, the same clock as the
app's CACurrentMediaTime) and the new size, then redraws the whole screen in
one write, like a TUI. A fast redraw, so what's measured is the path to and
from the program, not the program.
"""
import fcntl
import os
import signal
import struct
import sys
import termios
import time

pane = os.environ.get("HERDR_PANE_ID", str(os.getpid())).replace(":", "_")
log = open(os.path.join(sys.argv[1], f"winch-{pane}.log"), "a", buffering=1)
count = 0


def size() -> tuple[int, int]:
    rows, cols, _, _ = struct.unpack("HHHH", fcntl.ioctl(1, termios.TIOCGWINSZ, b"\0" * 8))
    return cols, rows


def draw() -> None:
    cols, rows = size()
    lines = ["\x1b[H\x1b[2J"]
    for r in range(rows - 1):
        lines.append(("%03d %s" % (r, "~" * cols))[:cols] + "\r\n")
    lines.append(("winch %d %dx%d" % (count, cols, rows))[:cols])
    os.write(1, "".join(lines).encode())


def on_winch(*_: object) -> None:
    global count
    now = time.monotonic()
    count += 1
    cols, rows = size()
    log.write("%.6f %d %d\n" % (now, cols, rows))
    draw()


signal.signal(signal.SIGWINCH, on_winch)
os.write(1, b"\x1b[?1049h")
draw()
while True:
    signal.pause()
