#!/usr/bin/env bash
# Tests for scripts/release/dogfood.sh. Everything happens in temp dirs with
# tiny ad-hoc signed stand-in apps: /Applications and a running Azulejo are
# never touched, and nothing is built.
#
#   scripts/release/test-dogfood.sh
#   AZJ_TEST_DOGFOOD_BUILD=1 scripts/release/test-dogfood.sh   # also run two real builds (slow)
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
dogfood="$root/scripts/release/dogfood.sh"
tmp="$(mktemp -d)"
dummy_pids=()
cleanup() {
  for p in "${dummy_pids[@]:-}"; do [[ -n "$p" ]] && kill -KILL "$p" 2>/dev/null || true; done
  rm -rf "$tmp"
}
trap cleanup EXIT

export AZJ_DOGFOOD_APPS_DIR="$tmp/apps"
export AZJ_DOGFOOD_BUILDS_DIR="$tmp/builds"
export AZJ_DOGFOOD_SRC="$tmp/src"
export AZJ_DOGFOOD_QUIT_TIMEOUT=2
mkdir -p "$AZJ_DOGFOOD_APPS_DIR" "$AZJ_DOGFOOD_BUILDS_DIR"

pass=0 fail=0
ok() { pass=$((pass + 1)); printf 'ok   %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf 'FAIL %s\n' "$1"; }
check() { if eval "$2"; then ok "$1"; else bad "$1"; fi; }

# A signed stand-in bundle whose "Azulejo" is a shell script.
# behaviour: exit (on TERM) | ignore (TERM ignored)
make_app() { # make_app <dir> <commit> <behaviour>
  local app="$1/Azulejo.app" commit="$2" behaviour="${3:-exit}"
  mkdir -p "$app/Contents/MacOS"
  if [[ "$behaviour" == ignore ]]; then
    printf '#!/bin/bash\ntrap "" TERM\nwhile :; do sleep 0.1; done\n' > "$app/Contents/MacOS/Azulejo"
  else
    printf '#!/bin/bash\ntrap "exit 0" TERM\nwhile :; do sleep 0.1; done\n' > "$app/Contents/MacOS/Azulejo"
  fi
  chmod +x "$app/Contents/MacOS/Azulejo"
  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>Azulejo</string>
<key>CFBundleIdentifier</key><string>dev.azulejo.test.dogfood</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
$( [[ -n "$commit" ]] && printf '<key>AzulejoCommit</key><string>%s</string><key>AzulejoDescribe</key><string>%s</string>' "$commit" "${commit:0:7}" )
</dict></plist>
PLIST
  codesign --force --sign - "$app" 2>/dev/null
}

# Put a fake build into the kept builds, as `just dogfood` would.
fake_build() { # fake_build <id> <commit>
  local d="$AZJ_DOGFOOD_BUILDS_DIR/$1"
  mkdir -p "$d"
  make_app "$d" "$2" exit
  printf 'commit=%s\ndescribe=%s\nversion=0.1.0 (1)\nkept=2026-11-03 12:00\nsource=test\n' "$2" "${2:0:7}" > "$d/meta"
}

installed_commit() {
  plutil -extract AzulejoCommit raw -o - "$AZJ_DOGFOOD_APPS_DIR/Azulejo.app/Contents/Info.plist" 2>/dev/null || echo none
}

start_installed() { # run the installed stand-in, like a running Azulejo; sets $pid
  "$AZJ_DOGFOOD_APPS_DIR/Azulejo.app/Contents/MacOS/Azulejo" >/dev/null 2>&1 &
  pid=$!
  dummy_pids+=("$pid")
  sleep 0.3
}

# --- list on an empty dir
out="$("$dogfood" --list)"
check "list with no builds" '[[ "$out" == *"no kept builds"* ]]'

# --- first install over a hand-installed app (no AzulejoCommit): it gets kept
make_app "$AZJ_DOGFOOD_APPS_DIR" "" exit
touch -t 202610310900 "$AZJ_DOGFOOD_APPS_DIR/Azulejo.app"   # installed by hand before any dogfood build
fake_build 20261101-100000-aaaaaaaaa aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
fake_build 20261102-100000-bbbbbbbbb bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
"$dogfood" --to bbbb --no-launch >/dev/null
check "--to installs the matching build" '[[ "$(installed_commit)" == bbbb* ]]'
check "current marker set" '[[ "$(cat "$AZJ_DOGFOOD_BUILDS_DIR/current")" == 20261102-100000-bbbbbbbbb ]]'
check "hand-installed app kept, dated by its install time" '[[ -d "$AZJ_DOGFOOD_BUILDS_DIR/20261031-090000-manual" ]]'
check "no leftover .new/.old bundles" '! ls "$AZJ_DOGFOOD_APPS_DIR" | grep -qE "\.(new|old)"'

# --- rollback goes to the next older kept build
"$dogfood" --rollback --no-launch >/dev/null
check "--rollback installs the previous build" '[[ "$(installed_commit)" == aaaa* ]]'

# --- rolling back again reaches the hand-installed app, then stops
"$dogfood" --rollback --no-launch >/dev/null
check "second --rollback reaches the hand-installed app" '[[ "$(installed_commit)" == none && "$(cat "$AZJ_DOGFOOD_BUILDS_DIR/current")" == 20261031-090000-manual ]]'
if "$dogfood" --rollback --no-launch >/dev/null 2>&1; then bad "--rollback past the oldest fails"; else ok "--rollback past the oldest fails"; fi
"$dogfood" --to aaaa --no-launch >/dev/null
check "hand-installed app not kept twice" '[[ $(ls "$AZJ_DOGFOOD_BUILDS_DIR" | grep -c -- "-manual$") -eq 1 ]]'

# --- list marks the installed one
out="$("$dogfood" --list)"
check "--list marks the installed build" 'printf "%s\n" "$out" | grep -q "^\* 20261101-100000-aaaaaaaaa"'

# --- quitting: a running app that exits on TERM is quit, then replaced
start_installed
"$dogfood" --to bbbb --no-launch >/dev/null
check "running app quit gracefully" '! kill -0 "$pid" 2>/dev/null'
check "replaced after quit" '[[ "$(installed_commit)" == bbbb* ]]'

# --- a stand-in that ignores TERM, a bystander with the same name elsewhere
fake_build 20261103-100000-ccccccccc cccccccccccccccccccccccccccccccccccccccc
rm -rf "$AZJ_DOGFOOD_APPS_DIR/Azulejo.app"
make_app "$AZJ_DOGFOOD_APPS_DIR" bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb ignore
mkdir -p "$tmp/elsewhere"
make_app "$tmp/elsewhere" zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz exit
"$tmp/elsewhere/Azulejo.app/Contents/MacOS/Azulejo" >/dev/null 2>&1 &
bystander=$!; dummy_pids+=("$bystander")
start_installed
if "$dogfood" --to cccc --no-launch >/dev/null 2>"$tmp/err"; then rc=0; else rc=$?; fi
check "stuck app: refuses after the timeout" '[[ $rc -ne 0 ]] && grep -q "didn.t quit" "$tmp/err"'
check "stuck app: still running, nothing changed" 'kill -0 "$pid" 2>/dev/null && [[ "$(installed_commit)" == bbbb* ]]'
check "stuck app: no half-installed .new bundle left" '[[ ! -d "$AZJ_DOGFOOD_APPS_DIR/Azulejo.app.new" ]]'
"$dogfood" --to cccc --no-launch --force-kill >/dev/null
check "--force-kill: killed and replaced" '! kill -0 "$pid" 2>/dev/null && [[ "$(installed_commit)" == cccc* ]]'
check "bystander Azulejo in another folder untouched" 'kill -0 "$bystander" 2>/dev/null'

# --- errors
if "$dogfood" --to deadbeef --no-launch >/dev/null 2>&1; then bad "--to unknown hash fails"; else ok "--to unknown hash fails"; fi
if "$dogfood" --bogus >/dev/null 2>&1; then bad "unknown option fails"; else ok "unknown option fails"; fi

# --- pruning keeps AZJ_DOGFOOD_KEEP builds plus the installed one
if [[ "${AZJ_TEST_DOGFOOD_BUILD:-0}" == 1 ]]; then
  # Two real builds from HEAD in a worktree, installed into the temp apps dir.
  AZJ_DOGFOOD_KEEP=2 "$dogfood" --no-launch
  first="$(cat "$AZJ_DOGFOOD_BUILDS_DIR/current")"
  check "real build installed and signed" 'codesign --verify --deep --strict "$AZJ_DOGFOOD_APPS_DIR/Azulejo.app"'
  check "real build records its commit" '[[ "$(installed_commit)" == "$(git -C "$root" rev-parse HEAD)" ]]'
  AZJ_DOGFOOD_KEEP=2 "$dogfood" --no-launch
  second="$(cat "$AZJ_DOGFOOD_BUILDS_DIR/current")"
  check "second build is a new kept build" '[[ "$first" != "$second" && -d "$AZJ_DOGFOOD_BUILDS_DIR/$first" ]]'
  check "pruned to KEEP + current" '[[ $(ls -d "$AZJ_DOGFOOD_BUILDS_DIR"/*/ | wc -l) -eq 3 ]]'
  "$dogfood" --rollback --no-launch >/dev/null
  check "rollback after real builds" '[[ "$(cat "$AZJ_DOGFOOD_BUILDS_DIR/current")" == "$first" ]]'
  git -C "$root" worktree remove --force "$AZJ_DOGFOOD_SRC" >/dev/null 2>&1 || true
fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
(( fail == 0 ))
