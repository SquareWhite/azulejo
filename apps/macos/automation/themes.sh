#!/usr/bin/env bash
# Themes, live (docs/themes.md): the mockup's scene (mockup.sh's fixtures, on
# its own dev instance, AZJ_DEV_NAME=thm) switched through all four built-in
# themes while running, by editing the profile's config file and through ⌘K
# "Set theme…" (with the arrow-key preview and Esc), then auto with the app's
# own system-appearance override, a user theme file and its hot reload, and
# the Ghostty colour precedence. Never changes the Mac's appearance.
#
#   apps/macos/automation/themes.sh [--keep]
#
# Checks: every switch leaves every PTY its size and every terminal attached
# (themes.azs.in); then, in the app's own render of each theme, the sidebar,
# tab strip, the focused tile's header, its terminal, the preview page, the
# ⌘K menu and the find bar each show the new theme's colour (from
# shared/design/tokens.json) and not the previous theme's; going back to
# atom-one-dark gives the first capture's pixels again. Writes small copies
# of the four captures to docs/design/theme-<id>.png with --docs.
# Output: .dev/thm.out/mockup/.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-thm}"
keep="" docs=0
for a in "$@"; do
  case "$a" in
    --keep) keep="--keep" ;;
    --docs) docs=1 ;;
    *) echo "themes.sh: unknown argument $a" >&2; exit 2 ;;
  esac
done
out="$root/.dev/$AZJ_DEV_NAME.out/mockup"
tail="$(mktemp "${TMPDIR:-/tmp}/azj-themes.XXXXXX")"
# The profile's user themes and Ghostty file: none left from an earlier run.
profile="$root/.dev/$AZJ_DEV_NAME/config"
clean() { rm -rf "$profile/azulejo/themes" "$profile/ghostty/config"; }
clean
trap 'rm -f "$tail"; [[ -n "$keep" ]] || clean' EXIT
sed -e "s|@HERE@|$here/themes|g" "$here/themes/themes.azs.in" >"$tail"

status=0
AZJ_MOCKUP_TAIL="$tail" AZJ_MOCKUP_TIMEOUT=600 "$here/mockup.sh" $keep || status=1
echo "themes: automation $(cat "$out/result" 2>/dev/null || echo 'no result')"

python3 - "$root" "$out" "$docs" <<'PY' || status=1
import json, re, sys
from pathlib import Path
from PIL import Image

root, out, docs = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3] == "1"
tokens = json.loads((root / "shared/design/tokens.json").read_text())
log = (out / "automation.log").read_text()

def hexrgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4)), (int(h[6:8], 16) if len(h) == 8 else 255)

def colour(theme, token):
    v = tokens["themes"][theme]["color"][token]
    return hexrgb(v if isinstance(v, str) else v["hex"])

def over(top, alpha, under):
    return tuple(round(t * alpha / 255 + u * (1 - alpha / 255)) for t, u in zip(top, under))

def solid(theme, token, under_token="window"):
    rgb, a = colour(theme, token)
    if a == 255:
        return rgb
    under, _ = colour(theme, under_token)
    return over(rgb, a, under)

def dim(theme, rgb):
    fill, _ = colour(theme, "dimFill")
    return over(fill, round(255 * tokens["alpha"]["unfocusedDim"]), rgb)

def mode(img, box):
    """The commonest colour in a box (points), as the 2x capture has it."""
    x, y, w, h = box
    crop = img.crop((x * 2, y * 2, (x + w) * 2, (y + h) * 2))
    counts = crop.getcolors(crop.size[0] * crop.size[1])
    return max(counts)[1][:3]

def near(a, b, tol):
    return max(abs(p - q) for p, q in zip(a, b)) <= tol

def regions(name):
    """`remember theme.regions = palette=…;find=…` logged before capture `name`."""
    before = log.split(f"capture {name} ")[0]
    found = re.findall(r"remember theme\.regions = (\S+)", before)
    parts = dict(p.split("=", 1) for p in found[-1].split(";") if "=" in p) if found else {}
    return {k: tuple(int(n) for n in v.split(",")) for k, v in parts.items()}

failures = []
# Where things sit in the mockup scene at 1440 × 900 (points, top left).
SIDEBAR = (60, 600, 140, 180)      # the card below the machine groups
TABSTRIP = (507, 22, 14, 18)       # the strip's pill, between "release" and "+"
GUTTER = (600, 15, 400, 30)        # the window colour, between the tabs and the search field
HEADER = (260, 54, 160, 22)        # the focused claude tile's strip, left of its title
TERMINAL = (640, 700, 200, 140)    # the claude terminal, below its last line
PREVIEW = (890, 815, 230, 60)      # the preview page, below its text

