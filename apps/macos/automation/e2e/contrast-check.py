#!/usr/bin/env python3
"""contrast.sh's checks: the live colours through scripts/theme-contrast, and
the contact sheets.

    contrast-check.py ROOT SHOTS OUT THEME…

SHOTS is mockup.sh's output (automation.log and the captures), OUT where the
live colour files, the table and the sheets go. For each theme, the
`remember theme.colors` line the automation logged becomes OUT/live-<id>.json,
with a sample from the window capture for themes whose sidebar card is
vibrant: the card (a vibrancy view, so no token has its colour). Exit status 1 when a theme fails or a capture is missing.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw

root, shots, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
themes = sys.argv[4:]
tokens = json.loads((root / "shared/design/tokens.json").read_text())
log = (shots / "automation.log").read_text() if (shots / "automation.log").exists() else ""
failures: list[str] = []

# Where things sit at 1440 × 900 (points, top left): mockup.azs's window.
SIDEBAR_CARD = (60, 600, 140, 180)   # the card below the machine groups (themes.sh)


def capture(name: str, system_only: bool = False) -> Image.Image | None:
    """The system capture (vibrancy as drawn), else the app's own render
    (which can't draw vibrancy: no samples from it)."""
    for suffix in ("",) if system_only else ("", "-app"):
        path = shots / f"{name}{suffix}.png"
        if path.exists():
            img = Image.open(path).convert("RGB")
            return img
    return None


def scale(img: Image.Image) -> float:
    return img.size[0] / 1440


def mode(img: Image.Image, box: tuple[int, int, int, int]) -> str:
    """The commonest colour in a box (points)."""
    s = scale(img)
    x, y, w, h = box
    crop = img.crop((int(x * s), int(y * s), int((x + w) * s), int((y + h) * s)))
    counts = crop.getcolors(crop.size[0] * crop.size[1])
    rgb = max(counts)[1][:3]
    return "#%02x%02x%02x" % rgb


def regions_before(name: str) -> dict[str, tuple[int, ...]]:
    before = log.split(f"capture {name} ")[0]
    found = re.findall(r"remember theme\.regions = (\S+)", before)
    parts = dict(p.split("=", 1) for p in found[-1].split(";") if "=" in p) if found else {}
    return {k: tuple(int(n) for n in v.split(",")) for k, v in parts.items()}


# Live colours, in the order the themes ran.
live: dict[str, dict] = {}
for m in re.finditer(r"remember theme\.colors = (\{.*\})", log):
    d = json.loads(m.group(1))
    live[d["theme"]] = d

files = []
for theme in themes:
    d = live.get(theme)
    if d is None:
        failures.append(f"{theme}: no theme.colors in the automation log")
        continue
    samples = {}
    if tokens["themes"][theme].get("sidebarMaterial"):
        scene = capture(f"{theme}-scene", system_only=True)
        if scene is not None:
            samples["sample.sidebar"] = mode(scene, SIDEBAR_CARD)
            # Text on the card sits on what it shows, not on the token.
            d["colors"]["sidebar"] = samples["sample.sidebar"]
        else:
            spec = json.loads((root / "shared/design/contrast.json").read_text())
            d["colors"]["sidebar"] = spec["system"]["sample.sidebar"][d["appearance"]]
            print(f"NOTE {theme}: no system capture of the sidebar; its card is contrast.json's default")
    d["samples"] = samples
    path = out / f"live-{theme}.json"
    path.write_text(json.dumps(d, indent=1, sort_keys=True))
    files.append(path)

if files:
    run = subprocess.run([str(root / "scripts/theme-contrast"), "--table", "--live", *map(str, files)],
                         capture_output=True, text=True)
    (out / "table.txt").write_text(run.stdout)
    summary = [line for line in run.stdout.splitlines() if not line.startswith(("ok ", "FAIL ", "KNOWN "))]
    print("\n".join(summary))
    if run.returncode != 0:
        failures.append("theme-contrast failed (table: %s)" % (out / "table.txt"))

# Contact sheets: one per surface, every theme side by side.
SHEETS = {
    # name: (capture suffix, box in points or None for the whole window, output width per theme)
    "window": ("scene", None, 720),
    "sidebar": ("scene", (0, 0, 250, 900), 250),
    "topbar": ("scene", (230, 0, 1210, 56), 1400),
    "tiles": ("scene", (240, 44, 1200, 856), 600),
    "headers": ("scene", (240, 44, 1200, 120), 1400),
    "palette": ("palette", "palette", 560),
    "rail": ("rail", (0, 0, 240, 900), 160),
    "colours": ("colours", (240, 44, 1200, 856), 600),
    "selection": ("selection", (240, 44, 700, 420), 700),
    "hints": ("hints", (240, 44, 700, 420), 700),
    "files": ("files", (0, 0, 250, 900), 250),
}
sheets = out / "sheets"
sheets.mkdir(exist_ok=True)
for name, (suffix, box, width) in SHEETS.items():
    tiles = []
    for theme in themes:
        img = capture(f"{theme}-{suffix}")
        if img is None:
            continue
        s = scale(img)
        if box == "palette":
            b = regions_before(f"{theme}-{suffix}").get("palette")
            if not b:
                continue
            x, y, w, h = b
            b = (x - 20, y - 20, w + 40, h + 40)
        else:
            b = box or (0, 0, 1440, 900)
        x, y, w, h = b
        crop = img.crop((int(x * s), int(y * s), int((x + w) * s), int((y + h) * s)))
        crop = crop.resize((width, int(crop.size[1] * width / crop.size[0])), Image.LANCZOS)
        tiles.append((theme, crop))
    if not tiles:
        continue
    pad, label = 12, 22
    # Wide strips stack, one theme per row; the rest sit side by side.
    stack = tiles[0][1].size[0] > 2 * tiles[0][1].size[1]
    if stack:
        W = max(t.size[0] for _, t in tiles) + pad * 2
        H = sum(t.size[1] + label for _, t in tiles) + pad * (len(tiles) + 1)
    else:
        W = sum(t.size[0] for _, t in tiles) + pad * (len(tiles) + 1)
        H = max(t.size[1] for _, t in tiles) + pad * 2 + label
    sheet = Image.new("RGB", (W, H), (128, 128, 128))
    draw = ImageDraw.Draw(sheet)
    x, y = pad, pad
    for theme, t in tiles:
        draw.text((x, y - 6), theme, fill=(255, 255, 255))
        sheet.paste(t, (x, y + label - 6))
        if stack:
            y += t.size[1] + label + pad
        else:
            x += t.size[0] + pad
    sheet.save(sheets / f"{name}.png")
print(f"contrast: contact sheets in {sheets}")

for f in failures:
    print("FAIL " + f)
sys.exit(1 if failures else 0)
