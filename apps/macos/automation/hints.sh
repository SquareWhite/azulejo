#!/usr/bin/env bash
# Visible-terminal hints: the core's recognition checks (cargo) plus real key
# routing, frozen selection, private-pasteboard copy, open dispatch and command
# execution in the app.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-hints}"
case "$AZJ_DEV_NAME" in
  e2e*) ;;
  *) echo "hints: use an e2e-prefixed dev name" >&2; exit 2 ;;
esac
fixtures="$(mktemp -d /tmp/azj-hints.XXXXXX)"
fixtures="$(cd "$fixtures" && pwd -P)"
out="$root/.dev/e2e.out/hints-$AZJ_DEV_NAME"
cleanup() {
  kill "${watchdog:-}" 2>/dev/null || true
  "$root/apps/macos/dev.sh" --reset >/dev/null
  rm -rf "$fixtures"
}
trap cleanup EXIT
"$root/apps/macos/dev.sh" --reset >/dev/null
# Run's new panes start the login shell. With fish installed, make that fish
# (without the user's config or history) so a multi-line Run proves it never
# relies on POSIX syntax in the login shell, and Run into an idle fish proves
# the line clearing works there.
fish_login=""
if fish="$(command -v fish)"; then
  fish_login="$fixtures/fish-login"
  printf '#!/bin/sh\nexec %s --no-config --private "$@"\n' "$fish" > "$fish_login"
  chmod +x "$fish_login"
  export SHELL="$fish_login"
fi
config="$root/.dev/$AZJ_DEV_NAME/config/azulejo/config.toml"
mkdir -p "$(dirname "$config")"
# Run reuses idle shells by default; the scenarios before "Run reuses idle
# shells" check the new-pane path, so they start with `run_target = "new"`.
printf '[herdr]\nsession = "%s"\n[keys.macos.terminal]\nselect_visible_item = "cmd+s"\n[hints]\nrun_target = "new"\n' \
  "${AZJ_DEV_SESSION:-azj-dev-$AZJ_DEV_NAME}" > "$config"
mkdir -p "$fixtures/src" "$out"
touch "$fixtures/src/example.swift" "$fixtures/src/setup.sh" "$fixtures/two words.txt"
printf '%s\n' 'https://example.invalid/first' '界👩‍💻é ./src/example.swift:12:3' '* feature/hints' '  feature/other' '$ printf hint-ran > ran.txt' '"two words.txt"' > "$fixtures/screen.txt"
printf '%s\n' '!printf agent-ran > agent-ran.txt' > "$fixtures/agent-screen.txt"
# Three commands with paths between them, for the arrow keys while choosing.
printf '%s\n' '$ printf focus-one > one.txt' ' M src/example.swift' '$ printf focus-two > two.txt' 'src/setup.sh: ok' '$ printf focus-three > three.txt' > "$fixtures/focus.txt"
# Detection, selection and callout placement live in the Rust core
# (core/crates/hints): its unit checks and the corpus scorer run first.
(cd "$root" && cargo test -q -p azulejo-hints --test detector --test select --test corpus -- --nocapture)
# Substitute fixture paths without embedding workstation paths in source.
python3 - "$here/hints.azs.in" "$fixtures/hints.azs" "$fixtures" "$fish_login" <<'PY'
import pathlib, sys
source, output, fixture, fish_login = sys.argv[1:]
wrapped = 'https://example.invalid/' + 'path/' * 30 + 'end'
pathlib.Path(fixture, 'wrapped.txt').write_text(wrapped + '\n')
# Lines for `l`: an indent and trailing blanks to trim, a blank row to skip,
# a repeated line (its URL gets one label, each line its own) and one long
# enough to soft-wrap, which stays one line.
long_line = 'wrap:' + 'abcdefghij' * 30
pathlib.Path(fixture, 'lines.txt').write_text('\n'.join([
    '  alpha line with indent   ', '', 'beta https://example.invalid/beta', 'beta https://example.invalid/beta',
    long_line, 'gamma', '']))
