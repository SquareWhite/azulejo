#!/usr/bin/env bash
# The mockup's scene (PLAN D.9), built with fixture data on its own dev
# instance (AZJ_DEV_NAME=mock), then captured next to docs/design/mockup-v1.png.
# Local workspaces are small fixture repos; "build-server" and "dev-server" are
# throwaway local herdr sessions reached through the ssh stand-in
# (scripts/ssh-standin.sh) — nothing connects to a real machine. The browser
# tile shows a static fixture page served from a temp folder.
#   apps/macos/automation/mockup.sh [--keep]   (--keep: leave the instance)
# Output: .dev/<name>.out/mockup/01-scene.png (system capture, needs an
# unlocked screen) and 01-scene-app.png (the app's own render).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-mock}"
keep="${1:-}"
dev="$root/.dev/$AZJ_DEV_NAME"
side="$root/.dev/$AZJ_DEV_NAME.machines"
out="$root/.dev/$AZJ_DEV_NAME.out/mockup"
remote_b="azj-dev-$AZJ_DEV_NAME-build"
remote_t="azj-dev-$AZJ_DEV_NAME-dev"
fx="$(mktemp -d "${TMPDIR:-/tmp}/azj-mock.XXXXXX")"
fx="$(cd "$fx" && pwd -P)"

# Never the user's herdr: every herdr call names a throwaway session.
unset HERDR_SOCKET_PATH HERDR_PANE_ID HERDR_WORKSPACE_ID HERDR_TAB_ID HERDR_ENV
herdr="$(command -v herdr || echo "$HOME/.local/bin/herdr")"

stop_remotes() {
  for s in "$remote_b" "$remote_t"; do "$herdr" --session "$s" server stop >/dev/null 2>&1 || true; done
  sleep 0.5
  for s in "$remote_b" "$remote_t"; do "$herdr" session delete "$s" >/dev/null 2>&1 || true; done
}
cleanup() {
  kill "${watchdog:-}" "${watcher:-}" "${server:-}" 2>/dev/null || true
  rm -rf "$fx"
  [[ "$keep" == "--keep" ]] && return
  "$root/apps/macos/dev.sh" --reset >/dev/null 2>&1 || true
  stop_remotes
  rm -rf "$side"
}
trap cleanup EXIT

"$root/apps/macos/dev.sh" --reset >/dev/null
stop_remotes   # left over from a --keep run
rm -rf "$side" "$out"
mkdir -p "$side" "$out" "$(dirname "$dev/config/azulejo/config.toml")"

# Fixture repos, on the mockup's branches.
repo() {
  local dir="$fx/repos/$1"
  mkdir -p "$dir"
  printf '# %s\n' "$1" >"$dir/README.md"
  git -C "$dir" -c init.defaultBranch="$2" init -q
  git -C "$dir" add -A
  git -C "$dir" -c user.email=t@t -c user.name=t -c commit.gpgsign=false commit -qm init
}
repo azulejo main
repo sample-service development
repo sample-app main
repo sample-api main
repo sample-project audit/critical-path-review
repo sample-dashboard dev
repo config main

# The browser tile's page: a static fixture on a local port.
mkdir -p "$fx/site/patterns/14"
cp "$here/mockup/pattern.html" "$fx/site/patterns/14/index.html"
cp "$here/mockup/0009-pane-exit.md" "$here/mockup/claude.txt" "$fx/"
cp "$here/mockup/just" "$fx/just"
port="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')"
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$fx/site" >/dev/null 2>&1 &
server=$!

sed -e "s|@FX@|$fx|g" -e "s|@REPO@|$root|g" -e "s|@PORT@|$port|g" \
  "$here/mockup/scene.toml.in" >"$fx/scene.toml"

# The two "remote" machines: throwaway herdr sessions with one workspace each.
for s in "$remote_b" "$remote_t"; do
  nohup "$herdr" --session "$s" server >/dev/null 2>&1 &
  for _ in $(seq 50); do "$herdr" --session "$s" workspace list >/dev/null 2>&1 && break; sleep 0.1; done
done
# A fresh herdr server opens a "~" workspace of its own; keep only ours.
only_workspace() {
  "$herdr" --session "$1" workspace create --label "$2" >/dev/null
  "$herdr" --session "$1" workspace list | python3 -c '
import json, sys
for w in json.load(sys.stdin)["result"]["workspaces"]:
    if w["label"] != sys.argv[1]: print(w["workspace_id"])' "$2" |
    while read -r id; do "$herdr" --session "$1" workspace close "$id" >/dev/null; done
}
only_workspace "$remote_b" infra
only_workspace "$remote_t" notes

# ssh: the stand-in, except dev-server fails while $side/ssh.dev-server.fail exists.
cp "$root/scripts/ssh-standin.sh" "$side/standin"
cat >"$side/ssh" <<'SH'
#!/bin/sh
skip=0; target=""
for a in "$@"; do
  if [ "$skip" = 1 ]; then skip=0; continue; fi
  case "$a" in -o|-F|-S|-i|-l|-p|-J) skip=1 ;; --) break ;; -*) ;; *) target="$a"; break ;; esac
done
if [ "$target" = "dev-host" ] && [ -e "$0.dev-server.fail" ]; then
  echo "ssh: connect to host dev-host port 22: Connection refused" >&2
  exit 255
fi
exec "$(dirname "$0")/standin" "$@"
SH
chmod +x "$side/ssh" "$side/standin"
cat >"$side/machines.json" <<JSON
[{"id":"0123456789abcdef0123456789abcdef","label":"build-server","target":"build-host",
  "session":"$remote_b","enabled":true,"selected":false},
 {"id":"fedcba9876543210fedcba9876543210","label":"dev-server","target":"dev-host",
  "session":"$remote_t","enabled":true,"selected":false}]
JSON
cat >"$dev/config/azulejo/config.toml" <<TOML
# Dev profile config (written by automation/mockup.sh)
[herdr]
session = "azj-dev-$AZJ_DEV_NAME"
machines = true
machines_file = "$side/machines.json"
ssh = "$side/ssh"
TOML

sed -e "s|@FX@|$fx|g" -e "s|@SIDE@|$side|g" -e "s|@REMOTE_T@|$remote_t|g" \
  "$here/mockup.azs.in" >"$fx/mockup.azs"

# The app asks for window captures by dropping files in $out/.capture.
watch_captures() {
  mkdir -p "$out/.capture"
  while [[ ! -f "$out/result" ]]; do
    for req in "$out/.capture"/*; do
      [[ -e "$req" ]] || continue
      name="$(basename "$req")"; rm -f "$req"
      wid="$(cat "$out/window-id" 2>/dev/null || true)"
      [[ -n "$wid" ]] && /usr/sbin/screencapture -l "$wid" -x -o "$out/$name.png" 2>/dev/null || true
    done
    sleep 0.05
  done
}
watch_captures & watcher=$!
( sleep 240; "$root/apps/macos/dev.sh" --stop ) & watchdog=$!

status=0
"$root/apps/macos/dev.sh" --automation "$fx/mockup.azs" --out "$out" || status=1
[[ "$(cat "$out/result" 2>/dev/null)" == ok ]] || status=1
echo "mockup: $(cat "$out/result" 2>/dev/null || echo 'no result') ($out)"
ls "$out"/*.png 2>/dev/null || true
exit "$status"
