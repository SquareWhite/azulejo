#!/usr/bin/env bash
# Slow folders never freeze the app: every folder listing is held back
# (AZJ_FILES_LIST_DELAY_MS, standing in for a network volume or a permission
# prompt) while a files tile and the sidebar's Files view open a 20k-entry
# folder holding a git repo. The script keeps running meanwhile (each step
# needs the main thread), the tile shows "Loading…", the rows arrive, and a
# listing for a root that changed since is dropped. In-process, on its own
# dev instance (AZJ_DEV_NAME=fslow by default); fixtures live in a temp
# folder that is removed afterwards.
#   apps/macos/automation/files-slow.sh [--keep]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-fslow}"
export AZJ_FILES_LIST_DELAY_MS="${AZJ_FILES_LIST_DELAY_MS:-2500}"
dev="$root/.dev/$AZJ_DEV_NAME"
out="$dev/out/files-slow"
fixtures="$(mktemp -d "${TMPDIR:-/tmp}/azj-fslow.XXXXXX")"
fixtures="$(cd "$fixtures" && pwd -P)"
cleanup() {
  rm -rf "$fixtures"
  [[ "${1:-}" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null
}
trap 'cleanup "${1:-}"' EXIT

git_quiet() { git -c user.email=t@t -c user.name=t -c init.defaultBranch=main "$@" >/dev/null; }

# big: 20,000 files, sub/ (100 files) and repo/ (a small git repo).
big="$fixtures/big"
python3 - "$big" <<'PY'
import os, sys
root = sys.argv[1]
os.makedirs(os.path.join(root, "sub"))
os.makedirs(os.path.join(root, "repo", "src"))
for i in range(20000):
    open(os.path.join(root, f"f{i:05}.txt"), "w").close()
for i in range(100):
    open(os.path.join(root, "sub", f"s{i:03}.txt"), "w").close()
open(os.path.join(root, "repo", "README.md"), "w").write("# repo\n")
open(os.path.join(root, "repo", "src", "main.rs"), "w").write("fn main() {}\n")
open(os.path.join(root, "repo", ".gitignore"), "w").write("*.log\n")
open(os.path.join(root, "repo", "debug.log"), "w").close()
PY
git_quiet -C "$big/repo" init && git_quiet -C "$big/repo" add -A && git_quiet -C "$big/repo" commit -qm init

small="$fixtures/small"
mkdir -p "$small"
: >"$small/a.txt"; : >"$small/b.txt"

"$root/apps/macos/dev.sh" --reset >/dev/null
mkdir -p "$out"
sed -e "s|@BIG@|$big|g" -e "s|@SMALL@|$small|g" "$here/files-slow.azs.in" >"$fixtures/files-slow.azs"

"$root/apps/macos/dev.sh" --automation "$fixtures/files-slow.azs" --out "$out"
result="$(cat "$out/result")"
echo "files-slow: $result (listings held back ${AZJ_FILES_LIST_DELAY_MS} ms)"
grep -h "first rows" "$dev/logs/app.log" 2>/dev/null | sed 's/^.*files: /files: /' | sort -u || true
[[ "$result" == ok ]]
