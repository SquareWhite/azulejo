#!/usr/bin/env bash
# Showcase captures for design work: the main features in every built-in
# theme, on fixture data only. Builds the mockup's scene (mockup.sh, on its
# own dev instance, AZJ_DEV_NAME=shw), adds a "sample-api" workspace whose
# terminals show an agent, git and a log tail (showcase/), then, per theme
# (shared/design/tokens.json), captures through showcase.azs.in:
#   hero                  the mockup scene: tiles, tab strip, sidebar, focused header
#   sidebar-rail          the sidebar collapsed to its rail
#   quick-open            ⌘K, empty query (what needs you first)
#   quick-open-query      ⌘K with "upl" typed
#   quick-open-commands   ⌘K with ">"
#   quick-open-jump       ⌘K with "@"
#   quick-open-theme      ⌘K "Set theme…", the theme list
#   workspace             the sample-api workspace, sidebar on Agents
#   sidebar-files         its Files view, a few folders open, git changes shown
#   smart-select          ⌘S: the Select panel, each kind with its count
#   smart-select-paths    ⌘S after `p`: the paths, labelled a, s, d…
# Every shell is bash with a bare environment and none of the user's profile,
# history or prompt (herdr's default_shell is a wrapper with a neutral
# "folder $" prompt), the fixtures live in /tmp/azj-<name> and the hosts are
# reserved example domains, so no capture shows anything of this Mac. Never
# changes the Mac's appearance.
#
#   apps/macos/automation/showcase.sh [--keep]   (--keep: leave the instance)
#
# Output: .dev/showcase/<theme>-<view>.png (2×, the window as screencapture
# takes it, else the app's own render of its layer tree), from the run's
# captures in .dev/<name>.out/mockup/.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-shw}"
keep=""
for a in "$@"; do
  case "$a" in
    --keep) keep="--keep" ;;
    *) echo "showcase.sh: unknown argument $a" >&2; exit 2 ;;
  esac
done
out="$root/.dev/$AZJ_DEV_NAME.out/mockup"
dest="$root/.dev/showcase"
# A short, fixed fixture folder: tile headers and ⌘K show its path.
work="/tmp/azj-$AZJ_DEV_NAME"
rm -rf "$work"
mkdir -p "$work"
work="$(cd "$work" && pwd -P)"
cleanup() {
  [[ -n "$keep" ]] || "$root/apps/macos/dev.sh" --reset >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

# Neutral shells: herdr starts this wrapper for every pane (its config's
# default_shell; the stand-in machines' too): bash with a bare environment,
# no profile or history, a fixture HOME and a "folder $" prompt. A program
# that renames itself (the mockup's `just`) shows its environment in ⌘K, so
# nothing of the user's may be in it.
mkdir -p "$work/home"
cat >"$work/shell" <<SH
#!/bin/sh
exec env -i HOME='$work/home' PATH=/usr/bin:/bin:/usr/sbin:/sbin TERM="\${TERM:-xterm-256color}" \\
  COLORTERM=truecolor LANG=en_GB.UTF-8 HISTFILE=/dev/null BASH_SILENCE_DEPRECATION_WARNING=1 \\
  PS1='\[\e[32m\]\W\[\e[0m\] \$ ' /bin/bash --noprofile --norc "\$@"
SH
chmod +x "$work/shell"
printf '[terminal]\ndefault_shell = "%s"\nshell_mode = "non_login"\n' "$work/shell" >"$work/herdr.toml"
export AZJ_HERDR_CONFIG="$work/herdr.toml" HERDR_CONFIG_PATH="$work/herdr.toml"

# Fixture projects: "azulejo" is the mockup's working folder, "sample-api"
# the ⌘S and Files workspace, with uncommitted changes for the tree's marks.
commit() {
  git -C "$1" -c init.defaultBranch="$2" init -q
  git -C "$1" add -A
  git -C "$1" -c user.email=dev@example.invalid -c user.name=dev -c commit.gpgsign=false commit -qm init
}
mkdir -p "$work/azulejo"
printf '# azulejo\n' >"$work/azulejo/README.md"
commit "$work/azulejo" main
api="$work/sample-api"
for f in README.md package.json tsconfig.json docs/api.md docs/upload.md \
  scripts/deploy.sh scripts/seed.ts src/server.ts src/config.ts \
  src/session/store.ts src/session/token.ts src/upload/client.ts \
  src/upload/retry.ts src/upload/index.ts tests/session/store.test.ts \
  tests/upload/client.test.ts logs/api.log logs/upload.err; do
  mkdir -p "$(dirname "$api/$f")"
  printf '// %s\n' "$f" >"$api/$f"
done
printf 'logs/\nnode_modules/\n' >"$api/.gitignore"
commit "$api" fix/upload-retry
printf 'export const retry = backoff(3);\n' >>"$api/src/upload/retry.ts"
printf 'await putChunk(url, body);\n' >>"$api/src/upload/client.ts"
printf 'test("retries a chunk", () => {});\n' >"$api/tests/upload/retry.test.ts"

mkdir -p "$work/screens"
cp "$here/showcase/agent.txt" "$here/showcase/git.txt" "$here/showcase/logs.txt" "$work/screens/"
cp "$here/showcase/show" "$work/show"
sed -e "s|@WORK@|$work|g" "$here/showcase/sample-api.toml.in" >"$work/sample-api.toml"

# The tail: the per-theme block once for each built-in theme.
python3 - "$here/showcase.azs.in" "$root/shared/design/tokens.json" "$work" >"$work/showcase.azs" <<'PY'
import json, sys
script, tokens, work = sys.argv[1:4]
themes = [t for t in json.load(open(tokens))["themes"] if not t.startswith("_")]
text = open(script).read().replace("@WORK@", work)
head, rest = text.split("@EACH_THEME@\n")
block, end = rest.split("@END_THEME@\n")
sys.stdout.write(head + "".join(block.replace("@THEME@", t) for t in themes) + end)
PY
themes="$(python3 -c 'import json,sys; print(" ".join(t for t in json.load(open(sys.argv[1]))["themes"] if not t.startswith("_")))' \
  "$root/shared/design/tokens.json")"

status=0
# TMPDIR: the mockup's own fixtures (its ADR preview's path shows in ⌘K).
TMPDIR="$work" AZJ_MOCKUP_TAIL="$work/showcase.azs" AZJ_MOCKUP_CWD="$work/azulejo" AZJ_MOCKUP_TIMEOUT=900 \
  "$here/mockup.sh" $keep || status=1

# Collect: the system capture when the harness got one, else the app's render.
rm -rf "$dest"
mkdir -p "$dest"
views="hero sidebar-rail quick-open quick-open-query quick-open-commands quick-open-jump quick-open-theme
  workspace sidebar-files smart-select smart-select-paths"
missing=0
for theme in $themes; do
  for view in $views; do
    name="$theme-$view"
    if [[ -s "$out/$name.png" ]]; then
      cp "$out/$name.png" "$dest/$name.png"
    elif [[ -s "$out/$name-app.png" ]]; then
      cp "$out/$name-app.png" "$dest/$name.png"
    else
      echo "showcase: no capture $name" >&2
      missing=$((missing + 1))
    fi
  done
done
[[ "$missing" == 0 ]] || status=1
echo "showcase: $(find "$dest" -name '*.png' | wc -l | tr -d ' ') captures in $dest"
exit "$status"