# A coding agent's footer (an update notice, an input box, a status line, an
# editor mode line) over a two-line prompt with nothing typed: only the
# status line's path and the prompt's path are items.
rule = '─' * 60
pathlib.Path(fixture, 'footer.txt').write_text('\n'.join([
    '✻ Worked for 1m 12s · done 9:41 AM · 1 shell still running',
    ' ' * 20 + '\x1b[33mUpdate available! Run: brew upgrade example-tool@latest\x1b[0m',
    rule, '❯ tidy up the parser and ship it', rule,
    f'  feature/demo * | {fixture}/src',
    '-- INSERT -- ⏵⏵ auto mode on · 1 shell · ← for agents',
    f'╭─ {fixture}/src (feature/demo|✔)', '╰─> ']))
# A screen full of lines (more than 26, some blank, some indented) for `l`:
# every non-blank row gets a label.
pathlib.Path(fixture, 'full.txt').write_text('\n'.join(
    ('' if i % 7 == 0 else f'{"  " if i % 5 == 0 else ""}row {i} of the full screen') for i in range(1, 201)))
# A coding agent's multi-line tool call: Run must not type it line by line
# into the login shell (fish has no heredocs).
pathlib.Path(fixture, 'shell-probe.txt').write_text('$ printf %s "$FISH_VERSION" > shell.txt\n')
pathlib.Path(fixture, 'tool-heredoc.txt').write_text(
    "\u25cf Bash(cat > heredoc-ran.txt <<'EOF'\n      heredoc-ran\n      EOF\n      )\n  \u23bf  (No content)\n")
# Dense screens for callout placement (core/crates/hints/tests/select.rs has the same).
pathlib.Path(fixture, 'tool-call.txt').write_text('\n'.join([
    '\u25cf Bash(gh run watch 4321 --interval 30 --exit-status > /dev/null 2>&1; echo "exit=$?"; gh r\u2026)',
    '  \u23bf  $ gh run watch 4321 --interval 30 --exit-status > /dev/null 2>&1; echo "exit=$?"; gh',
    '     run view 4321 --json conclusion,jobs -q',
    "     '{c:.conclusion,jobs:[.jobs[]|{n:.name,c:.conclusion}]}' (4m 29s)",
    '     (ctrl+b to run in background)',
    '',
    "\u25cf The run passed. Next I'll check the release notes in docs/release.md.", '']))
# A prompt repeating its working directory on every line.
pathlib.Path(fixture, 'repeat.txt').write_text('\n'.join([
    '~/proj (main|\u21917|\u2714) echo kek', 'kek', '~/proj (main|\u21917|\u2714) ls', 'notes.txt  src',
    '~/proj (main|\u21917|\u2714) git status --short', ' M src/main.rs', '~/proj (main|\u21917|\u2714)', '']))
pathlib.Path(fixture, 'compiler.txt').write_text('\n'.join([
    "src/upload/client.ts:31:7 - error TS2345: Argument of type 'string' is not assignable.",
    "src/upload/retry.ts:12:3 - error TS2304: Cannot find name 'backoff'.",
    "src/session/store.ts:88:14 - warning: unused variable 'ttl'.",
    "tests/int/session.test.ts:4:1 - error TS1005: ';' expected.",
    '$ npm run lint -- --fix && npm test -- --runInBand',
    '$ git diff --stat HEAD~1',
    ' src/upload/client.ts | 31 +++++++++++++++++++++++++------',
    ' src/upload/retry.ts  | 12 ++++++------',
    ' 2 files changed, 30 insertions(+), 13 deletions(-)', '']))
pathlib.Path(fixture, 'search.txt').write_text('\n'.join([
    '● Bash(rg -n "retry" src/ | head -5 && curl -s https://api.example.invalid/v1/status)',
    '  ⎿  src/upload/retry.ts:12:  const retry = backoff(3);',
    '     src/upload/client.ts:31:  await retry(() => put(url));',
    '     src/session/store.ts:40:  // retry once on 503 from 192.0.2.10:8443',
    '     {"status":"ok","region":"eu-west","commit":"7c3e9a1"}',
    '● Update(src/upload/retry.ts)',
    '  ⎿  Updated src/upload/retry.ts with 2 additions and 1 removal', '']))
