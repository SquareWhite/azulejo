#!/usr/bin/env python3
"""An agent drives another terminal (PRD use cases "agent supervision" and
F20): open a terminal tile beside this one, run a command in it and read its
output back.

    uv run examples/drive_terminal.py "ls -la"
"""

import sys
import time

from azulejo import ApiError, Client


def wait_for_pane(c: Client, tile: str, timeout: float = 10) -> None:
    """A new terminal tile gets its herdr pane a moment after it opens."""
    deadline = time.monotonic() + timeout
    while c.tile_get(tile=tile).get("herdr") is None:
        if time.monotonic() > deadline:
            sys.exit("the terminal never got a herdr pane (is herdr running?)")
        time.sleep(0.1)


def main() -> None:
    command = " ".join(sys.argv[1:]) or "echo hello from azulejo"
    marker = "__azj_done__"
    with Client() as c:
        tile = c.layout_split("right")["tile"]
        wait_for_pane(c, tile)
        c.terminal_send_text(f"{command}; echo {marker}\n", tile=tile)
        deadline = time.monotonic() + 15
        while True:
            try:
                text = c.terminal_read(tile=tile, source="recent_unwrapped")["text"]
            except ApiError as e:
                if e.code != "unavailable":
                    raise
                text = ""
            # The marker shows twice once it ran: in the echoed command line, and alone.
            done = [l for l in text.splitlines() if l.strip() == marker]
            if done:
                break
            if time.monotonic() > deadline:
                sys.exit(f"no output yet; last screen:\n{text}")
            time.sleep(0.1)
        lines = text.splitlines()
        start = max(i for i, l in enumerate(lines) if command in l and marker in l)
        end = max(i for i, l in enumerate(lines) if l.strip() == marker)
        print("\n".join(lines[start + 1 : end]))
        print(f"(tile {tile})", file=sys.stderr)


if __name__ == "__main__":
    main()
