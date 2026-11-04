#!/usr/bin/env bash
# Focus dim (dim.azs): unfocused tiles dim, the focused one stays bright,
# nothing dims with one tile on screen, the keyboard leaving the tiles
# changes nothing, the cross-fade is a fade with motion on and instant with
# it off. Twice: with Azulejo's default strength, then with Ghostty's
# unfocused-split-opacity and -fill set in the dev profile's own Ghostty
# config (never the user's). Dev instance e2e-dim.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-dim}" E2E_SUITE=D-dim
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
ghostty="$dev/config/ghostty/config"
: >"$out/check-output.txt"

# check_pixels RUN STRENGTH FILL: the title in each tile's header strip, as
# bright as it gets there (the strip's brightest pixel), against what the dim
# should make of it.
check_pixels() {
  python3 - "$out/$1" "$2" "$3" <<'PY'
import re, sys
from PIL import Image
d, strength, fill = sys.argv[1], float(sys.argv[2]), sys.argv[3]
fill = sum(int(fill[i:i + 2], 16) for i in (1, 3, 5)) / 3
log = open(f"{d}/automation.log").read()
frames, chrome = {}, {}
for m in re.finditer(r"frames (\S+): (.*)", log):
    tiles = {k: tuple(map(int, v)) for k, *v in re.findall(r"(\w+)=(-?\d+),(-?\d+),(\d+),(\d+)", m.group(2))}
    focus = re.search(r"focus=(\w+)", m.group(2)).group(1)
    frames[m.group(1)] = (tiles, focus)
for m in re.finditer(r"chrome (\S+): container=(\d+),(\d+),(\d+),(\d+)", log):
    chrome[m.group(1)] = tuple(map(int, m.groups()[1:]))

def title(name, tile):
    im = Image.open(f"{d}/{name}-app.png").convert("RGB")
    cx, cy, cw, ch = chrome[name]
    rx, ry, rw, rh = frames[name][0][tile]  # renders are @2x
    top = cy + ch - (ry + rh)
    box = (int((cx + rx + 4) * 2), int((top + 4) * 2), int((cx + rx + rw - 4) * 2), int((top + 24) * 2))
    px = im.crop(box).load()
    w, h = box[2] - box[0], box[3] - box[1]
    return max(sum(px[x, y]) / 3 for x in range(w) for y in range(h))

ok = True
def expect(what, value, lo, hi):
    global ok
    good = lo <= value <= hi
    ok &= good
    print(f"{'PASS' if good else 'FAIL'} {what}: {value:.0f} (want {lo:.0f}–{hi:.0f})")

tiles, focus = frames["dim-4-tiles"]
bright = title("dim-4-tiles", focus)
dimmed = bright * (1 - strength) + fill * strength
expect("focused title, 4 tiles", bright, 180, 255)
for t in tiles:
    if t != focus:
        expect(f"unfocused title {t}", title("dim-4-tiles", t), dimmed - 6, dimmed + 6)
for name in ("dim-zoom", "dim-monocle"):
    t, f = frames[name]
    expect(f"{name}: the one tile on screen", title(name, f), bright - 6, bright + 6)
# Mid cross-fade: the tile that lost focus and the one that got it are both
# on their way, neither at either end.
before = frames["dim-quickopen"][1]
after = frames["dim-fade-on"][1]
for t, role in ((before, "losing focus"), (after, "getting focus")):
    expect(f"mid-fade, the tile {role}", title("dim-fade-on", t), dimmed + 4, bright - 4)
# Motion off: at once.
expect("motion off, the tile losing focus", title("dim-fade-off", after), dimmed - 6, dimmed + 6)
expect("motion off, the tile getting focus", title("dim-fade-off", before), bright - 6, bright + 6)
sys.exit(0 if ok else 1)
PY
}

# A small repository for the sidebar's Files panel (the dev workspace's own
# root would be the home folder).
fx="$(mktemp -d "${TMPDIR:-/tmp}/azj-dim.XXXXXX")"
fx="$(cd "$fx" && pwd -P)"
printf '# fixture\n' >"$fx/README.md"
git -C "$fx" -c init.defaultBranch=main init -q
git -C "$fx" add -A
git -C "$fx" -c user.email=t@t -c user.name=t commit -qm init

reset_instance
write_config </dev/null
rm -f "$ghostty"
sed -e 's/@STRENGTH@/0.25/' -e "s|@FX@|$fx|" "$e2e_here/dim.azs" >"$out/dim-default.azs"
run_azs "$out/dim-default.azs" default
check "dim pixels (default 0.25 of the window colour)" check_pixels default 0.25 "#1b1d23"
reset_instance

# Ghostty's settings, from the profile's own Ghostty file.
mkdir -p "$(dirname "$ghostty")"
printf 'unfocused-split-opacity = 0.6\nunfocused-split-fill = #000000\n' >"$ghostty"
sed -e 's/@STRENGTH@/0.40/' -e "s|@FX@|$fx|" "$e2e_here/dim.azs" >"$out/dim-ghostty.azs"
run_azs "$out/dim-ghostty.azs" ghostty
check "dim pixels (Ghostty: opacity 0.6, fill #000000)" check_pixels ghostty 0.40 "#000000"
rm -f "$ghostty"
cat "$out/check-output.txt"
! grep -q FAIL "$checks"
