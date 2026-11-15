#!/usr/bin/env bash
# Theme contrast, end to end (docs/themes.md, "Contrast"): the mockup's scene
# (mockup.sh's fixtures) plus a "colours" workspace (a terminal printing all
# 16 ANSI colours with bold, dim and a selection, a files tile on a git repo
# with every status, a dimmed terminal), shown in every built-in theme on its
# own dev instance (AZJ_DEV_NAME=e2e-ctr). Never changes the Mac's appearance.
#
#   apps/macos/automation/e2e/contrast.sh [--keep] [--themes "id id…"]
#
# Checks: in each theme, `theme.colors` (every token as the app resolves it,
# system colours and the user's accent included, the terminal palette and
# the focus dim) goes through scripts/theme-contrast with the pairs in
# shared/design/contrast.json, with the ⌘K menu's backdrop and, for themes
# with a vibrant sidebar, the sidebar card sampled from the window capture.
# Writes per-surface contact sheets (every theme side by side) for a look:
# .dev/e2e.out/contrast/sheets/. Captures: .dev/<name>.out/mockup/.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
auto="$(cd "$here/.." && pwd)"
root="$(cd "$here/../../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-ctr}"
case "$AZJ_DEV_NAME" in e2e*) ;; *) echo "contrast: AZJ_DEV_NAME must start with e2e" >&2; exit 2 ;; esac
keep="" themes="atom-one-dark atom-one-light macos-dark macos-light"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --keep) keep="--keep" ;;
    --themes) themes="$2"; shift ;;
    *) echo "contrast.sh: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
shots="$root/.dev/$AZJ_DEV_NAME.out/mockup"
out="$root/.dev/e2e.out/contrast"
rm -rf "$out"
mkdir -p "$out"
ctr="$(mktemp -d "${TMPDIR:-/tmp}/azj-ctr.XXXXXX")"
ctr="$(cd "$ctr" && pwd -P)"
tail="$ctr/tail.azs"
trap 'rm -rf "$ctr"' EXIT

# The colours workspace: an ANSI test card, a git repo with every status.
cat >"$ctr/colours.sh" <<'SH'
printf '\033[2J\033[H'
printf 'normal text  \033[1mbold\033[0m  \033[2mdim\033[0m  \033[3mitalic\033[0m  \033[4munderline\033[0m  \033[7minverse\033[0m\n'
printf 'the quick brown fox jumps over the lazy dog: a selection goes here\n'
printf 'selected red \033[31merror: something failed\033[0m and blue \033[34m./src/main.rs\033[0m\n'
names="black red green yellow blue magenta cyan white"
i=0; for n in $names; do printf '\033[3%dm%-9s\033[0m' $i $n; i=$((i+1)); done; printf '\n'
i=0; for n in $names; do printf '\033[9%dm%-9s\033[0m' $i "br-$n"; i=$((i+1)); done; printf '\n'
i=0; for n in $names; do printf '\033[1;3%dm%-9s\033[0m' $i $n; i=$((i+1)); done; printf '  bold\n'
i=0; for n in $names; do printf '\033[2;3%dm%-9s\033[0m' $i $n; i=$((i+1)); done; printf '  dim\n'
printf '\n\033[1mdiff --git a/src/main.rs b/src/main.rs\033[0m\n'
printf '\033[36m@@ -1,4 +1,5 @@\033[0m\n'
printf '\033[31m-    println!("old");\033[0m\n\033[32m+    println!("new");\033[0m\n'
printf '\033[90m# a comment in bright black, as shells show suggestions\033[0m\n'
printf '\033[33mwarning:\033[0m unused variable  \033[35mnote:\033[0m see https://example.invalid/docs\n'
printf '\033[1;34mdirectory/\033[0m  \033[32mscript.sh\033[0m  \033[36mlink\033[0m  file.txt\n'
exec cat >/dev/null
SH
mkdir -p "$ctr/repo"
(
  cd "$ctr/repo"
  git -c init.defaultBranch=main init -q
  printf '# fixture\n' >README.md
  printf 'one\n' >modified.txt
  printf 'two\n' >deleted.txt
  printf 'three\n' >renamed-old.txt
  printf 'build/\n*.log\n' >.gitignore
  git add -A
  git -c user.email=e2e@example.invalid -c user.name=e2e -c commit.gpgsign=false commit -qm init
  printf 'changed\n' >>modified.txt
  rm deleted.txt
  git mv renamed-old.txt renamed.txt
  printf 'new\n' >added.txt && git add added.txt
  printf 'loose\n' >untracked.txt
  mkdir -p build src && printf 'out\n' >build/output.bin && printf 'log\n' >debug.log
  printf 'fn main() {}\n' >src/main.rs
  printf 'secret\n' >.hidden
)
cat >"$ctr/colours.toml" <<TOML
name = "colours"

[[tab]]
name = "colours"
[tab.root]
split = "horizontal"
sizes = [58, 42]
[[tab.root.children]]
title = "ansi"
cwd = "$ctr"
command = "sh $ctr/colours.sh"
focus = true
[[tab.root.children]]
split = "vertical"
sizes = [55, 45]
children = [
  { root = "$ctr/repo" },
  { title = "shell", cwd = "$ctr", command = "sh $ctr/colours.sh" },
]
TOML

# The tail: the setup, then the THEME block once per theme.
python3 - "$here/contrast.azs.in" "$ctr" "$themes" >"$tail" <<'PY'
import sys
text, ctr, themes = open(sys.argv[1]).read(), sys.argv[2], sys.argv[3].split()
head, rest = text.split("#THEME\n", 1)
block, end = rest.split("#END\n", 1)
out = head + "".join(block.replace("@THEME@", t) for t in themes) + end
sys.stdout.write(out.replace("@CTR@", ctr))
PY

status=0
AZJ_MOCKUP_TAIL="$tail" AZJ_MOCKUP_TIMEOUT=900 AZJ_MOCKUP_CWD="$ctr" "$auto/mockup.sh" $keep || status=1
echo "contrast: automation $(cat "$shots/result" 2>/dev/null || echo 'no result')"

python3 "$here/contrast-check.py" "$root" "$shots" "$out" $themes || status=1
exit "$status"
