#!/usr/bin/env bash
# Visible-terminal hints: pure recognition boundaries plus real key routing,
# frozen selection, private-pasteboard copy, open dispatch and command execution.
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
config="$root/.dev/$AZJ_DEV_NAME/config/azulejo/config.toml"
mkdir -p "$(dirname "$config")"
printf '[herdr]\nsession = "%s"\n[keys.macos.terminal]\nselect_visible_item = "cmd+s"\n' \
  "${AZJ_DEV_SESSION:-azj-dev-$AZJ_DEV_NAME}" > "$config"
mkdir -p "$fixtures/src" "$out"
touch "$fixtures/src/example.swift" "$fixtures/src/setup.sh" "$fixtures/two words.txt"
printf '%s\n' 'https://example.invalid/first' '界👩‍💻é ./src/example.swift:12:3' '* feature/hints' '  feature/other' '$ printf hint-ran > ran.txt' '"two words.txt"' > "$fixtures/screen.txt"
printf '%s\n' '!printf agent-ran > agent-ran.txt' > "$fixtures/agent-screen.txt"
swiftc "$root/apps/macos/Sources/Azulejo/TerminalHints.swift" "$here/hints-detector.swift" -o "$fixtures/detector"
"$fixtures/detector"
# Substitute fixture paths without embedding workstation paths in source.
python3 - "$here/hints.azs.in" "$fixtures/hints.azs" "$fixtures" <<'PY'
import pathlib, sys
source, output, fixture = sys.argv[1:]
wrapped = 'https://example.invalid/' + 'path/' * 30 + 'end'
pathlib.Path(fixture, 'wrapped.txt').write_text(wrapped + '\n')
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
pathlib.Path(output).write_text(pathlib.Path(source).read_text().replace('@DIR@', fixture)
                              .replace('@WRAPPED@', wrapped).replace('@INLINE_WRAPPED@', inline_wrapped))
PY
(sleep 180; "$root/apps/macos/dev.sh" --stop) &
watchdog=$!
"$root/apps/macos/dev.sh" --automation "$fixtures/hints.azs" --out "$out"
printf 'terminal hints: GUI scenarios passed\n'
