#!/usr/bin/env bash
# PLAN 1.13: memory, one window with 4 attached terminal tiles (idle shells),
# browser tiles excluded (target < 150 MB). Reports the app's phys_footprint
# (what Activity Monitor calls Memory), plus separately: the attach children,
# azj-observe helpers, the herdr server, and with a browser + preview tile
# added, the app and the new WebKit processes.
#
#   scripts/nfr/memory.sh                      debug build
#   NFR_APP=target/release-mac/Azulejo.app scripts/nfr/memory.sh
set -euo pipefail
source "$(dirname "$0")/lib.sh"
build="$(basename "$(dirname "$app")")"
cargo build -q -p azulejo-cli
reset_all

fp() { # pid... → total footprint in MB
  local total=0 p kb
  for p in "$@"; do
    kb="$(footprint "$p" 2>/dev/null | python3 -c '
import re, sys
m = re.search(r"Footprint: ([0-9.]+) (B|KB|MB|GB)", sys.stdin.read())
f = {"B": 1/1024, "KB": 1, "MB": 1024, "GB": 1024*1024}
print(float(m.group(1)) * f[m.group(2)] if m else 0)')"
    total="$(python3 -c "print($total + $kb)")"
  done
  python3 -c "print('%.1f' % ($total / 1024))"
}
fp_settled() { # pid → median of 5 samples, 2 s apart
  local i v=()
  for i in 1 2 3 4 5; do v+=("$(fp "$1")"); sleep 2; done
  python3 -c "import statistics,sys; v=[float(x) for x in sys.argv[1:]]; print('%.1f (min %.1f, max %.1f)' % (statistics.median(v), min(v), max(v)))" "${v[@]}"
}
descendants() { local p c; for p in "$@"; do for c in $(pgrep -P "$p" || true); do echo "$c"; descendants "$c"; done; done; }
webkit() { pgrep -f 'com.apple.WebKit.(WebContent|Networking|GPU)' | sort; }

herdr_fresh
webkit >"$out/webkit-before.txt"
launch
wait_ping
wait_tiles 1
for _ in 1 2 3; do azj layout split-auto >/dev/null; done
sleep 4
pid="$(running_pid)"
kids=($(descendants "$pid"))
attach=() observe=()
for k in "${kids[@]}"; do
  cmd="$(ps -o command= -p "$k" 2>/dev/null || true)"
  [[ $cmd == *"terminal attach"* ]] && attach+=("$k")
  [[ $cmd == *azj-observe* ]] && observe+=("$k")
done
herdr_pid="$(pgrep -f "herdr --session $session server" | head -1)"
say "4 terminal tiles ($build)"
echo "app (pid $pid)                 $(fp_settled "$pid") MB"
echo "attach chain (${#kids[@]} procs, ${#attach[@]} attach)  $(fp "${kids[@]}") MB"
echo "azj-observe helpers (${#observe[@]})      $( ((${#observe[@]})) && fp "${observe[@]}" || echo 0) MB"
echo "herdr server (pid $herdr_pid)  $(fp "$herdr_pid") MB"

# A browser on a local page and a markdown preview.
site="$out/site"; mkdir -p "$site"
printf '<!doctype html><title>nfr</title><h1>NFR page</h1><p>%s</p>\n' "$(printf 'lorem ipsum %.0s' $(seq 200))" >"$site/index.html"
(cd "$site" && exec python3 -m http.server 8767 --bind 127.0.0.1 >/dev/null 2>&1) &
srv=$!
trap 'kill $srv 2>/dev/null || true' EXIT
printf '# Plan\n\n- [x] memory\n\n```rust\nfn main() {}\n```\n' >"$dev/plan.md"
sleep 0.5
azj layout split-auto --kind browser --url "http://127.0.0.1:8767/" >/dev/null
azj layout split-auto --kind preview --path "$dev/plan.md" >/dev/null
sleep 5
new_webkit=($(comm -13 "$out/webkit-before.txt" <(webkit)))
say "+ browser + preview ($build)"
echo "app (pid $pid)                 $(fp_settled "$pid") MB"
echo "new WebKit processes (${#new_webkit[@]})     $( ((${#new_webkit[@]})) && fp "${new_webkit[@]}" || echo 0) MB"
stop
