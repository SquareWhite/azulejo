#!/usr/bin/env python3
"""Converts the ⌘S detector's corpus (data/hints-corpus.txt)
into this crate's corpus format, for scoring commands only:

    convert-hints-corpus.py <hints-corpus.txt> > out.txt

Rows get a placeholder label (O), so row accuracy on the output means
nothing; command precision and recall do. Styled fixtures (rows with SGR
escapes) are skipped: their commands depend on colours this format can't
carry. Rows wider than the pane soft-wrap, as the corpus test does.
"""

import sys


def main():
    fixtures = []
    in_rows = False
    for line in open(sys.argv[1], encoding="utf-8").read().split("\n"):
        if line.startswith("@@ fixture "):
            name, category, cols = line[len("@@ fixture "):].split(" | ")
            fixtures.append({"name": name, "cols": int(cols[5:]), "rows": [], "cmds": [], "styled": False})
            in_rows = True
        elif line.startswith("@@ cmd "):
            in_rows = False
            fixtures[-1]["cmds"].append(line[len("@@ cmd "):])
        elif line.startswith("@@ + "):
            fixtures[-1]["cmds"][-1] += " " + line[len("@@ + "):]
        elif line.startswith("@@ "):
            in_rows = False
        elif in_rows and fixtures:
            if "\\e" in line:
                fixtures[-1]["styled"] = True
            fixtures[-1]["rows"].append(line)
    out = sys.stdout
    out.write("# Converted from hints-corpus.txt: commands only, placeholder row labels.\n\n")
    for f in fixtures:
        if f["styled"]:
            continue
        rows = f["rows"]
        while rows and not rows[-1]:
            rows.pop()
        out.write(f"=== {f['name']} cols={f['cols']} | converted\n")
        for row in rows:
            rest = row
            while len(rest) > f["cols"]:
                out.write(f"O~| {rest[:f['cols']]}\n")
                rest = rest[f["cols"]:]
            out.write(f"O| {rest}\n" if rest else "B|\n")
        for c in f["cmds"]:
            out.write(f"--- command: {c}\n")
        out.write("\n")


if __name__ == "__main__":
    main()
