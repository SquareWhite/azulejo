#!/bin/sh
# PLAN.md step 0.2: probe herdr's attach, events and layout behaviour.
# Runs in an isolated named session (default "azj-probe") and deletes it afterwards.
# It never touches your main herdr session. Findings: docs/herdr-notes.md.
#
#   scripts/herdr-probe.sh              # all checks, one JSON line per check
#   AZJ_PROBE_TUI=1 scripts/herdr-probe.sh   # also run the TUI client (connects to saved SSH machines)
#   AZJ_PROBE_OUT=/tmp/x scripts/herdr-probe.sh   # keep raw captures in /tmp/x
set -eu
command -v herdr >/dev/null || { echo "herdr not found on PATH" >&2; exit 1; }
exec python3 "$(cd "$(dirname "$0")" && pwd)/herdr-probe/probe.py" "$@"
