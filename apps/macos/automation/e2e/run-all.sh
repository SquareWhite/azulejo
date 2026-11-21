#!/usr/bin/env bash
# Runs the end-to-end suites and prints how long each took. Each suite runs on
# its own dev instance and resets it when done. Output: .dev/e2e.out/<suite>/.
#
#   apps/macos/automation/e2e/run-all.sh [-j N] [suite…]   (default: all)
#
#   -j N        run up to N suites at once (default 1: one after another, in
#               the order given). With N > 1 the longest go first, and the
#               load-sensitive ones (QUIET below: frame-drop and timing
#               checks) run alone at the end. Each suite's output goes to
#               .dev/e2e.out/run-all/<suite>.log instead of the terminal.
#   E2E_TAG=x   give suite n the dev instance `e2e<x><nn>` (herdr session
#               azj-dev-e2e<x><nn>), so two checkouts running suites at once
#               don't share herdr sessions (their names are global). Keep it
#               short: the socket path has to fit macOS's limit, which a
#               .claude/worktrees/agent-* checkout nearly fills (a tag of up
#               to 3 characters keeps the name at 8).
#
# Without E2E_TAG each suite uses its default instance (`e2e-dyn`…), or, with
# AZJ_DEV_NAME exported, that one name for every suite (serial only).
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../../.." && pwd)"

# Every suite, longest first (serial wall times on a quiet machine, rounded;
# only the order matters).
ALL="contrast fixes mtabs wsslide wshints chords unread dim gallery rfiles agents topbar drop fops
  tabdrag reorder sbwidth resize wsnum rfops motion wssync hover ended rdrop adopt reload animoff git
  palette-motion api drag dyn"
# Frame-drop counts (dyn and drag: `anim expect-clean`), the ⌘K menu's
# 140–260 ms open timings (palette-motion) and api's 1 s herdr round trips
# go wrong under load: with -j > 1 they run alone, after the rest.
QUIET="dyn drag palette-motion api"
ALL="$(echo $ALL)"  # one line, single spaces: " $ALL " matches whole names

jobs=1
while [[ $# -gt 0 ]]; do
  case "$1" in
    -j) jobs="${2:-}"; shift 2 ;;
    -j*) jobs="${1#-j}"; shift ;;
    -h | --help) sed -n '2,21p' "$0"; exit 0 ;;
    --) shift; break ;;
    -*) echo "run-all: unknown option $1" >&2; exit 2 ;;
    *) break ;;
  esac