pathlib.Path(fixture, 'paths.txt').write_text(
    f'Compiling sample v0.1.0 ({fixture}/src)\n'
    f'--prepared:{fixture}/src/example.swift\n'
    'See [./src/example.swift:12:3].\n'
    'target/release/missing.app\n')
inline_wrapped = 'sudo bash /tmp/' + 'sample/' * 30 + 'check.sh'
pathlib.Path(fixture, 'inline-wrapped.txt').write_text(
    'Run once: \x1b[38;2;100;170;230m' + inline_wrapped + '\x1b[0m, then continue.\n')
pathlib.Path(fixture, 'inline-screen.txt').write_text(
    "- Run once: \x1b[94msudo bash ./src/setup.sh\x1b[0m's folder\n"
    '- Or use \x1b[38;5;117msudo usermod -aG wheel example\x1b[0m, then continue.\n'
    "- Try \x1b[38;2;100;170;230mprintf '%s' color-ran > color-ran.txt\x1b[0m, then continue.\n"
    'DIR /b\nGet-ChildItem -Force\ngit status --short\n')
# A coding-agent answer, hard-wrapped to the live pane width by sheet.sh
# the way agents wrap their own output (newline plus repeated indent).
pathlib.Path(fixture, 'sheet-src.txt').write_text('\n'.join([
    "Right, first up, the big one you'd paste into the prompt:", '',
    '! find . -type f -name "*.swift" -not -path "./vendor/*" -not -path "./.dev/*" -exec grep -l '
    '"TerminalHint" {} + | xargs -I{} sh -c \'echo "== {}"; wc -l "{}"\' | sort -t: -k2 -n | tee /tmp/hints-report.txt', '',
    'The docs live at https://docs.example.invalid/azulejo/hints?section=terminal&lang=en-GB and you can '
    'find the mirror if you squint hard enough.', '',
    'Plain domain, no scheme: mirror.example.org', '',
    '$ cargo build --release --manifest-path shared/Cargo.toml --features "hints,mirror" && ls -lah target/release '
    '&& printf "%s\\n" one two three four five six seven', '',
    'The flaky box sits at 192.0.2.47, and its noisy neighbour is at 198.51.100.12 on port 8443 (198.51.100.12:8443).', '',
    'Session UUID: 3f9c2a7e-8b41-4d6f-a0c3-5e17b9d2f864', '',
    'That bug got squashed in commit 7c3e9a1f4b2d8e05a6f1c9b3d7e2a4f80b5c6d19 (short form 7c3e9a1).', '',
    "Check out feature/terminal-hints-v2 before you rebase onto release/2026.11, or just stay on main.", '']))
pathlib.Path(fixture, 'sheet.sh').write_text(r"""cols=$(stty size | cut -d' ' -f2)
printf '\033[2J\033[H'
awk -v w=$((cols - 1)) '{
  if ($0 == "") { print ""; next }
  line = "  "; n = split($0, words, " ")
  for (i = 1; i <= n; i++) {
    if (line == "  ") line = line words[i]
    else if (length(line) + 1 + length(words[i]) <= w) line = line " " words[i]
    else { print line; line = "  " words[i] }
  }
  print line
}' sheet-src.txt
""")
# A coding agent's answer with a long one-liner on its own line: the agent
# wraps it itself (newline plus indent, two columns short of the pane) and
# paints every row in its text colour, so no row carries the terminal's
# soft-wrap flag and colours mark nothing.
agent_loop = ('for env in dev prod; do s=sub-demo-$env-001; echo "== $env $(az provider show -n Example.App '
              '--subscription $s --query registrationState -o tsv) kv-rbac=$(az keyvault show -n kvdemo-$env-app01 '
              '--subscription $s --query properties.enableRbacAuthorization -o tsv)"; done')
pathlib.Path(fixture, 'agent-wrap-src.txt').write_text('\n'.join([
    'If you would rather skip the script, the check is short enough to type by hand. In the demo shell:',
    agent_loop, '', 'That prints one line per environment.', '']))
