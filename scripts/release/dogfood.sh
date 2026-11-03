#!/usr/bin/env bash
# Build Azulejo from a clean commit and swap it into /Applications, keeping the
# last few builds for rollback (docs/dogfooding.md).
#
#   just dogfood                  # build HEAD, quit Azulejo, install, relaunch
#   just dogfood --check          # run just build/test/lint on that commit first
#   just dogfood --dirty          # build the working tree as it is, uncommitted changes included
#   just dogfood --rollback       # reinstall the build before the current one
#   just dogfood --to <hash>      # reinstall a kept build by commit
#   just dogfood --list           # show kept builds
#   add --no-launch to any install to skip the relaunch
#
# By default the build runs in a separate git worktree checked out at HEAD
# (AZJ_DOGFOOD_SRC), so uncommitted work in this checkout, including other
# agents' work in progress, never ends up in the installed app. That worktree
# is kept between runs so its cargo and Swift caches make rebuilds quick.
#
# Overrides (tests use these to stay away from the real app):
#   AZJ_DOGFOOD_APPS_DIR    where Azulejo.app is installed     (/Applications)
#   AZJ_DOGFOOD_BUILDS_DIR  where kept builds live             (~/Library/Application Support/Azulejo/dogfood-builds)
#   AZJ_DOGFOOD_SRC         build worktree                     (~/Library/Caches/Azulejo/dogfood-src)
#   AZJ_DOGFOOD_KEEP        builds to keep besides the current (3)
#   AZJ_DOGFOOD_QUIT_TIMEOUT seconds to wait for Azulejo to quit (10)
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
apps_dir="${AZJ_DOGFOOD_APPS_DIR:-/Applications}"
builds_dir="${AZJ_DOGFOOD_BUILDS_DIR:-$HOME/Library/Application Support/Azulejo/dogfood-builds}"
src="${AZJ_DOGFOOD_SRC:-$HOME/Library/Caches/Azulejo/dogfood-src}"
keep="${AZJ_DOGFOOD_KEEP:-3}"
quit_timeout="${AZJ_DOGFOOD_QUIT_TIMEOUT:-10}"
app="$apps_dir/Azulejo.app"
exe="$app/Contents/MacOS/Azulejo"

say() { printf '==> %s\n' "$*"; }
die() { printf 'dogfood: %s\n' "$*" >&2; exit 1; }

usage() {
  sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

mode=build dirty=0 check=0 launch=1 force_kill=0 target=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dirty) dirty=1 ;;
    --check) check=1 ;;
    --no-launch) launch=0 ;;
    --force-kill) force_kill=1 ;;
    --rollback) mode=rollback ;;
    --list) mode=list ;;
    --to) mode=to; target="${2:-}"; [[ -n "$target" ]] || die "--to needs a commit hash"; shift ;;
    -h|--help) usage 0 ;;
    *) printf 'dogfood: unknown option %s\n\n' "$1" >&2; usage 2 ;;
  esac
  shift
done

# --- kept builds ------------------------------------------------------------
# Each build is $builds_dir/<YYYYmmdd-HHMMSS>-<short>/Azulejo.app plus a
# `meta` file; `$builds_dir/current` names the installed one.

plist_get() { plutil -extract "$2" raw -o - "$1/Contents/Info.plist" 2>/dev/null || echo unknown; }

build_ids() { # oldest first
  [[ -d "$builds_dir" ]] || return 0
  find "$builds_dir" -mindepth 1 -maxdepth 1 -type d -exec basename {} \; | sort
}

current_id() { cat "$builds_dir/current" 2>/dev/null || true; }

meta() { sed -n "s/^$2=//p" "$builds_dir/$1/meta" 2>/dev/null | head -1; }

keep_build() { # keep_build <app-path> <source-label> [stamp] -> prints the new id
  # Ids sort by time, which is what --rollback walks back through. An app
  # that was already installed passes its install time so it sorts before
  # the build replacing it.
  local from="$1" label="$2" stamp="${3:-}" commit short id
  commit="$(plist_get "$from" AzulejoCommit)"
  short="${commit:0:9}"
  if [[ "$commit" == unknown ]]; then short=manual; fi
  [[ -n "$stamp" ]] || stamp="$(date +%Y%m%d-%H%M%S)"
  id="$stamp-$short"
  while [[ -e "$builds_dir/$id" ]]; do sleep 1; id="$(date +%Y%m%d-%H%M%S)-$short"; done
  mkdir -p "$builds_dir/$id"
  ditto "$from" "$builds_dir/$id/Azulejo.app"
  {
    echo "commit=$commit"
    echo "describe=$(plist_get "$from" AzulejoDescribe)"
    echo "version=$(plist_get "$from" CFBundleShortVersionString) ($(plist_get "$from" CFBundleVersion))"
    echo "kept=$(date '+%Y-%m-%d %H:%M')"
    echo "source=$label"
  } > "$builds_dir/$id/meta"
  echo "$id"
}

