#!/usr/bin/env bash
# Launch AzjSpike attached to a pane in the throwaway `azj-spike` herdr session.
#
#   ./run.sh            start the session if needed, attach to its first pane
#   ./run.sh --new      create a fresh tab/pane and attach to it
#   ./run.sh --stop     stop the azj-spike session (never touches the default one)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SESSION="azj-spike"
SOCK="$HOME/.config/herdr/sessions/$SESSION/herdr.sock"
HERDR="$(command -v herdr)"
APP="$HERE/build/AzjSpike.app/Contents/MacOS/AzjSpike"

# Never let anything here talk to the user's live session: drop the herdr
# env inherited from a parent pane and pin the spike session's socket.
unset HERDR_PANE_ID HERDR_WORKSPACE_ID HERDR_TAB_ID HERDR_ENV
export HERDR_SOCKET_PATH="$SOCK"
h() { "$HERDR" --session "$SESSION" "$@"; }

if [[ "${1:-}" == "--stop" ]]; then
  h server stop
  exit 0
fi

if ! h workspace list >/dev/null 2>&1; then
  echo "==> starting headless herdr session $SESSION"
  nohup "$HERDR" --session "$SESSION" server >"$HERE/build/herdr-server.log" 2>&1 &
  for _ in $(seq 50); do h workspace list >/dev/null 2>&1 && break; sleep 0.1; done
fi

if [[ "${1:-}" == "--new" ]]; then
  h tab create --no-focus >/dev/null
fi

PANE_JSON="$(h pane list)"
TERM_ID="$(python3 -c 'import json,sys; p=json.load(sys.stdin)["result"]["panes"]; print(p[-1]["terminal_id"])' <<<"$PANE_JSON")"
PANE_ID="$(python3 -c 'import json,sys; p=json.load(sys.stdin)["result"]["panes"]; print(p[-1]["pane_id"])' <<<"$PANE_JSON")"
echo "==> attaching to $PANE_ID ($TERM_ID)"

[[ -x "$APP" ]] || "$HERE/build.sh"

exec "$APP" \
  --command "$HERDR --session $SESSION terminal attach $TERM_ID --takeover" \
  --env "HERDR_SOCKET_PATH=$SOCK" \
  --env "AZJ_SPIKE_PANE=$PANE_ID"
