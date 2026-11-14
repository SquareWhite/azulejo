#!/usr/bin/env bash
# PLAN 1.13: keystroke → glyph in the real app's terminal tile, with the 0.4
# method (LatencyBench.swift: in-process ghostty_surface_key, window-only
# ScreenCaptureKit capture, no OS input, window never raised). Compare with
# the 0.4 spike numbers in docs/measurements.md (config c: same surface via
# herdr attach; config b: direct, the baseline).
#
#   scripts/nfr/latency.sh [samples] [rounds]     default 250 × 3, debug build
#   NFR_APP=target/release-mac/Azulejo.app scripts/nfr/latency.sh
#
# Each round also runs the 0.4 spike harness (config c: the same libghostty
# surface code, attached to the same herdr pane) right after the app, so the
# two are compared under the same WindowServer conditions. Absolute numbers
# from a background window depend heavily on what covers it.
#
# The app binary is exec'd from this shell (not through `open`), so screen
# capture permission is the terminal's, as in 0.4.
set -euo pipefail
source "$(dirname "$0")/lib.sh"
samples="${1:-250}"
rounds="${2:-3}"
build="$(basename "$(dirname "$app")")"
results="$out/latency-$build"
mkdir -p "$results"
cargo build -q -p azulejo-cli
reset_all
herdr_fresh

say "one terminal tile running /bin/cat"
launch
wait_ping
wait_tiles 1
tile="$(azj --json tile list | jq -r '.tiles[0].id')"
term_id="$(azj --json tile get --tile "$tile" | jq -r '.. | .terminal_id? // empty' | head -1)"
for _ in $(seq 100); do grep -q "phase attached" "$log" && break; sleep 0.1; done
azj terminal send-text --tile "$tile" --text $'clear; exec /bin/cat\n' >/dev/null
sleep 1
stop
sleep 0.5

# Same Ghostty settings as the 0.4 configs: no cursor blink (it would show up
# in the glyph box once the surface has focus). XDG_CONFIG_HOME only feeds
# Ghostty here; herdr is linked through so the attach client finds its socket.
xdg="$out/xdg"
mkdir -p "$xdg/ghostty"
ln -sfn "$HOME/.config/herdr" "$xdg/herdr"
printf 'cursor-style-blink = false\nshell-integration = none\n' >"$xdg/ghostty/config"

for r in $(seq "$rounds"); do
  say "round $r ($build)"
  csv="$results/app-r$r.csv"
  rm -f "$csv"
  launch_direct AZULEJO_LATENCY_BENCH="$samples" AZULEJO_LATENCY_LABEL="app-r$r" \
    AZULEJO_LATENCY_CSV="$csv" AZULEJO_LATENCY_DELAY=3 XDG_CONFIG_HOME="$xdg"
  pid="$(running_pid)"
  while kill -0 "$pid" 2>/dev/null; do sleep 1; done
  grep -E "latency \[" "$log" || true
  [[ -s "$csv" ]] || fail "round $r wrote no CSV (see $log)"
  sleep 0.5
  # The spike surface on the same pane, same Ghostty settings.
  harness="$root/scripts/latency/build/LatHarness.app/Contents/MacOS/LatHarness"
  [[ -x $harness ]] || "$root/scripts/latency/harness/build.sh" >/dev/null
  "${clean_env[@]}" XDG_CONFIG_HOME="$xdg" "$harness" \
    --command "$(command -v herdr) --session $session terminal attach $term_id --takeover" \
    --env "XDG_CONFIG_HOME=$xdg" \
    --bench "$samples" --label "spike-r$r" --csv "$results/spike-r$r.csv" 2>>"$out/harness.log" | tail -1
  sleep 0.5
done

python3 - "$results" <<'EOF'
import csv, glob, statistics, sys
for kind in ("app", "spike"):
    for path in sorted(glob.glob(f"{sys.argv[1]}/{kind}-r*.csv")) + [None]:
        files = [path] if path else sorted(glob.glob(f"{sys.argv[1]}/{kind}-r*.csv"))
        v, miss = [], 0
        for f in files:
            for row in csv.DictReader(open(f)):
                if row["latency_ms"]: v.append(float(row["latency_ms"]))
                else: miss += 1
        if not v: continue
        v.sort()
        p = lambda q: v[min(len(v) - 1, round(q * (len(v) - 1)))]
        name = path.rsplit("/", 1)[1][:-4] if path else f"{kind} pooled"
        print(f"{name:14} n={len(v):4} misses={miss:3} min {v[0]:6.1f} median {statistics.median(v):6.1f} p95 {p(.95):6.1f} p99 {p(.99):6.1f} max {v[-1]:6.1f} ms")
EOF
