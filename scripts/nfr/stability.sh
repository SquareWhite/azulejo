#!/usr/bin/env bash
# PLAN 1.13: stability and security NFRs on the dev instance `nfr`.
#
# Stability: with two terminals running (one a ticker), kill the WebContent
# process behind a browser tile and the QuickLook process behind a preview
# tile. The app, the attach processes and the ticker must carry on.
# Security: API socket 0600, its directory 0700, owned by the user.
#
#   scripts/nfr/stability.sh
set -euo pipefail
source "$(dirname "$0")/lib.sh"
cargo build -q -p azulejo-cli
reset_all
herdr_fresh

new_since() { comm -13 "$1" <(pgrep -f "$2" | sort); }
descendants() { local p c; for p in "$@"; do for c in $(pgrep -P "$p" || true); do echo "$c"; descendants "$c"; done; done; }
tick() { azj --json terminal read --tile "$1" --source recent --lines 5 | jq -r '.text' | { grep -oE 'tick [0-9]+' || true; } | tail -1 | cut -d' ' -f2; }

say "two terminals, one a ticker"
launch
wait_ping
wait_tiles 1
azj layout split-auto >/dev/null
for _ in $(seq 100); do [[ $(grep -c "phase attached" "$log") -ge 2 ]] && break; sleep 0.1; done
t1="$(azj --json tile list | jq -r '[.tiles[] | select(.kind=="terminal")][0].id')"
azj terminal send-text --tile "$t1" --text $'sh -c \'i=0; while :; do i=$((i+1)); echo tick $i; sleep 0.2; done\'\n' >/dev/null
pid="$(running_pid)"

say "security: socket and directory permissions"
sock_mode="$(stat -f '%Lp %Su' "$sock")"
dir_mode="$(stat -f '%Lp %Su' "$(dirname "$sock")")"
echo "   socket $sock_mode, dir $dir_mode"
[[ ${sock_mode%% *} == 600 ]] || fail "socket mode $sock_mode"
[[ ${dir_mode%% *} == 700 ]] || fail "socket dir mode $dir_mode"

say "a browser and a QuickLook preview"
site="$out/site"; mkdir -p "$site"
printf '<!doctype html><title>nfr</title><h1>stability</h1>\n' >"$site/index.html"
(cd "$site" && exec python3 -m http.server 8768 --bind 127.0.0.1 >/dev/null 2>&1) &
srv=$!
trap 'kill $srv 2>/dev/null || true' EXIT
# A one-page PDF, written by hand, for QLPreviewView.
python3 - "$dev/nfr.pdf" <<'EOF'
import sys
objs = [b"<< /Type /Catalog /Pages 2 0 R >>", b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
        None, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
stream = b"BT /F1 24 Tf 40 100 Td (Azulejo NFR) Tj ET"
objs[3] = b"<< /Length %d >>\nstream\n" % len(stream) + stream + b"\nendstream"
out, offs = b"%PDF-1.4\n", []
for i, o in enumerate(objs, 1):
    offs.append(len(out)); out += b"%d 0 obj\n" % i + o + b"\nendobj\n"
x = len(out)
out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1) + b"".join(b"%010d 00000 n \n" % o for o in offs)
out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, x)
open(sys.argv[1], "wb").write(out)
EOF
pgrep -f 'com.apple.WebKit.WebContent' | sort >"$out/wc-before.txt"
pgrep -f -i 'quicklook' | sort >"$out/ql-before.txt"
sleep 0.3
azj layout split-auto --kind browser --url "http://127.0.0.1:8768/" >/dev/null
azj layout split-auto --kind preview --path "$dev/nfr.pdf" >/dev/null
sleep 4
webcontent=($(new_since "$out/wc-before.txt" 'com.apple.WebKit.WebContent'))
quicklook=($(comm -13 "$out/ql-before.txt" <(pgrep -f -i 'quicklook' | sort)))
echo "   new WebContent: ${webcontent[*]:-none}"
for q in "${quicklook[@]}"; do echo "   new QuickLook-related: $q $(ps -o comm= -p "$q" 2>/dev/null)"; done

attach_before="$(descendants "$pid" | sort | tr '\n' ' ')"
tick_before="$(tick "$t1")"
echo "   attach processes: $attach_before"
echo "   ticker at $tick_before"

say "kill the browser tile's WebContent process"
((${#webcontent[@]})) || fail "no WebContent process appeared for the browser tile"
kill -KILL "${webcontent[@]}"
sleep 2
grep -E "web content|WebContent|crash|terminated" "$log" | tail -3 || true

say "kill the preview tile's QuickLook processes"
if ((${#quicklook[@]})); then
  # Only processes that appeared for our preview tile, never pre-existing ones.
  kill -KILL "${quicklook[@]}" 2>/dev/null || true
else
  echo "   none appeared: QLPreviewView rendered this PDF in-process"
fi
sleep 2

say "terminals unaffected?"
kill -0 "$pid" 2>/dev/null || fail "the app died"
attach_after="$(descendants "$pid" | sort | tr '\n' ' ')"
[[ $attach_before == "$attach_after" ]] || fail "attach processes changed: $attach_before → $attach_after"
tick_after="$(tick "$t1")"
[[ -n $tick_after && $tick_after -gt $tick_before ]] || fail "ticker stopped ($tick_before → ${tick_after:-none})"
echo "   app alive, same attach processes, ticker $tick_before → $tick_after"
azj --json tile list | jq -r '.tiles[] | "   \(.kind) \(.id[-4:])"'
stop
echo
echo "PASS"
