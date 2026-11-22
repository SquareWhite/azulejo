#!/usr/bin/env bash
# Rail captures: the collapsed sidebar (five workspaces, agents working and
# blocked, the second selected) in all four built-in themes, at rest and
# with ⌘ held (<theme>-rail-cmd), the third row's hover card
# (<theme>-card: a fixture repo with a deep path, a long branch and
# uncommitted changes) and the machine letter's (<theme>-group-card)
# (rail-shots.azs.in), on its own dev instance
# (AZJ_DEV_NAME=railcap). Window captures of our own window only; the left
# 64 pt of each, with 2x zooms of the rows, and the card's corner land in
# .dev/railcap.out/rail-<label>/ for before/after comparisons. Shells are
# bash with a bare environment and a bare "$ " prompt, and the fixtures
# live in /tmp/azj-<name>, so no capture shows anything of this Mac.
#   apps/macos/automation/rail.sh LABEL [--keep]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
label="${1:?usage: rail.sh LABEL [--keep]}"
KEEP="${2:-}"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-railcap}"
out="$root/.dev/$AZJ_DEV_NAME.out/rail"
dest="$out-$label"
work="/tmp/azj-$AZJ_DEV_NAME"
rm -rf "$work"
mkdir -p "$work"
work="$(cd "$work" && pwd -P)"
cleanup() {
  kill "${watcher:-}" "${watchdog:-}" 2>/dev/null || true
  rm -rf "$out/.capture"
  [[ "$KEEP" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

# Never the user's herdr: the dev instance has its own session.
unset HERDR_SOCKET_PATH HERDR_PANE_ID HERDR_WORKSPACE_ID HERDR_TAB_ID HERDR_ENV
rm -f "$root/.dev/$AZJ_DEV_NAME/config/azulejo/config.toml"
"$root/apps/macos/dev.sh" --reset >/dev/null
rm -rf "$out"
mkdir -p "$out/.capture"

# Neutral shells (as showcase.sh's): bash, a bare environment, a fixture
# HOME and a bare "$ " prompt (a pane starts in this Mac's home folder).
mkdir -p "$work/home"
cat >"$work/shell" <<SH
#!/bin/sh
exec env -i HOME='$work/home' PATH=/usr/bin:/bin:/usr/sbin:/sbin TERM="\${TERM:-xterm-256color}" \\
  COLORTERM=truecolor LANG=en_GB.UTF-8 HISTFILE=/dev/null BASH_SILENCE_DEPRECATION_WARNING=1 \\
  PS1='\$ ' /bin/bash --noprofile --norc "\$@"
SH
chmod +x "$work/shell"
printf '[terminal]\ndefault_shell = "%s"\nshell_mode = "non_login"\n' "$work/shell" >"$work/herdr.toml"
export AZJ_HERDR_CONFIG="$work/herdr.toml" HERDR_CONFIG_PATH="$work/herdr.toml"

# The hover card's fixture: deep, on a long branch, uncommitted changes.
card="$work/clients/example-studio/quarterly-reporting-dashboard"
mkdir -p "$card"
printf '# dashboard\n' >"$card/README.md"
git -C "$card" -c init.defaultBranch=chore/release-candidate-handover-final init -q
git -C "$card" add -A
git -C "$card" -c user.email=dev@example.invalid -c user.name=dev -c commit.gpgsign=false commit -qm init
printf 'draft\n' >>"$card/README.md"
sed -e "s|@CARD@|$card|g" "$here/rail-shots.azs.in" >"$work/rail-shots.azs"

# The app asks for window captures by dropping files in $out/.capture; take
# them of its own window (by id) from here.
watch_captures() {
  while [[ ! -f "$out/result" ]]; do
    for req in "$out/.capture"/*; do
      [[ -e "$req" ]] || continue
      name="$(basename "$req")"
      rm -f "$req"
      wid="$(cat "$out/window-id" 2>/dev/null || true)"
      [[ -n "$wid" ]] && /usr/sbin/screencapture -l "$wid" -x -o "$out/$name.png" 2>/dev/null || true
    done
    sleep 0.02
  done
}
watch_captures &
watcher=$!
( sleep 180; "$root/apps/macos/dev.sh" --stop ) &
watchdog=$!

"$root/apps/macos/dev.sh" --automation "$work/rail-shots.azs" --out "$out"
[[ "$(cat "$out/result")" == ok ]] || { echo "rail: $(cat "$out/result")"; exit 1; }

mkdir -p "$dest"
python3 - "$out" "$dest" <<'PY'
import sys
from pathlib import Path
from PIL import Image

out, dest = Path(sys.argv[1]), Path(sys.argv[2])
# The system capture when the watcher got one, else the app's own render.
names = ["%s-%s" % (t, v) for t in ("atom-one-dark", "atom-one-light", "macos-dark", "macos-light")
         for v in ("rail", "rail-cmd", "card", "group-card")]
shots = []
for name in names:
    for p in (out / f"{name}.png", out / f"{name}-app.png"):
        if p.exists() and p.stat().st_size:
            shots.append((name, p))
            break
for name, p in shots:
    img = Image.open(p)
    scale = round(img.width / 1000) or 1
    if name.endswith("-card"):
        # The rail and the card right of its third row, with its shadow.
        img.crop((0, 40 * scale, 460 * scale, 360 * scale)).save(dest / f"{name}.png", optimize=True)
        continue
    rail = img.crop((0, 0, 64 * scale, img.height))
    rail.save(dest / f"{name}.png", optimize=True)
    # The rows, zoomed: the group header and the five workspaces.
    rows = img.crop((0, 52 * scale, 64 * scale, 240 * scale))
    rows.resize((rows.width * 2, rows.height * 2), Image.NEAREST).save(
        dest / f"{name}.png".replace("-rail", "-rows"), optimize=True)
print(f"rail: {len(shots)} captures → {dest}")
PY
echo "rail: ok ($out)"
