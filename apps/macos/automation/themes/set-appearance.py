#!/usr/bin/env python3
"""set-appearance.py CONFIG key=value…: rewrites the [appearance] section of a
dev profile's config.toml (themes.sh edits the file as a user would; the app
picks it up through its watcher). Values are written as TOML strings."""

import sys
from pathlib import Path

path = Path(sys.argv[1])
pairs = [a.split("=", 1) for a in sys.argv[2:]]
lines = path.read_text().splitlines() if path.exists() else []
out, skipping = [], False
for line in lines:
    stripped = line.strip()
    if stripped.startswith("["):
        skipping = stripped == "[appearance]"
    if not skipping:
        out.append(line)
while out and not out[-1].strip():
    out.pop()
out += ["", "[appearance]"] + [f'{k} = "{v}"' for k, v in pairs]
# Write beside and rename over, as editors do.
tmp = path.with_name(path.name + ".tmp")
tmp.write_text("\n".join(out) + "\n")
tmp.replace(path)