themes = [("t1", "atom-one-dark"), ("t2", "atom-one-light"), ("t3", "macos-dark"), ("t4", "macos-light")]
seen = {}
for tag, theme in themes:
    shot = out / f"{tag}-{theme}-app.png"
    if not shot.exists():
        failures.append(f"{theme}: no capture {shot.name}")
        continue
    img = Image.open(shot).convert("RGB")
    pal = Image.open(out / f"{tag}-{theme}-palette-app.png").convert("RGB")
    find = Image.open(out / f"{tag}-{theme}-find-app.png").convert("RGB")
    t = tokens["themes"][theme]
    term_bg, _ = hexrgb(t["terminal"]["background"])
    vibrant = t.get("sidebarMaterial") is not None
    samples = {
        "sidebar": (mode(img, SIDEBAR), None if vibrant else solid(theme, "sidebar")),
        "tab strip": (mode(img, TABSTRIP), solid(theme, "tabStrip")),
        "header": (mode(img, HEADER), solid(theme, "header", "tile")),
        "terminal": (mode(img, TERMINAL), term_bg),
        "preview": (mode(img, PREVIEW), dim(theme, solid(theme, "tileAlt", "tile"))),
    }
    r = regions(f"{tag}-{theme}-palette")
    if "palette" in r:
        x, y, w, h = r["palette"]
        samples["palette"] = (mode(pal, (x + 20, y + h - 40, w - 40, 30)), None)
    else:
        failures.append(f"{theme}: the palette's frame wasn't logged")
    r = regions(f"{tag}-{theme}-find")
    if "find" in r:
        x, y, w, h = r["find"]
        samples["find bar"] = (mode(find, (x + w - 60, y + 3, 50, h - 6)), solid(theme, "browserChrome", "tile"))
    else:
        failures.append(f"{theme}: the find bar's frame wasn't logged")
    print(f"{theme}:")
    # The rail: its card, the selected workspace's fill and a blocked
    # agent's tint, somewhere down the column.
    rail_shot = out / f"{tag}-{theme}-rail-app.png"
    if rail_shot.exists():
        rail = Image.open(rail_shot).convert("RGB")
        column = rail.crop((10 * 2, 70 * 2, 46 * 2, 880 * 2))
        colours = {c[1][:3] for c in column.getcolors(column.size[0] * column.size[1])}
        card = mode(rail, (12, 700, 32, 150))
        sel = solid(theme, "selected", "sidebar") if vibrant else None
        if not vibrant:
            sel_rgb, sel_a = colour(theme, "selected")
            sel = over(sel_rgb, sel_a, solid(theme, "sidebar"))
            if not near(card, solid(theme, "sidebar"), 3):
                failures.append(f"{theme} rail card: #{bytes(card).hex()} is not #{bytes(solid(theme, 'sidebar')).hex()}")
        else:
            sel_rgb, sel_a = colour(theme, "selected")
            sel = over(sel_rgb, sel_a, card)
        blk_rgb, _ = colour(theme, "statusBlocked")
        tint = over(blk_rgb, round(255 * tokens["alpha"]["railAttention"]), card)
        for name, want in [("rail selected row", sel), ("rail blocked tint", tint)]:
            hit = any(near(c, want, 3) for c in colours)
            print(f"  {name:<18} #{bytes(want).hex()} {'found' if hit else 'MISSING'}")
            if not hit:
                failures.append(f"{theme} {name}: no #{bytes(want).hex()} in the rail")
        samples["rail card"] = (card, None)
    else:
        failures.append(f"{theme}: no rail capture")
    for name, (got, want) in samples.items():
        note = ""
        if want is not None and not near(got, want, 3):
            failures.append(f"{theme} {name}: #{bytes(got).hex()} is not the theme's #{bytes(want).hex()}")
            note = f"  ✗ want #{bytes(want).hex()}"
        print(f"  {name:<10} #{bytes(got).hex()}{note}")
    seen[theme] = {k: v[0] for k, v in samples.items()}
    if docs:
        small = img.resize((1200, 750), Image.LANCZOS)
        small.save(root / f"docs/design/theme-{theme}.png", optimize=True)

# No view kept the previous theme's colour.
order = [t for _, t in themes]
for prev, cur in zip(order, order[1:]):
    if prev in seen and cur in seen:
        for name, value in seen[cur].items():
            if value == seen[prev].get(name):
                failures.append(f"{cur} {name}: still {prev}'s #{bytes(value).hex()}")

# Back to atom-one-dark: the same pixels as the first time (the clock and the
# spinners aside), so nothing drifted on the way round.
first, again = out / "t1-atom-one-dark-app.png", out / "t5-atom-one-dark-again-app.png"
if first.exists() and again.exists():
    a, b = Image.open(first).convert("RGB"), Image.open(again).convert("RGB")
    for name, box in [("sidebar", SIDEBAR), ("tab strip", TABSTRIP), ("header", HEADER),
                      ("terminal", TERMINAL), ("preview", PREVIEW)]:
        if mode(a, box) != mode(b, box):
            failures.append(f"atom-one-dark again {name}: #{bytes(mode(b, box)).hex()} ≠ #{bytes(mode(a, box)).hex()}")

# The user theme: its window colour, then the edit's, live.
for shot, want in [("t6-dusk-app.png", (0x3a, 0x1f, 0x4d)), ("t7-dusk-edited-app.png", (0x1f, 0x3a, 0x4d))]:
    p = out / shot
    if p.exists():
        got = mode(Image.open(p).convert("RGB"), GUTTER)
        print(f"{shot}: window #{bytes(got).hex()}")
        if not near(got, want, 2):
            failures.append(f"{shot}: window #{bytes(got).hex()}, want #{bytes(want).hex()}")
banner = re.findall(r"remember banner\.full = (.*)", log)
print("banner with dusk:", banner[-1] if banner else "-")
if not banner or "sparkle" not in banner[-1] or "tab_strip" not in banner[-1]:
    failures.append("the user theme's unknown key and bad colour didn't reach the banner")

for f in failures:
    print("FAIL:", f)
print("themes: pixels", "ok" if not failures else f"{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
PY
exit "$status"
