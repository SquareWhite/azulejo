#!/usr/bin/env bash
# Runs Azulejo.app against an isolated dev profile: its own config, state, UI
# settings (state/settings.plist, not the user defaults), API socket and herdr
# session (`azj-dev`), all under .dev/ in the repo. The user's real herdr
# session, saved Azulejo state and defaults are never touched.
#
#   apps/macos/dev.sh [--bg] [--automation FILE] [--out DIR] [--wait]
#   apps/macos/dev.sh --stop      quit the dev app
#   apps/macos/dev.sh --reset     quit the dev app, stop and delete the herdr session, wipe .dev state
#   apps/macos/dev.sh --env       print the env that points `azj` at the dev app
#
#   --bg            launch in the background: never activates or raises a window
#   --automation F  run an in-process automation script (implies --bg and --wait)
#   --out DIR       where automation writes screenshots and its log (default .dev/out)
#   --wait          wait for the app to exit
#
# AZJ_DEV_NAME=<name> runs a separate instance: everything under .dev/<name>/ and
# herdr session `azj-dev-<name>`, so several agents can test side by side.
# Without it the profile lives directly in .dev/ with session `azj-dev`.
# AZJ_DEV_SESSION overrides the herdr session name; "default" is refused.
#
# AZJ_HERDR_BIN=<file> runs that herdr (e.g. your own build) instead of the one
# on PATH: the server the app starts, its herdr calls, every terminal tile's
# attach, and --reset. AZJ_HERDR_CONFIG=<file> hands the dev session's herdr
# that config.toml instead of herdr's own; without it, the profile's
# .dev/<name>/config/herdr/config.toml is used if it exists. A running herdr
# server keeps the binary and config it started with: --reset, then launch.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
name="${AZJ_DEV_NAME:-}"
case "$name" in
  "") dev="$root/.dev"; default_session="azj-dev" ;;
  config | state | run | logs | out) echo "dev.sh: AZJ_DEV_NAME=$name clashes with a .dev/ folder" >&2; exit 2 ;;
  *[!A-Za-z0-9_-]*) echo "dev.sh: AZJ_DEV_NAME may only use letters, digits, - and _" >&2; exit 2 ;;
  *) dev="$root/.dev/$name"; default_session="azj-dev-$name" ;;
esac
app="$root/target/macos/Azulejo.app"
session="${AZJ_DEV_SESSION:-$default_session}"
config="$dev/config/azulejo/config.toml"
state="$dev/state/state.json"
socket="$dev/run/azulejo.sock"
pidfile="$dev/run/app.pid"
logfile="$dev/logs/app.log"

# The whole point of the profile: never the user's own herdr session.
case "$session" in
  "" | default)
    echo "dev.sh: refusing to run the dev profile against herdr's default session" >&2
    exit 2
    ;;
esac

