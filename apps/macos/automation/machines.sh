#!/usr/bin/env bash
# herdr machines in the sidebar (PLAN 2.13, ADR 0011), in-process on their
# own dev instance (AZJ_DEV_NAME=mm). The "remote" machine is a second,
# throwaway local herdr session reached only through the SSH path: an ssh
# stand-in (scripts/ssh-standin.sh) runs herdr's remote-api-bridge and
# `terminal attach` here. Nothing connects to a real machine.
#   apps/macos/automation/machines.sh [--keep]   (--keep: leave the instance)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-mm}"
keep="${1:-}"
dev="$root/.dev/$AZJ_DEV_NAME"
side="$root/.dev/$AZJ_DEV_NAME.machines"
out="$root/.dev/$AZJ_DEV_NAME.out"
remote="azj-dev-$AZJ_DEV_NAME-remote"

# Never the user's herdr: every herdr call names a throwaway session.
unset HERDR_SOCKET_PATH HERDR_PANE_ID HERDR_WORKSPACE_ID HERDR_TAB_ID HERDR_ENV
herdr="$(command -v herdr || echo "$HOME/.local/bin/herdr")"

cleanup() {
  [[ "$keep" == "--keep" ]] && return
  "$root/apps/macos/dev.sh" --reset >/dev/null 2>&1 || true
  "$herdr" --session "$remote" server stop >/dev/null 2>&1 || true
  sleep 0.5
  "$herdr" session delete "$remote" >/dev/null 2>&1 || true
  rm -rf "$side"
}
trap cleanup EXIT

"$root/apps/macos/dev.sh" --reset >/dev/null
rm -rf "$side" "$out"
mkdir -p "$side" "$(dirname "$dev/config/azulejo/config.toml")"

# The "remote" herdr: running, with one workspace of its own.
nohup "$herdr" --session "$remote" server >/dev/null 2>&1 &
for _ in $(seq 50); do
  "$herdr" --session "$remote" workspace list >/dev/null 2>&1 && break
  sleep 0.1
done
"$herdr" --session "$remote" workspace create --label on-remote >/dev/null

cp "$root/scripts/ssh-standin.sh" "$side/ssh"
cat >"$side/machines.json" <<EOF
[{"id":"0123456789abcdef0123456789abcdef","label":"standin","target":"standin-host",
  "session":"$remote","enabled":true,"selected":false},
 {"id":"fedcba9876543210fedcba9876543210","label":"parked","target":"parked-host",
  "session":"default","enabled":false,"selected":false}]
EOF
cat >"$dev/config/azulejo/config.toml" <<EOF
# Dev profile config (written by automation/machines.sh)
[herdr]
session = "azj-dev-$AZJ_DEV_NAME"
machines = true
machines_file = "$side/machines.json"
ssh = "$side/ssh"
EOF

( sleep 180; "$root/apps/macos/dev.sh" --stop ) &
watchdog=$!
status=0
"$root/apps/macos/dev.sh" --automation "$here/machines.azs" --out "$out" || status=1
kill "$watchdog" 2>/dev/null || true
[[ "$(cat "$out/result" 2>/dev/null)" == ok ]] || status=1
exit "$status"
