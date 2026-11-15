#!/usr/bin/env bash
# PTY resize latency through `herdr terminal attach`, no libghostty: a
# throwaway herdr session (e2e-resizeprobe) with one pane running
# resize_pane.py, and resize_client.py standing in for libghostty.
#
#   scripts/latency/resize-probe.sh single 40          isolated resizes, 300 ms apart
#   scripts/latency/resize-probe.sh burst 33 200 10    a resize every 33 ms for 200 ms, ×10
#   scripts/latency/resize-probe.sh usr1 30            pane output with no resize
#   scripts/latency/resize-probe.sh --stop             stop and delete the session
#
# Prints per hop: PTY ioctl -> SIGWINCH in the pane, pane write -> bytes at
# the client. docs/measurements.md has the Nov 4 numbers.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
session=e2e-resizeprobe
sock="$HOME/.config/herdr/sessions/$session/herdr.sock"
work="${TMPDIR:-/tmp}/azj-resize-probe"
mkdir -p "$work"
unset HERDR_PANE_ID HERDR_WORKSPACE_ID HERDR_TAB_ID HERDR_ENV
export HERDR_SOCKET_PATH="$sock"
h() { herdr --session "$session" "$@"; }
if [[ "${1:-}" == "--stop" ]]; then
  h server stop >/dev/null 2>&1 || true
  sleep 0.5
  env -u HERDR_SOCKET_PATH herdr session delete "$session" >/dev/null 2>&1 || true
  exit 0
fi
if ! h workspace list >/dev/null 2>&1; then
  env -i HOME="$HOME" USER="$USER" PATH="$PATH" SHELL=/bin/sh TERM=xterm-256color HERDR_SOCKET_PATH="$sock" \
    nohup herdr --session "$session" server >"$work/server.log" 2>&1 &
  for _ in $(seq 50); do h workspace list >/dev/null 2>&1 && break; sleep 0.1; done
  h workspace create --label probe --cwd "$HOME" >/dev/null
  sleep 0.5
  pane="$(h pane list | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["panes"][0]["pane_id"])')"
  : >"$work/pane.log"
  h pane run "$pane" "python3 '$here/resize_pane.py' '$work/pane.log'" >/dev/null
  sleep 1
fi
term="$(h pane list | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["panes"][0]["terminal_id"])')"
python3 "$here/resize_client.py" "$session" "$term" "$work/pane.log" "$@"