prune() {
  local cur ids n
  cur="$(current_id)"
  ids="$(build_ids | grep -vx "$cur" || true)"
  n="$(printf '%s\n' "$ids" | grep -c . || true)"
  if (( n > keep )); then
    printf '%s\n' "$ids" | head -n "$((n - keep))" | while read -r old; do
      if [[ -n "$old" ]]; then rm -rf "${builds_dir:?}/$old"; say "dropped old build $old"; fi
    done
  fi
}

list() {
  local cur ids
  cur="$(current_id)"
  ids="$(build_ids)"
  if [[ -z "$ids" ]]; then echo "no kept builds in $builds_dir"; return; fi
  printf '%s\n' "$ids" | while read -r id; do
    local mark=" "
    if [[ "$id" == "$cur" ]]; then mark="*"; fi
    printf '%s %-26s %-28s %-18s %s\n' "$mark" "$id" "$(meta "$id" describe)" "$(meta "$id" version)" "$(meta "$id" kept)"
  done
  echo "(* = installed; roll back with: just dogfood --rollback, or --to <hash>)"
}

# --- quitting and installing -------------------------------------------------

running_pids() { # processes running $exe, directly or as a script argument
  # exe goes in through the environment: on the command line, awk would match itself.
  ps -axo pid=,command= | AZJ_EXE="$exe" awk '
    BEGIN { exe = ENVIRON["AZJ_EXE"] }
    { pid=$1; $1=""; cmd=substr($0, 2)
      if (cmd == exe || index(cmd, exe " ") == 1 || index(cmd, " " exe) > 0) print pid }'
}

quit_running() { # returns 0 if Azulejo was running (and has now quit)
  local pids waited=0
  pids="$(running_pids)"
  [[ -n "$pids" ]] || return 1
  say "quitting Azulejo (SIGTERM; it saves state, herdr keeps your sessions)"
  # shellcheck disable=SC2086
  kill -TERM $pids 2>/dev/null || true
  while [[ -n "$(running_pids)" ]]; do
    if (( waited >= quit_timeout * 10 )); then
      if (( force_kill )); then
        say "Azulejo didn't quit in ${quit_timeout}s: sending SIGKILL (--force-kill); unsaved layout changes since the last save are lost"
        # shellcheck disable=SC2046
        kill -KILL $(running_pids) 2>/dev/null || true
        sleep 0.5
        break
      fi
      rm -rf "$app.new"
      die "Azulejo didn't quit within ${quit_timeout}s. Quit it yourself and re-run, or pass --force-kill. Nothing was changed."
    fi
    sleep 0.1
    waited=$((waited + 1))
  done
  return 0
}

install_from() { # install_from <kept-id>
  local id="$1" from="$builds_dir/$1/Azulejo.app" was_running=0 old_desc
  [[ -d "$from" ]] || die "kept build $id is missing"
  [[ -d "$apps_dir" ]] || die "$apps_dir doesn't exist"
  [[ -w "$apps_dir" ]] || die "can't write to $apps_dir (try an admin account, or set AZJ_DOGFOOD_APPS_DIR)"

  old_desc="none"
  if [[ -d "$app" ]]; then
    old_desc="$(plist_get "$app" AzulejoDescribe)"
    # An app installed by hand (the .dmg) isn't in the kept builds yet:
    # keep it so --rollback can get back to it.
    local old_commit known=0 k cur
    old_commit="$(plist_get "$app" AzulejoCommit)"
    cur="$(current_id)"
    if [[ -n "$cur" && -d "$builds_dir/$cur" && "$(meta "$cur" commit)" == "$old_commit" ]]; then known=1; fi
    for k in $(build_ids); do
      if [[ "$old_commit" != unknown && "$(meta "$k" commit)" == "$old_commit" ]]; then known=1; fi
    done
    if (( ! known )); then
      local kept_id
      kept_id="$(keep_build "$app" installed "$(stat -f %Sm -t %Y%m%d-%H%M%S "$app")")"
      say "kept the previously installed app as $kept_id"
    fi
  fi

  rm -rf "$app.new"
  ditto "$from" "$app.new"
  xattr -dr com.apple.quarantine "$app.new" 2>/dev/null || true
  codesign --verify --deep --strict "$app.new" || die "signature check failed for $id; nothing was changed"

  if quit_running; then was_running=1; fi
  if [[ -d "$app" ]]; then mv "$app" "$app.old.$$"; fi
  mv "$app.new" "$app"
  rm -rf "$app.old.$$"
  echo "$id" > "$builds_dir/current"

  say "installed $(meta "$id" describe) ($(meta "$id" version)): $old_desc → $(meta "$id" describe)"
  say "roll back with: just dogfood --rollback"

  if (( launch )) && [[ -t 1 ]]; then
    open -a "$app"
    say "relaunched Azulejo"
  elif (( was_running )); then
    say "Azulejo was quit and not relaunched (--no-launch or not a terminal): open it when ready"
  fi
}

