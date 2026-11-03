"""Runs each test against a headless `azulejod` with its own socket and state,
never the user's. Terminal tests use a throwaway herdr session (`azj-py-*`)."""

from __future__ import annotations

import os
import shutil
import socket
import subprocess
import tempfile
import time
import uuid
from collections.abc import Iterator
from dataclasses import dataclass
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]


def _azulejod() -> Path:
    if path := os.environ.get("AZULEJOD"):
        return Path(path)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    return target / "debug" / "azulejod"


def _clean_env() -> dict[str, str]:
    """No inherited pointers at the user's herdr session or Azulejo."""
    return {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "AZULEJO_"))}


@dataclass
class Daemon:
    socket: Path
    state: Path
    proc: subprocess.Popen[bytes]
    herdr_session: str | None

    def stop(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()


def _start(tmp: Path, herdr_session: str | None) -> Daemon:
    binary = _azulejod()
    if not binary.exists():
        pytest.fail(f"{binary} is missing: run `cargo build -p azulejo-daemon` (or `just python-test`)")
    sock = tmp / "s.sock"
    state = tmp / "state.json"
    args = [str(binary), "--no-config", "--socket", str(sock), "--state", str(state)]
    args += ["--herdr-session", herdr_session] if herdr_session else ["--no-herdr"]
    proc = subprocess.Popen(args, env=_clean_env(), stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    deadline = time.monotonic() + 15
    while True:
        if proc.poll() is not None:
            err = proc.stderr.read().decode() if proc.stderr else ""
            pytest.fail(f"azulejod exited early ({proc.returncode}): {err}")
        try:
            with socket.socket(socket.AF_UNIX) as s:
                s.connect(str(sock))
            break
        except OSError:
            if time.monotonic() > deadline:
                proc.kill()
                pytest.fail("azulejod never opened its socket")
            time.sleep(0.02)
    return Daemon(sock, state, proc, herdr_session)


@pytest.fixture
def tmp_short() -> Iterator[Path]:
    # Unix socket paths are capped near 104 bytes on macOS; pytest's tmp_path is long.
    path = Path(tempfile.mkdtemp(prefix="azj-py-", dir="/tmp"))
    yield path
    shutil.rmtree(path, ignore_errors=True)


@pytest.fixture
def daemon(tmp_short: Path, monkeypatch: pytest.MonkeyPatch) -> Iterator[Daemon]:
    """A core without herdr. Tests run as if outside any Azulejo tile."""
    for k in list(os.environ):
        if k.startswith(("HERDR_", "AZULEJO_")):
            monkeypatch.delenv(k)
    d = _start(tmp_short, None)
    monkeypatch.setenv("AZULEJO_SOCKET", str(d.socket))
    yield d
    d.stop()


@pytest.fixture
def herdr_daemon(tmp_short: Path, monkeypatch: pytest.MonkeyPatch) -> Iterator[Daemon]:
    """A core on a throwaway herdr session, removed afterwards."""
    herdr = shutil.which("herdr")
    if herdr is None:
        pytest.skip("herdr isn't installed")
    for k in list(os.environ):
        if k.startswith(("HERDR_", "AZULEJO_")):
            monkeypatch.delenv(k)
    session = f"azj-py-{uuid.uuid4().hex[:8]}"
    d = _start(tmp_short, session)
    monkeypatch.setenv("AZULEJO_SOCKET", str(d.socket))
    try:
        yield d
    finally:
        d.stop()
        env = _clean_env()
        for cmd in (["session", "stop", session], ["session", "delete", session]):
            subprocess.run([herdr, *cmd], env=env, capture_output=True, timeout=30)
