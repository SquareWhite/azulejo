#!/usr/bin/env bash
# PLAN 0.5 spike: three herdr panes in the throwaway `azj-anim` session, three
# libghostty surfaces attached to them, animated layout changes.
#
#   ./run.sh [app args...]   e.g. ./run.sh --stress 50 --exit --log build/run.jsonl
#   ./run.sh --stop          stop the azj-anim session (never the default one)
#
# The app launches in the background (open -g semantics: accessory app, never
# activates, window ordered to the back). No synthetic input anywhere.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SESSION="azj-anim"
SOCK="$HOME/.config/herdr/sessions/$SESSION/herdr.sock"
HERDR="$(command -v herdr)"
APP="$HERE/build/AzjAnim.app/Contents/MacOS/AzjAnim"
OUT="$HERE/build"
mkdir -p "$OUT"

unset HERDR_PANE_ID HERDR_WORKSPACE_ID HERDR_TAB_ID HERDR_ENV
export HERDR_SOCKET_PATH="$SOCK"
h() { "$HERDR" --session "$SESSION" "$@"; }

if [[ "${1:-}" == "--stop" ]]; then h server stop; exit 0; fi

if ! h workspace list >/dev/null 2>&1; then
  echo "==> starting headless herdr session $SESSION" >&2
  # Clean env so nothing from this shell (e.g. CLAUDE_CODE_*) leaks into panes.
  env -i HOME="$HOME" USER="$USER" PATH="$PATH" SHELL="${SHELL:-/bin/zsh}" TERM=xterm-256color \
    LANG="${LANG:-en_GB.UTF-8}" HERDR_SOCKET_PATH="$SOCK" \
    nohup "$HERDR" --session "$SESSION" server >"$OUT/herdr-server.log" 2>&1 &
  for _ in $(seq 50); do h workspace list >/dev/null 2>&1 && break; sleep 0.1; done
  WS="$(h workspace create --label anim --cwd "$HOME" | python3 -c 'import json,sys; r=json.load(sys.stdin)["result"]; print(r["workspace"]["workspace_id"])')"
  h tab create --workspace "$WS" --no-focus >/dev/null
  h tab create --workspace "$WS" --no-focus >/dev/null
  sleep 0.5
  mapfile -t PANES < <(h pane list | python3 -c 'import json,sys; [print(p["pane_id"]) for p in json.load(sys.stdin)["result"]["panes"]]')
  h pane run "${PANES[0]}" "htop -d 100" >/dev/null
  h pane run "${PANES[1]}" "bash '$HERE/winch.sh' B '$OUT/winch-B.log'" >/dev/null
  h pane run "${PANES[2]}" "bash '$HERE/winch.sh' C '$OUT/winch-C.log'" >/dev/null
fi

mapfile -t TERMS < <(h pane list | python3 -c 'import json,sys; [print(p["terminal_id"]) for p in json.load(sys.stdin)["result"]["panes"]]')
echo "==> attaching A=${TERMS[0]} B=${TERMS[1]} C=${TERMS[2]}" >&2
[[ -x "$APP" ]] || "$HERE/build.sh"

ARGS=()
for t in "${TERMS[@]:0:3}"; do ARGS+=(--attach "$HERDR --session $SESSION terminal attach $t --takeover"); done
exec "$APP" "${ARGS[@]}" --env "HERDR_SOCKET_PATH=$SOCK" "$@"
