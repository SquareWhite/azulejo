# Shared helpers for the PLAN 1.13 NFR scripts. Source it; don't run it.
#
# Everything runs on the dev instance `nfr` (.dev/nfr, herdr session
# azj-dev-nfr): never the user's own herdr session or saved state. The app is
# launched in the background (never activated, windows ordered back) and no
# OS input is posted.
#
#   NFR_APP=/path/Azulejo.app   which bundle to launch (default: the debug build)

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-nfr}"
dev="$root/.dev/$AZJ_DEV_NAME"
session="azj-dev-$AZJ_DEV_NAME"
config="$dev/config/azulejo/config.toml"
state="$dev/state/state.json"
sock="$dev/run/azulejo.sock"
pidfile="$dev/run/app.pid"
log="$dev/logs/app.log"
out="$root/scripts/nfr/build"
app="${NFR_APP:-$root/target/macos/Azulejo.app}"
dev_sh="$root/apps/macos/dev.sh"

clean_env=(env -u HERDR_SOCKET_PATH -u HERDR_PANE_ID -u HERDR_TAB_ID -u HERDR_WORKSPACE_ID -u HERDR_ENV
  -u AZULEJO_TILE_ID -u AZULEJO_WORKSPACE_ID -u AZULEJO_SOCKET)
azj() { "${clean_env[@]}" AZULEJO_SOCKET="$sock" "$root/target/debug/azj" --no-caller "$@"; }
hd() { "${clean_env[@]}" herdr --session "$session" "$@"; }
say() { printf '\n== %s\n' "$*"; }
fail() { echo "FAIL: $*" >&2; exit 1; }

case "$session" in "" | default) fail "refusing herdr's default session" ;; esac

prepare() {
  mkdir -p "$(dirname "$config")" "$(dirname "$state")" "$dev/run" "$dev/logs" "$out"
  chmod 700 "$dev/run"
  [[ -f "$config" ]] || printf '[herdr]\nsession = "%s"\n' "$session" >"$config"
}

running_pid() {
  [[ -f "$pidfile" ]] || return 1
  local pid
  pid="$(cat "$pidfile")"
  kill -0 "$pid" 2>/dev/null && echo "$pid"
}

# launch [KEY=VALUE ...]: start the app in the background with the dev
# profile's options plus extra environment, and wait for its pid file.
launch() {
  prepare
  local options envs=() kv
  options="$(printf '{"config_path":"%s","state_path":"%s","socket_path":"%s","herdr_session":"%s","api_required":true}' \
    "$config" "$state" "$sock" "$session")"
  envs=(--env "AZULEJO_PROFILE=dev" --env "AZULEJO_CORE_OPTIONS=$options" --env "AZULEJO_PIDFILE=$pidfile"
    --env "AZULEJO_BACKGROUND=1")
  for kv in "$@"; do envs+=(--env "$kv"); done
  rm -f "$pidfile"
  : >"$log"
  "${clean_env[@]}" open -n -g "${envs[@]}" --stdout "$log" --stderr "$log" "$app"
  for _ in $(seq 200); do [[ -s "$pidfile" ]] && break; sleep 0.05; done
  running_pid >/dev/null || fail "app didn't start (see $log)"
}

# The same launch, but exec'ing the binary from this shell, so ScreenCaptureKit
# permission comes from the terminal running the script (as in 0.4).
launch_direct() {
  prepare
  local options
  options="$(printf '{"config_path":"%s","state_path":"%s","socket_path":"%s","herdr_session":"%s","api_required":true}' \
    "$config" "$state" "$sock" "$session")"
  rm -f "$pidfile"
  : >"$log"
  "${clean_env[@]}" AZULEJO_PROFILE=dev AZULEJO_CORE_OPTIONS="$options" AZULEJO_PIDFILE="$pidfile" \
    AZULEJO_BACKGROUND=1 "$@" "$app/Contents/MacOS/Azulejo" >>"$log" 2>&1 &
  for _ in $(seq 200); do [[ -s "$pidfile" ]] && break; sleep 0.05; done
  running_pid >/dev/null || fail "app didn't start (see $log)"
}

wait_ping() {
  for _ in $(seq 200); do azj core ping >/dev/null 2>&1 && return 0; sleep 0.05; done
  fail "app didn't answer on $sock"
}

# Waits until the session has at least n tiles (the app creates its first
# window and workspace after the API is already up).
wait_tiles() {
  local n=${1:-1}
  for _ in $(seq 200); do
    [[ $(azj --json tile list 2>/dev/null | jq '.tiles | length' 2>/dev/null || echo 0) -ge $n ]] && return 0
    sleep 0.05
  done
  fail "session never reached $n tiles"
}

stop() {
  local pid
  if pid="$(running_pid)"; then
    kill -TERM "$pid"
    for _ in $(seq 100); do kill -0 "$pid" 2>/dev/null || break; sleep 0.05; done
    kill -0 "$pid" 2>/dev/null && kill -KILL "$pid"
  fi
  rm -f "$pidfile"
}

# A fresh, empty herdr session for the dev instance, server already running
# (started headless with a clean env), so app launches don't pay for it and
# adoption finds nothing left over from earlier runs.
herdr_fresh() {
  hd server stop >/dev/null 2>&1 || true
  for _ in $(seq 50); do hd workspace list >/dev/null 2>&1 || break; sleep 0.1; done
  hd session delete "$session" >/dev/null 2>&1 || true
  local herdr_bin
  herdr_bin="$(command -v herdr)"
  env -i HOME="$HOME" PATH="/usr/bin:/bin:/usr/sbin:/sbin:$(dirname "$herdr_bin")" USER="$USER" \
    TERM=xterm-256color SHELL=/bin/zsh LANG=en_GB.UTF-8 TMPDIR="$TMPDIR" \
    nohup "$herdr_bin" --session "$session" server >"$out/herdr-server.log" 2>&1 &
  for _ in $(seq 100); do hd workspace list >/dev/null 2>&1 && return 0; sleep 0.1; done
  fail "herdr server for $session didn't start"
}

reset_all() {
  stop
  "$dev_sh" --reset >/dev/null 2>&1 || true
}
