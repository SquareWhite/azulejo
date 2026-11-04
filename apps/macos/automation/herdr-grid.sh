#!/usr/bin/env bash
# herdr-grid.sh HERDR SESSION [LABEL]: a herdr workspace whose tab is a 2×2
# grid of panes, split the way the TUI does it (right, then down in each
# column). The running dev app adopts it as a manual tab (ADR 0008, 0010 §5),
# as it adopts the maintainer's own workspaces. Automation calls it with `sh`:
#
#     sh "$(dirname "$AZULEJO_AUTOMATION")/herdr-grid.sh" {herdr} {session}
#
# Throwaway dev sessions only: refuses herdr's default session.
set -euo pipefail
herdr="${1:?herdr binary}" session="${2:?session}" label="${3:-grid}"
case "$session" in "" | default) echo "herdr-grid: refusing herdr's default session" >&2; exit 2 ;; esac
case "$session" in azj-*) ;; *) echo "herdr-grid: only azj-* dev sessions, not $session" >&2; exit 2 ;; esac
unset HERDR_SOCKET_PATH HERDR_PANE_ID HERDR_TAB_ID HERDR_WORKSPACE_ID HERDR_ENV
export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
h() { "$herdr" --session "$session" "$@"; }
# The new pane's ID in a `workspace create` or `pane split` reply.
pane_id() {
  python3 -c 'import json, sys
r = json.load(sys.stdin)["result"]
print((r.get("root_pane") or r.get("pane") or r)["pane_id"])'
}
# A pause after each step, as a person splitting in the TUI makes: the app
# adopts the workspace, then each split-off pane beside its sibling. (Four
# panes in a burst can reach a busy app's survey half made, and it adopts
# them in another shape; the mirror then pushes that shape back to herdr.)
top_left="$(h workspace create --label "$label" | pane_id)"
sleep 0.5
top_right="$(h pane split "$top_left" --direction right | pane_id)"
sleep 0.5
h pane split "$top_left" --direction down >/dev/null
sleep 0.5
h pane split "$top_right" --direction down >/dev/null
