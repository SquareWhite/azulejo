#!/usr/bin/env bash
# Preview tile (1.9) check, in-process, on its own dev instance
# (AZJ_DEV_NAME=preview by default): markdown, live reload, code, image, PDF,
# missing file. Fixtures live in a temp folder that is removed afterwards.
#   apps/macos/automation/preview.sh [--keep]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-preview}"
out="$root/.dev/$AZJ_DEV_NAME/out/preview"
fixtures="$(mktemp -d "${TMPDIR:-/tmp}/azj-preview.XXXXXX")"
writer=""
cleanup() {
  [[ -n "$writer" ]] && kill "$writer" 2>/dev/null || true
  rm -rf "$fixtures"
  # --keep leaves the dev instance (and $out: log, screenshots) for a look.
  [[ "${1:-}" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null
}
trap 'cleanup "${1:-}"' EXIT

printf '# One\n\nA plan.\n' >"$fixtures/plan.md"
printf 'fn main() {\n    println!("hi");\n}\n' >"$fixtures/main.rs"
sips -s format png /System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericDocumentIcon.icns \
  --out "$fixtures/shot.png" >/dev/null
printf 'Azulejo preview check\n' | cupsfilter -i text/plain -m application/pdf - >"$fixtures/doc.pdf" 2>/dev/null

"$root/apps/macos/dev.sh" --reset >/dev/null   # also clears $out
mkdir -p "$out"
sed "s|@DIR@|$fixtures|g" "$here/preview.azs.in" >"$fixtures/preview.azs"

# Edit plan.md once the first screenshot exists; capture the window (our own,
# by id, never raised) for the QuickLook kinds, which in-app shots can't see.
( wait_for() { for _ in $(seq 1 600); do [[ -f "$1" ]] && return 0; sleep 0.1; done; return 1; }
  wait_for "$out/01-markdown.png" && printf '# Two\n\nThe plan changed.\n' >"$fixtures/plan.md"
  for name in 03-image 04-pdf; do
    wait_for "$out/$name.png" || exit 0
    sleep 1.5
    screencapture -x -o -l "$(cat "$out/window-id")" "$out/$name-window.png" || true
  done ) &
writer=$!

"$root/apps/macos/dev.sh" --automation "$fixtures/preview.azs" --out "$out"
[[ "$(cat "$out/result")" == ok ]]