done
[[ "$jobs" =~ ^[1-9][0-9]*$ ]] || { echo "run-all: -j takes a number from 1" >&2; exit 2; }
if [[ $# -gt 0 ]]; then suites=("$@"); else read -r -a suites <<<"$(echo $ALL)"; fi
for s in "${suites[@]}"; do
  [[ -x "$here/$s.sh" ]] || { echo "run-all: no suite $s ($here/$s.sh)" >&2; exit 2; }
done

tag="${E2E_TAG:-}"
if [[ -n "$tag" ]]; then
  [[ "$tag" =~ ^[A-Za-z0-9]+$ ]] || { echo "run-all: E2E_TAG takes letters and digits" >&2; exit 2; }
elif (( jobs > 1 )) && [[ -n "${AZJ_DEV_NAME:-}" ]]; then
  echo "run-all: -j $jobs with AZJ_DEV_NAME=$AZJ_DEV_NAME would run every suite on one instance;" \
    "set E2E_TAG instead" >&2
  exit 2
fi

# The instance for suite number $1 (from 1), or empty for the suite's own.
name_for() {
  [[ -n "$tag" ]] && printf 'e2e%s%02d' "$tag" "$1"
}
# macOS caps socket paths at 103 bytes and paths just under it still fail:
# refuse up front rather than have a suite die at launch.
for i in "${!suites[@]}"; do
  n="$(name_for $((i + 1)))"
  [[ -n "$n" ]] || continue
  sock="$root/.dev/$n/run/azulejo.sock"
  if (( ${#sock} > 101 )); then
    echo "run-all: E2E_TAG=$tag makes a ${#sock}-byte socket path ($sock); use a shorter tag" >&2
    exit 2
  fi
done

now() { python3 -c 'import time; print(time.time())'; }
secs() { python3 -c "print(f'{$2 - $1:.1f}')"; }
logs="$root/.dev/e2e.out/run-all"
mkdir -p "$logs"
declare -a order names
for i in "${!suites[@]}"; do names[i]="$(name_for $((i + 1)))"; done
results=()   # "suite<TAB>seconds<TAB>PASS|FAIL"
start_all="$(now)"

# One suite in the foreground, output on the terminal.
run_here() {
  local i="$1" s="${suites[$1]}" t0 t1 st=PASS
  printf '\n### %s\n' "$s"
  t0="$(now)"
  if [[ -n "${names[i]}" ]]; then
    AZJ_DEV_NAME="${names[i]}" "$here/$s.sh" || st=FAIL
  else
    "$here/$s.sh" || st=FAIL
  fi
  t1="$(now)"
  results+=("$s	$(secs "$t0" "$t1")	$st")
}

if (( jobs == 1 )); then
  for i in "${!suites[@]}"; do run_here "$i"; done
else
  # Longest first: the order in ALL; suites ALL doesn't name go last.
  busy=() quiet=()
  for s in $ALL; do
    for i in "${!suites[@]}"; do
      [[ "${suites[i]}" == "$s" ]] || continue
      if [[ " $QUIET " == *" $s "* ]]; then quiet+=("$i"); else busy+=("$i"); fi
    done
  done
  for i in "${!suites[@]}"; do
    [[ " $ALL " == *" ${suites[i]} "* ]] || busy+=("$i")
  done
  echo "run-all: ${#busy[@]} suite(s), $jobs at a time; then ${#quiet[@]} alone (${QUIET// /, }). Logs: $logs/"
  declare -a pid_of t0_of
  running=()
  launch() {
    local i="$1" s="${suites[$1]}"
    t0_of[i]="$(now)"
    (
      if [[ -n "${names[i]}" ]]; then export AZJ_DEV_NAME="${names[i]}"; else unset AZJ_DEV_NAME; fi
      exec "$here/$s.sh"
    ) >"$logs/$s.log" 2>&1 &
    pid_of[i]=$!
    running+=("$i")
  }
  reap() { # wait until a running suite ends; record it
    local i still
    while true; do
      still=()
      for i in "${running[@]}"; do
        if kill -0 "${pid_of[i]}" 2>/dev/null; then
          still+=("$i")
        else
          local st=PASS t1
          wait "${pid_of[i]}" || st=FAIL
          t1="$(now)"
          results+=("${suites[i]}	$(secs "${t0_of[i]}" "$t1")	$st")
          printf '%-4s %-16s %6ss\n' "$st" "${suites[i]}" "$(secs "${t0_of[i]}" "$t1")"
        fi
      done
      if (( ${#still[@]} < ${#running[@]} )); then
        running=("${still[@]+"${still[@]}"}")
        return
      fi
      sleep 0.5
    done
  }
  for i in "${busy[@]}"; do
    while (( ${#running[@]} >= jobs )); do reap; done
    launch "$i"
  done
  while (( ${#running[@]} > 0 )); do reap; done
  # The quiet lane: one at a time, nothing else running.
  for i in "${quiet[@]+"${quiet[@]}"}"; do
    launch "$i"
    while (( ${#running[@]} > 0 )); do reap; done
  done
fi

total="$(secs "$start_all" "$(now)")"
status=0
printf '\n### summary (-j %s)\n' "$jobs"
for r in "${results[@]}"; do
  IFS=$'\t' read -r s t st <<<"$r"
  printf '%-4s %-16s %6ss\n' "$st" "$s" "$t"
  [[ "$st" == PASS ]] || status=1
done
printf '%s suite(s) in %ss\n' "${#results[@]}" "$total"
exit "$status"
