#!/usr/bin/env python3
"""Runs in a herdr pane for resize.sh: a bottom-anchored block that redraws
on every SIGWINCH the way Ink (Claude Code's UI) does: erase the lines it
drew last time with relative cursor moves, then draw them again at the new
size. Some static history above it, as a conversation would leave."""
import fcntl
import os
import signal
import struct
import termios
import sys
import time

HEIGHT = int(sys.argv[1]) if len(sys.argv) > 1 else 4
LOG = open(sys.argv[2], "a", buffering=1) if len(sys.argv) > 2 else None
drawn = 0
redraws = 0


def size() -> tuple[int, int]:
    rows, cols, _, _ = struct.unpack("HHHH", fcntl.ioctl(1, termios.TIOCGWINSZ, b"\0" * 8))
    return cols, rows


def draw() -> None:
    global drawn, redraws
    cols, rows = size()
    if LOG:
        LOG.write("%.3f %d %d\n" % (time.monotonic(), cols, rows))
    out = ("\r" + "\x1b[2K\x1b[1A" * (drawn - 1) + "\x1b[2K") if drawn else ""
    width = max(cols - 2, 4)
    inner = ["| prompt block, redraw %d at %dx%d" % (redraws, cols, rows)]
    inner += ["| item %d" % i for i in range(1, HEIGHT - 3)] + ["| >"]
    block = ["+" + "-" * width + "+"] + [l.ljust(cols - 1)[: cols - 1] + "|" for l in inner] + ["+" + "-" * width + "+"]
    out += "\n".join(line[:cols] for line in block)
    os.write(1, out.encode())
    drawn = len(block)
    redraws += 1


for i in range(1, 61):
    os.write(1, ("history line %d\n" % i).encode())
draw()
signal.signal(signal.SIGWINCH, lambda *_: draw())
while True:
    time.sleep(3600)
