#!/usr/bin/env python3
"""Stray pixels in mid-motion captures.

    apps/macos/automation/outside.py OUT_DIR [--scale 2]

For each `capture NAME` in OUT_DIR/automation.log, reads the app's render
(NAME-app.png) and counts bright pixels in the tile area's rows that lie
outside every tile's on-screen frame and outside the sidebar card, as logged
at that instant ("frames NAME: …", "chrome NAME: container=… card=…").
Content drawn past a card's clip (a snapshot, a header, a ghost) shows up
here. The scan covers the whole window width, so tiles sliding in or out
with the sidebar are checked too. Prints one line per capture and a total;
exits 1 when any capture has stray pixels.

Successor of .dev/clipfix-evidence/outside.py, which assumed a fixed tile
area and a focus ring.
"""
import re
import sys

from PIL import Image

RECT = re.compile(r"(\w+)=(-?\d+),(-?\d+),(\d+),(\d+)")


def main():
    args = sys.argv[1:]
    if not args:
        sys.exit(__doc__)
    out = args[0]
    scale = float(args[args.index("--scale") + 1]) if "--scale" in args else 2.0
    log = open(f"{out}/automation.log").read()
    chrome = {}
    for m in re.finditer(r"chrome (\S+): (.*)", log):
        parts = {k: tuple(map(int, v)) for k, *v in RECT.findall(m.group(2))}
        chrome[m.group(1)] = parts
    total = 0
    checked = 0
    for m in re.finditer(r"frames (\S+): (.*)", log):
        name = m.group(1)
        tiles = [tuple(map(int, v)) for k, *v in RECT.findall(m.group(2))]
        c = chrome.get(name, {})
        if "container" not in c:
            continue
        cx, cy, cw, ch = c["container"]  # window points, top-left origin
        allowed = tiles + ([c["card"]] if "card" in c else [])  # container points, bottom-up
        try:
            im = Image.open(f"{out}/{name}-app.png").convert("RGB")
        except FileNotFoundError:
            continue
        px = im.load()
        width = im.size[0] / scale
        bad, first = 0, None
        step = 1 / scale
        y = cy + 2
        while y < cy + ch - 2:
            x = 2.0
            while x < width - 2:
                # container coordinates, bottom-up
                tx, ty = x - cx, ch - (y - cy)
                inside = any(rx - 2 <= tx <= rx + rw + 2 and ry - 2 <= ty <= ry + rh + 2
                             for rx, ry, rw, rh in allowed)
                if not inside:
                    r, g, b = px[int(x * scale), int(y * scale)]
                    if r + g + b > 130:
                        bad += 1
                        if first is None:
                            first = (round(x), round(y), (r, g, b))
                x += step
            y += step
        checked += 1
        total += bad
        print(f"{name:28s} outside-bright={bad} first={first}")
    print(f"captures checked: {checked}; stray bright pixels: {total}")
    sys.exit(1 if total else 0)


if __name__ == "__main__":
    main()
