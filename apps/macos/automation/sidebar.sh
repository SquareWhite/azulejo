#!/usr/bin/env bash
# Sidebar (D.3) check, in-process, on its own dev instance
# (AZJ_DEV_NAME=side by default): machine group, workspace rows with status
# icons and branches, footer counts, row clicks, and the Files view (tree,
# keys, open beside the focused tile, Esc, following the focused workspace).
# Fixture repositories live in a temp folder that is removed afterwards.
#   apps/macos/automation/sidebar.sh [--keep]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-side}"
out="$root/.dev/$AZJ_DEV_NAME.out/sidebar"
fixtures="$(mktemp -d "${TMPDIR:-/tmp}/azj-side.XXXXXX")"
fixtures="$(cd "$fixtures" && pwd -P)"
cleanup() {
  rm -rf "$fixtures"
  [[ "${1:-}" == "--keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null
}
trap 'cleanup "${1:-}"' EXIT

repo() {
  local dir="$fixtures/$1" branch="$2"
  mkdir -p "$dir"
  printf '# %s\n' "$1" >"$dir/README.md"
  if [[ "$1" == azulejo ]]; then mkdir -p "$dir/src"; printf 'fn main() {}\n' >"$dir/src/main.rs"; fi
  git -C "$dir" -c init.defaultBranch="$branch" init -q
  git -C "$dir" add -A
  git -C "$dir" -c user.email=t@t -c user.name=t commit -qm init
}
repo azulejo main
repo sample-service development
repo sample-project audit/critical-path-review
repo sample-dashboard dev
repo config main

"$root/apps/macos/dev.sh" --reset >/dev/null
rm -rf "$out"
mkdir -p "$out"
sed -e "s|@DIR@|$fixtures|g" "$here/sidebar.azs.in" >"$fixtures/sidebar.azs"
( sleep 180; "$root/apps/macos/dev.sh" --stop ) &
watchdog=$!
status=0
"$root/apps/macos/dev.sh" --automation "$fixtures/sidebar.azs" --out "$out" || status=1
kill "$watchdog" 2>/dev/null || true
[[ "$(cat "$out/result" 2>/dev/null)" == ok ]] || status=1
echo "sidebar: $(cat "$out/result" 2>/dev/null || echo 'no result')"
exit "$status"
