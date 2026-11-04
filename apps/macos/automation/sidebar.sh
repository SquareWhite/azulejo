#!/usr/bin/env bash
# Sidebar (D.3) check, in-process, on its own dev instance
# (AZJ_DEV_NAME=side by default): machine group, workspace rows with status
# icons and branches, footer counts, row clicks, and the Files view (tree,
# keys, open beside the focused tile, Esc, following the focused workspace).
# Then the rail, the collapsed sidebar: statuses, blocked agents, clicks,
# tooltips, the right-click menu, the keyboard; after a relaunch
# (sidebar-relaunch.azs) it's still collapsed, a stand-in remote machine
# reconnects, and `[sidebar] collapsed = "hidden"` hides it instead.
# Fixture repositories live in a temp folder that is removed afterwards.
#   apps/macos/automation/sidebar.sh [--keep]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-side}"
out="$root/.dev/$AZJ_DEV_NAME.out/sidebar"
dev="$root/.dev/$AZJ_DEV_NAME"
side="$root/.dev/$AZJ_DEV_NAME.machines"
remote="azj-dev-$AZJ_DEV_NAME-remote"
fixtures="$(mktemp -d "${TMPDIR:-/tmp}/azj-side.XXXXXX")"
fixtures="$(cd "$fixtures" && pwd -P)"

# Never the user's herdr: every herdr call names a throwaway session.
unset HERDR_SOCKET_PATH HERDR_PANE_ID HERDR_WORKSPACE_ID HERDR_TAB_ID HERDR_ENV
herdr="$(command -v herdr || echo "$HOME/.local/bin/herdr")"

cleanup() {
  rm -rf "$fixtures"
  [[ "${1:-}" == "--keep" ]] && return
  "$root/apps/macos/dev.sh" --reset >/dev/null
  "$herdr" --session "$remote" server stop >/dev/null 2>&1 || true
  sleep 0.5
  "$herdr" session delete "$remote" >/dev/null 2>&1 || true
  rm -rf "$side"
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
rm -rf "$out" "$side"
mkdir -p "$out" "$side"
sed -e "s|@DIR@|$fixtures|g" "$here/sidebar.azs.in" >"$fixtures/sidebar.azs"
sed -e "s|@SIDE@|$side|g" -e "s/azj-dev-side-remote/$remote/g" \
  "$here/sidebar-relaunch.azs.in" >"$fixtures/sidebar-relaunch.azs"
( sleep 360; "$root/apps/macos/dev.sh" --stop ) &
watchdog=$!
status=0
"$root/apps/macos/dev.sh" --automation "$fixtures/sidebar.azs" --out "$out" || status=1
[[ "$(cat "$out/result" 2>/dev/null)" == ok ]] || status=1
echo "sidebar: $(cat "$out/result" 2>/dev/null || echo 'no result')"

# The relaunch, with a stand-in remote machine: a second, throwaway local
# herdr session reached only through the SSH path (as machines.sh).
if [[ $status -eq 0 ]]; then
  nohup "$herdr" --session "$remote" server >/dev/null 2>&1 &
  for _ in $(seq 50); do
    "$herdr" --session "$remote" workspace list >/dev/null 2>&1 && break
    sleep 0.1
  done
  "$herdr" --session "$remote" workspace create --label on-remote >/dev/null
  cp "$root/scripts/ssh-standin.sh" "$side/ssh"
  cat >"$side/machines.json" <<EOF
[{"id":"0123456789abcdef0123456789abcdef","label":"standin","target":"standin-host",
  "session":"$remote","enabled":true,"selected":false}]
EOF
  cat >"$dev/config/azulejo/config.toml" <<EOF
# Dev profile config (written by automation/sidebar.sh)
[herdr]
session = "azj-dev-$AZJ_DEV_NAME"
machines = true
machines_file = "$side/machines.json"
ssh = "$side/ssh"
EOF
  "$root/apps/macos/dev.sh" --automation "$fixtures/sidebar-relaunch.azs" --out "$out/relaunch" || status=1
  [[ "$(cat "$out/relaunch/result" 2>/dev/null)" == ok ]] || status=1
  echo "sidebar relaunch: $(cat "$out/relaunch/result" 2>/dev/null || echo 'no result')"
fi
kill "$watchdog" 2>/dev/null || true
exit "$status"
