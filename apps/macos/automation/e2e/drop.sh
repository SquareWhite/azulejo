#!/usr/bin/env bash
# Drag and drop onto terminal tiles (TerminalDrop), in-process: each drop is a
# private pasteboard handed to the tile's real drag-destination methods (never
# the general pasteboard, no OS input), and pastecap.py in the herdr pane
# records the bytes that reach the PTY. Files (one, a path that needs
# escaping, several), image data (PNG, TIFF, an image off a web page) written
# to a temp PNG, text and URLs, each with bracketed paste off and on; the
# highlight while a drag hovers; a drop on an unfocused tile focuses it; a
# drag of something else is refused.
# Dev instance e2e-drop (herdr session azj-dev-e2e-drop), reset before and after.
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-drop}" E2E_SUITE=drop
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config </dev/null

fx="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/azj-drop.XXXXXX")" && pwd -P)"
# The same name is written out in drop.azs.in (sed would read its `&`).
tricky="it's a \"quoted\" (file) & ünï 文件 \$x [1].txt"
echo plain >"$fx/plain.txt"
echo tricky >"$fx/$tricky"
echo other >"$fx/other file.md"
sips -s format png /System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericDocumentIcon.icns \
  --resampleWidth 96 --out "$fx/shot.png" >/dev/null
sips -s format tiff "$fx/shot.png" --out "$fx/shot.tiff" >/dev/null

# What each drop should type, before any bracketed-paste framing: Ghostty.app's
# escaping (a backslash before \ ()[]{}<>"'`!#$&;|*? and tab), written again
# here so the test doesn't share the app's code.
mkdir -p "$fx/expect"
python3 - "$fx" "$tricky" <<'PY'
import sys
fx, tricky = sys.argv[1], sys.argv[2]
def esc(p): return "".join("\\" + c if c in "\\ ()[]{}<>\"'`!#$&;|*?\t" else c for c in p)
cases = {
    "single": esc(f"{fx}/plain.txt"),
    "tricky": esc(f"{fx}/{tricky}"),
    "several": " ".join(esc(f"{fx}/{n}") for n in ["plain.txt", tricky, "other file.md"]),
    "text": "hello from a drop",
    "url": "https://example.com/a?b=1&c=2",
    "lines": "line one\nline two",
    "focus": "dropped on tile two",
}
for name, text in cases.items():
    open(f"{fx}/expect/{name}", "w", encoding="utf-8").write(text)
PY

drops="$(getconf DARWIN_USER_TEMP_DIR)azulejo-drops"
o="$out/drop"
sed -e "s|@FX@|$fx|g" -e "s|@HERE@|$e2e_here|g" -e "s|@DROPS@|$drops|g" \
  "$e2e_here/drop.azs.in" >"$out/drop.azs"
run_azs "$out/drop.azs" drop
grep -E '^ *[0-9]+ # ===|FAIL|line [0-9]+:' "$o/automation.log" | tee "$out/scenarios.txt" || true
check "drop: every scenario passed" grep -qx ok "$o/result"
grep -c PASS "$checks" | sed 's/^/passed: /'; ! grep -q FAIL "$checks"
