#!/usr/bin/env bash
# Runs the end-to-end gap suites one after another (one dev instance at a
# time; each resets its instance when done). Output: .dev/e2e.out/<suite>/.
#   apps/macos/automation/e2e/run-all.sh [suite…]   (default: all)
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
suites=("${@:-adopt chords dyn animoff agents plan ended api git reload stab gallery motion dim fixes topbar tabdrag palette-motion drop}")
status=0
for s in ${suites[*]}; do
  printf '\n### %s\n' "$s"
  "$here/$s.sh" || status=1
done
exit "$status"
