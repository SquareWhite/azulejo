#!/usr/bin/env bash
# Waits up to 4 s for a herdr pane's screen to contain a fixed string, or with
# --not checks for 1 s that it doesn't. focus.azs uses it to check what
# reached the PTY.
#   pane-has.sh [--not] HERDR SESSION PANE TEXT
set -uo pipefail
want=1 tries=40
[[ "$1" == "--not" ]] && { want=0 tries=10; shift; }
herdr="$1" session="$2" pane="$3" text="$4"
read_pane() { env -u HERDR_SOCKET_PATH "$herdr" --session "$session" pane read "$pane" --source recent 2>/dev/null; }
for _ in $(seq "$tries"); do
  if read_pane | grep -qF -- "$text"; then
    [[ $want == 1 ]] && exit 0
    echo "pane $pane shows '$text'" >&2; exit 1
  fi
  sleep 0.1
done
[[ $want == 0 ]] && exit 0
echo "pane $pane never showed '$text'; last screen:" >&2
read_pane | tail -5 >&2
exit 1