pathlib.Path(fixture, 'agent-wrap.sh').write_text(r"""cols=$(stty size | cut -d' ' -f2)
printf '\033[2J\033[H'
awk -v w=$((cols - 2)) '
function out(s) { printf "  \033[38;2;230;230;230m%s\033[39m\n", s }
{
  if ($0 == "") { print ""; next }
  line = ""; n = split($0, words, " ")
  for (i = 1; i <= n; i++) {
    if (line == "") line = words[i]
    else if (2 + length(line) + 1 + length(words[i]) <= w) line = line " " words[i]
    else { out(line); line = words[i] }
  }
  out(line)
}' agent-wrap-src.txt
""")
# Claude Code's own word wrap (Ink): a word longer than a row starts on the
# row it's on and breaks at the pane's edge, so a long program path or link
# splits mid-word, the rest on an indented row. `=` rows print as they are;
# a `● ` row keeps its bullet on the first row.
pathlib.Path(fixture, 'ink.sh').write_text(r"""cols=$(stty size | cut -d' ' -f2)
printf '\033[2J\033[H'
awk -v w=$((cols - 2)) '
function out(s) { printf "%s\033[38;2;230;230;230m%s\033[39m\n", lead, s; lead = "  " }
{
  if ($0 == "") { print ""; next }
  if (substr($0, 1, 1) == "=") { print substr($0, 2); next }
  lead = "  "; text = $0
  if (substr($0, 1, 4) == "\342\227\217 ") { lead = substr($0, 1, 4); text = substr($0, 5) }
  line = ""; n = split(text, words, " ")
  for (i = 1; i <= n; i++) {
    word = words[i]
    if (line != "" && length(line) + 1 + length(word) <= w) { line = line " " word; continue }
    if (length(word) <= w) { if (line != "") out(line); line = word; continue }
    if (line != "") line = line " "
    while (length(line) + length(word) > w) {
      k = w - length(line); out(line substr(word, 1, k)); word = substr(word, k + 1); line = ""
    }
    line = line word
  }
  out(line)
}' "$1"
""")
# An agent handing over `! cmd` lines to type; the long ones break inside
# the program's path.
bang = [
    '! /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister '
    '-f /Applications/Azulejo.app && killall Dock',
    '! /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister '
    '-gc && killall Dock',
    '! sudo rm -rf /Library/Caches/com.apple.iconservices.store && killall Dock']
pathlib.Path(fixture, 'bang-src.txt').write_text('\n'.join([
    bang[0], 'That forces macOS to re-register the installed app.', '',
    bang[1], 'That drops the registrations for builds that no longer exist on disk.', '',
    'As a last resort, wipe the icon cache (it rebuilds itself) and restart the Dock:', bang[2], '']))
# A link longer than a row in an agent's answer, and in plain output that
# the terminal soft-wraps. A `?` where the real link has a `#`: scripts
# end a line's text at `#` (core/crates/hints/tests/logical.rs keeps it).
artboard = 'https://claude.ai/artifact/DoFE22EBMbqBjD41cwUQf6?artboard-Icon-A3-1-single-stroke'
artboard_text = f'The A3 icon boards are on the canvas, to the right of A2 in the icon row: {artboard}'
pathlib.Path(fixture, 'url-src.txt').write_text('\n'.join([
    '=● Artifact("canvas/project/canvas.json")',
    '=  ⎿  Updated https://claude.ai/artifact/DoFE22EBMbqBjD41cwUQf6',
    '=  ⎿  Allowed by auto mode classifier', '',
    f'● {artboard_text}', '']))
pathlib.Path(fixture, 'url-soft.txt').write_text(artboard_text + '\n')
pathlib.Path(output).write_text(pathlib.Path(source).read_text().replace('@FISH@', fish_login or '-').replace('@DIR@', fixture)
                              .replace('@AGENT_LOOP@', agent_loop)
                              .replace('@WRAPPED@', wrapped).replace('@INLINE_WRAPPED@', inline_wrapped)
                              .replace('@LONG_LINE@', long_line)
                              .replace('@BANG1@', bang[0][2:]).replace('@BANG2@', bang[1][2:])
                              .replace('@BANG3@', bang[2][2:]).replace('@ARTBOARD@', artboard))
PY
(sleep 480; "$root/apps/macos/dev.sh" --stop) &
watchdog=$!
"$root/apps/macos/dev.sh" --automation "$fixtures/hints.azs" --out "$out"
printf 'terminal hints: GUI scenarios passed\n'