# --- building ----------------------------------------------------------------

prepare_worktree() { # check out HEAD in $src and wire up the Ghostty build
  local commit="$1" pin have
  if [[ ! -e "$src/.git" ]]; then
    mkdir -p "$(dirname "$src")"
    git -C "$root" worktree add --detach "$src" "$commit" >/dev/null
  else
    git -C "$src" checkout --detach --force -q "$commit"
    git -C "$src" clean -fdq   # untracked only; ignored build caches stay
  fi
  # vendor/ghostty is a submodule. Reuse this checkout's copy (with its built
  # GhosttyKit and resources) when the pin matches, instead of rebuilding it.
  pin="$(git -C "$root" ls-tree "$commit" vendor/ghostty | awk '{print $3}')"
  have="$(git -C "$root/vendor/ghostty" rev-parse HEAD 2>/dev/null || true)"
  [[ -n "$pin" && "$pin" == "$have" ]] \
    || die "vendor/ghostty in this checkout is at ${have:-nothing}, but $commit pins $pin: run 'git submodule update' and 'just ghostty' here first"
  if [[ -L "$src/vendor/ghostty" ]]; then
    rm "$src/vendor/ghostty"
  elif [[ -d "$src/vendor/ghostty" ]]; then
    rmdir "$src/vendor/ghostty" 2>/dev/null || die "$src/vendor/ghostty isn't empty; remove it and re-run"
  fi
  ln -s "$root/vendor/ghostty" "$src/vendor/ghostty"
}

build() {
  local tree commit
  if (( dirty )); then
    tree="$root"
    say "building the working tree as it is (--dirty), uncommitted changes included"
  else
    commit="$(git -C "$root" rev-parse HEAD)"
    local changed
    changed="$(git -C "$root" status --porcelain --untracked-files=no | wc -l | tr -d ' ')"
    if (( changed > 0 )); then
      say "note: $changed uncommitted file(s) in this checkout are NOT included; building HEAD $(git -C "$root" rev-parse --short HEAD) (use --dirty to include them)"
    fi
    prepare_worktree "$commit"
    tree="$src"
    say "building $(git -C "$src" describe --tags --always) in $src"
  fi
  if (( check )); then
    say "running just build, test and lint first (--check)"
    (cd "$tree" && just build && just test && just lint)
  fi
  (cd "$tree" && AZJ_RELEASE_SKIP_DMG=1 scripts/release/build-mac.sh)
  mkdir -p "$builds_dir"
  local label=worktree
  if (( dirty )); then label=dirty; fi
  built_id="$(keep_build "$tree/target/release-mac/Azulejo.app" "$label")"
}

case "$mode" in
  list) list ;;
  build)
    built_id=""
    build
    install_from "$built_id"
    prune
    ;;
  rollback)
    cur="$(current_id)"
    [[ -n "$cur" ]] || die "nothing to roll back to: no build installed by just dogfood yet"
    prev="$(build_ids | awk -v cur="$cur" '$0 < cur' | tail -1)"
    [[ -n "$prev" ]] || die "no build older than $cur is kept (just dogfood --list)"
    install_from "$prev"
    ;;
  to)
    match="$(build_ids | while read -r id; do
      if [[ "$(meta "$id" commit)" == "$target"* ]]; then echo "$id"; fi; done | tail -1)"
    [[ -n "$match" ]] || die "no kept build for commit $target (just dogfood --list)"
    install_from "$match"
    ;;
esac
