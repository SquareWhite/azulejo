#!/usr/bin/env python3
"""Shows the bytes each keypress sends, as hex: `got 1b5b31333b3275`.

    keyecho.py [FLAGS] [SECONDS]

FLAGS > 0 pushes those kitty keyboard protocol flags first (`CSI > FLAGS u`),
the way Claude Code and fish do; 0 (the default) leaves the terminal in
legacy mode. The tty goes raw, so nothing is cooked on the way. `q` or
SECONDS (default 30) ends it. focus.azs reads the lines back with
pane-has.sh.
"""
import os
import select
import sys
import termios
import time
import tty

flags = int(sys.argv[1]) if len(sys.argv) > 1 else 0
seconds = float(sys.argv[2]) if len(sys.argv) > 2 else 30
fd = sys.stdin.fileno()
saved = termios.tcgetattr(fd)
tty.setraw(fd)
try:
    if flags:
        os.write(1, f"\x1b[>{flags}u".encode())
    os.write(1, f"keyecho ready (kitty flags {flags})\r\n".encode())
    end = time.time() + seconds
    while time.time() < end:
        if not select.select([fd], [], [], 0.1)[0]:
            continue
        data = os.read(fd, 1024)
        if not data:
            break
        os.write(1, f"got {data.hex()}\r\n".encode())
        if data == b"q":
            break
finally:
    if flags:
        os.write(1, b"\x1b[<u")
    termios.tcsetattr(fd, termios.TCSADRAIN, saved)
