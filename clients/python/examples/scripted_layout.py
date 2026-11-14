#!/usr/bin/env python3
"""Scripted layouts: one command opens a project's layout (PRD use case).

A new tab in the tall layout: an editor terminal as master, then a test
watcher, a dev server and a browser on the dev server down the stack.

    uv run examples/scripted_layout.py ~/code/app --tests "npm test -- --watch" \\
        --server "npm run dev" --url http://localhost:3000
"""

import argparse
from pathlib import Path

from azulejo import Client


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("project", type=Path, nargs="?", default=Path.cwd())
    p.add_argument("--editor", default="")
    p.add_argument("--tests", default="")
    p.add_argument("--server", default="")
    p.add_argument("--url", default="http://localhost:3000")
    args = p.parse_args()
    cwd = str(args.project.expanduser().resolve())

    def terminal(command: str) -> dict:
        content: dict = {"kind": "terminal", "cwd": cwd}
        if command:
            content["command"] = ["sh", "-c", command]
        return content

    # caller=False: lay out in the focused workspace, wherever this runs.
    with Client(caller=False) as c:
        tab = c.tab_create(name=args.project.name or "project", content=terminal(args.editor))
        for content in (terminal(args.tests), terminal(args.server), {"kind": "browser", "url": args.url}):
            c.layout_split_auto(tab=tab["tab"], content=content)
        layout = c.layout_get()
        print(f"tab {tab['tab']}: {sum(1 for _ in leaves(layout['tree']))} tiles")


def leaves(node: dict):
    if node["type"] == "leaf":
        yield node["id"]
    else:
        yield from leaves(node["a"])
        yield from leaves(node["b"])


if __name__ == "__main__":
    main()
