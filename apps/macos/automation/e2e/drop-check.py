#!/usr/bin/env python3
"""Checks what a drop typed into the terminal (pastecap.py's capture), then
empties the capture for the next drop.

    drop-check.py CAP EXPECTED on|off
        waits up to 4 s for CAP to hold exactly EXPECTED's bytes: framed as a
        bracketed paste (ESC [200~ … ESC [201~) when `on`
    drop-check.py --image CAP DIR SOURCE on|off
        CAP holds one escaped path to a PNG in DIR with SOURCE's pixel size

Prints what it saw on failure (exit 1).
"""
import os
import subprocess
import sys
import time

START, END = b"\x1b[200~", b"\x1b[201~"


def frame(body: bytes, mode: str) -> bytes:
    return START + body + END if mode == "on" else body


def wait_for(cap: str, done) -> bytes:
    got = b""
    for _ in range(40):
        try:
            got = open(cap, "rb").read()
        except FileNotFoundError:
            got = b""
        if done(got):
            break
        time.sleep(0.1)
    # Anything still on its way lands in this one, not the next check.
    time.sleep(0.2)
    got = open(cap, "rb").read()
    open(cap, "wb").close()
    return got


def size(path: str) -> tuple[str, str]:
    text = subprocess.run(["sips", "-g", "pixelWidth", "-g", "pixelHeight", path],
                          capture_output=True, text=True).stdout
    vals = dict(line.split(":") for line in text.splitlines()[1:] if ":" in line)
    return vals.get("  pixelWidth", "?").strip(), vals.get("  pixelHeight", "?").strip()


def main() -> int:
    if sys.argv[1] == "--image":
        cap, folder, source, mode = sys.argv[2:6]
        got = wait_for(cap, lambda b: b.endswith(END) if mode == "on" else b.endswith(b".png"))
        body = got
        if mode == "on":
            if not (got.startswith(START) and got.endswith(END)):
                print(f"not a bracketed paste: {got!r}", file=sys.stderr)
                return 1
            body = got[len(START):-len(END)]
        path = body.decode()
        if os.path.realpath(os.path.dirname(path)) != os.path.realpath(folder) or not path.endswith(".png"):
            print(f"expected a .png in {folder}, got {got!r}", file=sys.stderr)
            return 1
        if not os.path.isfile(path) or open(path, "rb").read(8) != b"\x89PNG\r\n\x1a\n":
            print(f"{path} isn't a PNG file", file=sys.stderr)
            return 1
        if size(path) != size(source):
            print(f"{path} is {size(path)}, {source} is {size(source)}", file=sys.stderr)
            return 1
        print(path)
        return 0
    cap, expected, mode = sys.argv[1:4]
    want = frame(open(expected, "rb").read(), mode)
    got = wait_for(cap, lambda b: len(b) >= len(want))
    if got != want:
        print(f"want {want!r}\n got {got!r}", file=sys.stderr)
        return 1
    return 0


sys.exit(main())