# FILE given in VAR, as an absolute path with symlinked folders resolved
# (the app runs from /); fails loudly if it isn't a file.
abs_file() {
  local var="$1" p="$2"
  [[ "$p" == /* ]] || p="$PWD/$p"
  [[ -f "$p" ]] || { echo "dev.sh: $var=$2: no such file" >&2; return 2; }
  echo "$(cd -P "$(dirname "$p")" && pwd)/$(basename "$p")"
}

# A herdr build of our own, instead of the one on PATH.
herdr_bin=""
if [[ -n "${AZJ_HERDR_BIN:-}" ]]; then
  herdr_bin="$(abs_file AZJ_HERDR_BIN "$AZJ_HERDR_BIN")"
  [[ -x "$herdr_bin" ]] || { echo "dev.sh: AZJ_HERDR_BIN=$AZJ_HERDR_BIN: not executable" >&2; exit 2; }
fi
# The dev session's herdr config: never the user's own.
herdr_config=""
if [[ -n "${AZJ_HERDR_CONFIG:-}" ]]; then
  herdr_config="$(abs_file AZJ_HERDR_CONFIG "$AZJ_HERDR_CONFIG")"
elif [[ -f "$dev/config/herdr/config.toml" ]]; then
  herdr_config="$dev/config/herdr/config.toml"
fi
if [[ -n "$herdr_config" ]]; then
  user_herdr="$(cd -P "${XDG_CONFIG_HOME:-$HOME/.config}" 2>/dev/null && pwd)/herdr/"
  case "$herdr_config" in
    "$user_herdr"*) echo "dev.sh: AZJ_HERDR_CONFIG points into your own herdr config folder; refusing" >&2; exit 2 ;;
  esac
fi

# Our own shell may sit inside the user's herdr session; nothing below may
# inherit its socket.
herdr_env=(env -u HERDR_SOCKET_PATH -u HERDR_PANE_ID -u HERDR_TAB_ID -u HERDR_WORKSPACE_ID -u HERDR_ENV -u HERDR_CONFIG_PATH)
if [[ -n "$herdr_config" ]]; then herdr_env+=("HERDR_CONFIG_PATH=$herdr_config"); fi
herdr_dev() {
  "${herdr_env[@]}" "${herdr_bin:-herdr}" --session "$session" "$@"
}

running_pid() {
  [[ -f "$pidfile" ]] || return 1
  local pid
  pid="$(cat "$pidfile")"
  kill -0 "$pid" 2>/dev/null && ps -p "$pid" -o comm= | grep -q Azulejo && echo "$pid"
}

stop_app() {
  local pid
  if pid="$(running_pid)"; then
    kill -TERM "$pid"
    for _ in $(seq 50); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
    kill -0 "$pid" 2>/dev/null && kill -KILL "$pid"
  fi
  rm -f "$pidfile"
}

bg=0 wait=0 automation="" out="$dev/out"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bg) bg=1 ;;
    --wait) wait=1 ;;
    --automation) automation="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; bg=1; wait=1; shift ;;
    --out) out="$2"; shift ;;
    --stop) stop_app; exit 0 ;;
    --reset)
      stop_app
      herdr_dev server stop >/dev/null 2>&1 || true
      herdr_dev session delete "$session" >/dev/null 2>&1 \
        || "${herdr_env[@]}" "${herdr_bin:-herdr}" session delete "$session" >/dev/null 2>&1 || true
      rm -rf "$dev/state" "$dev/run" "$dev/out" "$dev/logs"
      echo "dev profile reset (herdr session $session stopped and deleted)"
      exit 0
      ;;
    --env) echo "AZULEJO_SOCKET=$socket"; exit 0 ;;
    -h | --help) sed -n '2,26p' "$0"; exit 0 ;;
    *) echo "dev.sh: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done

[[ -d "$app" ]] || { echo "dev.sh: build first (just mac)" >&2; exit 1; }
# macOS caps Unix socket paths at 103 bytes; the core would refuse to start.
if (( ${#socket} > 103 )); then
  echo "dev.sh: socket path is ${#socket} bytes, over macOS's 103: $socket" >&2
  echo "dev.sh: run from a shorter checkout path or use a shorter AZJ_DEV_NAME" >&2
  exit 2
fi
if pid="$(running_pid)"; then
  echo "dev.sh: the dev app is already running (pid $pid); use --stop" >&2
  exit 1
fi

mkdir -p "$(dirname "$config")" "$(dirname "$state")" "$dev/run" "$dev/logs" "$out"
out="$(cd "$out" && pwd)" # the app runs from /, so it needs an absolute path
chmod 700 "$dev/run"
if [[ ! -f "$config" ]]; then
  printf '# Dev profile config (generated by apps/macos/dev.sh)\n[herdr]\nsession = "%s"\n' \
    "$session" >"$config"
fi
if grep -Eq '^[[:space:]]*session[[:space:]]*=[[:space:]]*"default"' "$config"; then
  echo "dev.sh: $config points at herdr's default session; refusing" >&2
  exit 2
fi

# The app reads these; AZULEJO_PROFILE=dev makes it refuse herdr's default
# session on its own too.
# The Ghostty settings Azulejo honours itself (unfocused-split-opacity and
# -fill) come from the profile's own Ghostty file, which may not exist: dev
# runs never depend on the user's Ghostty config for them. libghostty still
# reads the user's Ghostty config for the terminals' fonts and colours.
ghostty="$dev/config/ghostty/config"
options="$(printf '{"config_path":"%s","ghostty_config_path":"%s","state_path":"%s","socket_path":"%s","herdr_session":"%s","api_required":true}' \
  "$config" "$ghostty" "$state" "$socket" "$session")"
envs=(--env "AZULEJO_PROFILE=dev" --env "AZULEJO_CORE_OPTIONS=$options" --env "AZULEJO_PIDFILE=$pidfile")
if [[ $bg -eq 1 ]]; then envs+=(--env "AZULEJO_BACKGROUND=1"); fi
# The core honours these on the dev profile only (azulejo_herdr::Herdr::locate).
if [[ -n "$herdr_bin" ]]; then envs+=(--env "AZJ_HERDR_BIN=$herdr_bin"); fi
if [[ -n "$herdr_config" ]]; then envs+=(--env "AZJ_HERDR_CONFIG=$herdr_config"); fi
if [[ -n "$herdr_bin$herdr_config" ]]; then
  echo "dev.sh: herdr ${herdr_bin:-from PATH}${herdr_config:+, config $herdr_config}"
  if herdr_dev status 2>/dev/null | grep -q '^ *status: running'; then
    echo "dev.sh: note: herdr session $session is already running and keeps the binary and config it started with (--reset to restart it)"
  fi
fi
# `open` doesn't pass our environment on: hand the core's debug switches over
# (AZJ_DEBUG_MIRROR=1 logs every mirror push and pull to the app log).
if [[ -n "${AZJ_DEBUG_MIRROR:-}" ]]; then envs+=(--env "AZJ_DEBUG_MIRROR=$AZJ_DEBUG_MIRROR"); fi
# A delay before every folder listing, standing in for a slow folder (files-slow.sh).
if [[ -n "${AZJ_FILES_LIST_DELAY_MS:-}" ]]; then envs+=(--env "AZJ_FILES_LIST_DELAY_MS=$AZJ_FILES_LIST_DELAY_MS"); fi
# The Files view's deletes go to this folder instead of the Trash (tests).
if [[ -n "${AZJ_TRASH_DIR:-}" ]]; then envs+=(--env "AZJ_TRASH_DIR=$AZJ_TRASH_DIR"); fi
# Reflow motion tuning (Motion.reflowInterval, Motion.reflowLead).
for v in AZJ_REFLOW_MS AZJ_REFLOW_LEAD_MS; do
  if [[ -n "${!v:-}" ]]; then envs+=(--env "$v=${!v}"); fi
done
if [[ -n "$automation" ]]; then
  rm -f "$out/result"
  envs+=(--env "AZULEJO_AUTOMATION=$automation" --env "AZULEJO_AUTOMATION_OUT=$out")
fi

flags=(-n)
[[ $bg -eq 1 ]] && flags+=(-g)
# A pidfile left by a crash would make this launch look dead.
rm -f "$pidfile"
: >"$logfile"
open "${flags[@]}" "${envs[@]}" --stdout "$logfile" --stderr "$logfile" "$app"

# The app failed to start: say so loudly, with the end of its log.
died() {
  echo "dev.sh: FAILED: $1" >&2
  if [[ -s "$logfile" ]]; then
    echo "dev.sh: last lines of $logfile:" >&2
    tail -n 15 "$logfile" | sed 's/^/  | /' >&2
  fi
  exit 1
}

# Wait for the app to write its pid (it does once the core is up).
for _ in $(seq 100); do
  [[ -s "$pidfile" ]] && break
  # The app logs this and exits when the core can't start; don't sit out 10 s.
  grep -q "the core didn't start\|azulejo: the dev profile\|azulejo: dev profile ended up" "$logfile" 2>/dev/null && break
  sleep 0.1
done
pid="$(cat "$pidfile" 2>/dev/null || true)"
[[ -n "$pid" ]] || died "the app never started (no pid after 10 s)"
# With the API required, the socket appears once the core is serving.
for _ in $(seq 100); do
  [[ -S "$socket" ]] && break
  kill -0 "$pid" 2>/dev/null || died "the app exited before serving its socket"
  sleep 0.1
done
[[ -S "$socket" ]] || died "no API socket at $socket after 10 s"
echo "dev app running (pid $pid); herdr session $session; socket $socket; log $logfile"

if [[ $wait -eq 1 ]]; then
  while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
  if [[ -n "$automation" ]]; then
    result="$(cat "$out/result" 2>/dev/null || true)"
    [[ -n "$result" ]] || died "the app exited without an automation result (log $out/automation.log)"
    echo "automation: $result (log $out/automation.log)"
    [[ "$result" == ok* ]] || exit 1
  fi
fi
