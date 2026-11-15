#!/usr/bin/env python3
"""Records every byte the terminal sends, raw, for drop.azs.in.

    pastecap.py OUT on|off [SECONDS]

`on` asks for bracketed paste (`CSI ? 2004 h`) the way shells and Claude
Code do; `off` leaves it off. The tty goes raw, so nothing is cooked on the
way; bytes are appended to OUT as they arrive. A lone ctrl+c or SECONDS
(default 120) ends it. drop-check.py reads OUT back.
"""
import os
import select
import sys
import termios
import time
import tty

out, mode = sys.argv[1], sys.argv[2]
seconds = float(sys.argv[3]) if len(sys.argv) > 3 else 120
fd = sys.stdin.fileno()
saved = termios.tcgetattr(fd)
tty.setraw(fd)
try:
    if mode == "on":
        os.write(1, b"\x1b[?2004h")
    open(out, "wb").close()
    os.write(1, f"pastecap ready (bracketed {mode})\r\n".encode())
    end = time.time() + seconds
    while time.time() < end:
        if not select.select([fd], [], [], 0.1)[0]:
            continue
        data = os.read(fd, 4096)
        if not data or data == b"\x03":
            break
        with open(out, "ab") as f:
            f.write(data)
        os.write(1, f"got {len(data)} bytes\r\n".encode())
finally:
    if mode == "on":
        os.write(1, b"\x1b[?2004l")
    termios.tcsetattr(fd, termios.TCSADRAIN, saved)
