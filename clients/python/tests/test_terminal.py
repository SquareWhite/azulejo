"""Terminal I/O and herdr caller resolution, on a throwaway herdr session."""

import subprocess
import sys
import time
from pathlib import Path

from azulejo import Client


def bound_terminal(c: Client, timeout: float = 15) -> dict:
    deadline = time.monotonic() + timeout
    while True:
        for t in c.tile_list()["tiles"]:
            if t["kind"] == "terminal" and t.get("herdr"):
                return t
        assert time.monotonic() < deadline, "no terminal got a herdr pane"
        time.sleep(0.1)


def test_drive_another_terminal(herdr_daemon):
    with Client() as c:
        tile = bound_terminal(c)["id"]
        # Works in any shell (the user's may be fish): the output differs from
        # the command line, so seeing it means the command ran.
        c.terminal_send_text("printf 'py-%s\\n' 42\n", tile=tile)
        deadline = time.monotonic() + 10
        while "py-42" not in c.terminal_read(tile=tile, source="recent_unwrapped")["text"]:
            assert time.monotonic() < deadline
            time.sleep(0.1)


def test_herdr_pane_fallback_finds_the_caller(herdr_daemon, monkeypatch):
    with Client() as c:
        t = bound_terminal(c)
        herdr_socket = c.call("core.version")["herdr_socket"]
        c.layout_split("right")  # focus elsewhere
    # As if run in that pane after herdr dropped Azulejo's variables.
    monkeypatch.setenv("HERDR_PANE_ID", t["herdr"]["pane_id"])
    monkeypatch.setenv("HERDR_SOCKET_PATH", herdr_socket)
    with Client() as inside:
        inside.tile_rename(title="found me")
        assert inside.tile_get(tile=t["id"]).get("title") == "found me"
    # A pane ID from another herdr session is never taken for ours.
    monkeypatch.setenv("HERDR_SOCKET_PATH", "/nonexistent/herdr.sock")
    with Client() as other:
        other.tile_rename(title="focused one")
        assert other.tile_get(tile=t["id"]).get("title") == "found me"


def test_drive_terminal_example(herdr_daemon):
    examples = Path(__file__).resolve().parents[1] / "examples"
    r = subprocess.run(
        [sys.executable, str(examples / "drive_terminal.py"), "printf", "'example-%s\\n'", "42"],
        capture_output=True, text=True, timeout=60,
    )
    assert r.returncode == 0, r.stderr
    assert "example-42" in r.stdout
