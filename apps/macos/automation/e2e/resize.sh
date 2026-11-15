#!/usr/bin/env bash
# R1: stale rows after vertical resizes (resize.azs.in) on dev instance
# e2e-rsz. Three terminals (a shell, vim, a bottom-anchored Ink-style
# redraw); the root divider is dragged up and down, and after every drag the
# text libghostty shows in each terminal must match herdr's own copy of the
# pane, row for row. A row libghostty kept from an earlier frame (a "trash
# line") shows up as a difference.
#
# Known, and only a NOTE: the Ink-style block is taller (12 rows) than the
# pane gets at its smallest (6), so its redraw can't reach the rows that
# went into scrollback, and herdr's own emulator pulls them back into view
# when the pane grows again. Those stale blocks are in herdr's copy too:
# herdr's side, not Azulejo's (docs/herdr-notes.md, question 13).
set -euo pipefail
export AZJ_DEV_NAME="${AZJ_DEV_NAME:-e2e-rsz}" E2E_SUITE=R1-resize
source "$(dirname "$0")/lib.sh"
trap e2e_cleanup EXIT
reset_instance
write_config <<'EOF'
[layout]
default = "manual"
EOF

fx="$out/fx"
rm -rf "$fx"; mkdir -p "$fx"
seq -f 'vim line %g' 1 400 >"$fx/lines.txt"
cp "$e2e_here/inkish.py" "$fx/inkish.py"

screens() { # NAME → the azs lines that dump both sides for every pane
  local n="$1" i
  echo "divider screens $n"
  for i in 1 2 3; do
    echo "sh p={pane:$i}; {herdr} --session {session} pane read \$p --source visible --format text > {out}/$n-herdr-\$(echo \$p | tr : _).txt"
  done
}
azs="$out/resize.azs"
while IFS= read -r line; do
  if [[ "$line" == SCREENS\ * ]]; then screens "${line#SCREENS }"; else printf '%s\n' "${line//\{FX\}/$fx}"; fi
done <"$e2e_here/resize.azs.in" >"$azs"
run_azs "$azs" resize

python3 - "$out/resize" <<'PY' | tee -a "$checks"
import pathlib, re, sys
d = pathlib.Path(sys.argv[1])
def rows(p):
    lines = [l.rstrip() for l in p.read_text(errors="replace").split("\n")]
    while lines and not lines[-1]:
        lines.pop()
    return lines
ok, compared = True, 0
for ours in sorted(d.glob("*.txt")):
    m = re.fullmatch(r"(\w+)-(w\d+_p\d+)\.txt", ours.name)
    if not m:
        continue
    theirs = d / f"{m[1]}-herdr-{m[2]}.txt"
    if not theirs.exists():
        print(f"FAIL {ours.stem}: no herdr copy"); ok = False; continue
    a, b = rows(ours), rows(theirs)
    compared += 1
    bad = [i for i in range(max(len(a), len(b))) if (a[i] if i < len(a) else "") != (b[i] if i < len(b) else "")]
    if bad:
        ok = False
        print(f"FAIL {ours.stem}: {len(bad)} row(s) differ from herdr's (libghostty {len(a)} rows, herdr {len(b)})")
        for i in bad[:4]:
            print(f"     row {i + 1}: libghostty {a[i] if i < len(a) else '':.60}")
            print(f"     {'':>{len(str(i + 1)) + 5}} herdr      {b[i] if i < len(b) else '':.60}")
print(f"{'PASS' if ok and compared else 'FAIL'} every terminal matches herdr's pane after each resize ({compared} compared)")
stale = sorted(p.name.split("-herdr-")[0] for p in d.glob("*-herdr-*.txt")
               if p.read_text(errors="replace").count("prompt block") > 1)
print(f"NOTE stale Ink blocks in herdr's own copy (pulled back from scrollback): {', '.join(stale) or 'none'}")
PY
! grep -q FAIL "$checks"
