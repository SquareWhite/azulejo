#!/usr/bin/env bash
# File browser tile (2.5) check, in-process, on its own dev instance
# (AZJ_DEV_NAME=files by default): opens a synthetic 50k-file git repo,
# expands folders by key, follows live changes, hidden/ignored, filter,
# reveal, open-in-preview and git decorations. Prints the measured numbers.
# Fixtures live in a temp folder that is removed afterwards.
#   apps/macos/automation/files.sh [--keep]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-files}"
dev="$root/.dev/$AZJ_DEV_NAME"
out="$dev/out/files"
fixtures="$(mktemp -d "${TMPDIR:-/tmp}/azj-files.XXXXXX")"
fixtures="$(cd "$fixtures" && pwd -P)"
writer=""
cleanup() {
  [[ -n "$writer" ]] && kill "$writer" 2>/dev/null || true
  rm -rf "$fixtures"
  [[ "${1:-}" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null
}
trap 'cleanup "${1:-}"' EXIT

git_quiet() { git -c user.email=t@t -c user.name=t -c init.defaultBranch=main "$@" >/dev/null; }

# 50,000 files: d00..d09 / s00..s49 / f000..f099.txt, plus ignored node_modules.
big="$fixtures/big"
python3 - "$big" <<'PY'
import os, sys
root = sys.argv[1]
for d in range(10):
    for s in range(50):
        p = os.path.join(root, f"d{d:02}", f"s{s:02}")
        os.makedirs(p)
        for f in range(100):
            open(os.path.join(p, f"f{f:03}.txt"), "w").close()
os.makedirs(os.path.join(root, "node_modules", "x"))
open(os.path.join(root, "README.md"), "w").write("# big\n")
open(os.path.join(root, ".gitignore"), "w").write("node_modules/\n")
open(os.path.join(root, ".env"), "w").close()
PY
start=$(python3 -c 'import time; print(time.time())')
git_quiet -C "$big" init && git_quiet -C "$big" add -A && git_quiet -C "$big" commit -qm init
echo "fixture: 50k-file repo committed in $(python3 -c "import time; print(round(time.time()-$start,1))") s"

small="$fixtures/small"
mkdir -p "$small/src"
printf 'a\n' >"$small/src/a.rs"; printf 'b\n' >"$small/b.md"
git_quiet -C "$small" init && git_quiet -C "$small" add -A && git_quiet -C "$small" commit -qm init
printf 'changed\n' >"$small/src/a.rs"; printf 'new\n' >"$small/new.txt"

"$root/apps/macos/dev.sh" --reset >/dev/null
mkdir -p "$out"
sed -e "s|@BIG@|$big|g" -e "s|@DIR@|$small|g" "$here/files.azs.in" >"$fixtures/files.azs"

now_ms() { python3 -c 'import time; print(int(time.time()*1000))'; }
( wait_for() { for _ in $(seq 1 600); do [[ -f "$1" ]] && return 0; sleep 0.1; done; return 1; }
  wait_for "$out/03-watching.png" || exit 0
  sleep 0.5
  echo "write create $(now_ms)" >>"$out/writes"; : >"$big/d03/live.txt"; sleep 0.8
  echo "write rename $(now_ms)" >>"$out/writes"; mv "$big/d03/live.txt" "$big/d03/live2.txt"; sleep 0.8
  echo "write delete $(now_ms)" >>"$out/writes"; rm "$big/d03/live2.txt" ) &
writer=$!

"$root/apps/macos/dev.sh" --automation "$fixtures/files.azs" --out "$out"
result="$(cat "$out/result")"

log="$dev/logs/app.log"
echo "--- measurements"
grep -h "first rows" "$log" "$out/automation.log" 2>/dev/null | sed 's/^.*files: /files: /' | sort -u || true
if [[ -f "$out/writes" ]]; then
  python3 - "$out/writes" "$log" <<'PY'
import re, sys
writes = [l.split() for l in open(sys.argv[1])]
applied = [int(m.group(1)) for m in re.finditer(r"change applied at (\d+)", open(sys.argv[2], errors="replace").read())]
for (_, op, t), a in zip(writes, applied):
    print(f"live {op}: {a - int(t)} ms from write to rows updated")
PY
fi
[[ "$result" == ok ]]
