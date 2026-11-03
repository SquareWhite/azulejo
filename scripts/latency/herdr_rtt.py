"""Byte-level echo round trip: keystroke written to a PTY → echoed bytes read back.

  python3 herdr_rtt.py direct <samples>                 /bin/cat on a PTY (kernel echo)
  python3 herdr_rtt.py attach <samples> <session> <id>  `herdr terminal attach` on a PTY,
                                                        pane running /bin/cat

No rendering, no screen capture: this isolates the extra hop that
`herdr terminal attach` adds (client → herdr server → pane PTY → herdr render
→ client), at microsecond resolution. Prints one latency (ms) per line.
"""
import fcntl
import os
import pty
import random
import select
import struct
import sys
import termios
import time


def spawn(argv: list[str]) -> tuple[int, int]:
    pid, fd = pty.fork()
    if pid == 0:
        os.execvp(argv[0], argv)
    # herdr attach refuses a zero-sized PTY.
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
    return pid, fd


def drain(fd: int, quiet: float) -> bytes:
    """Read until the PTY has been quiet for `quiet` seconds."""
    buf = b""
    while True:
        r, _, _ = select.select([fd], [], [], quiet)
        if not r:
            return buf
        try:
            buf += os.read(fd, 65536)
        except OSError:
            return buf


def wait_for(fd: int, needle: bytes, timeout: float) -> float | None:
    """Seconds until `needle` shows up in the output, or None."""
    t0 = time.perf_counter()
    buf = b""
    while (left := timeout - (time.perf_counter() - t0)) > 0:
        r, _, _ = select.select([fd], [], [], left)
        if r:
            buf += os.read(fd, 65536)
            if needle in buf:
                return time.perf_counter()
    return None


def main() -> None:
    mode, n = sys.argv[1], int(sys.argv[2])
    if mode == "direct":
        pid, fd = spawn(["/bin/cat"])
    else:
        session, term_id = sys.argv[3], sys.argv[4]
        pid, fd = spawn(["herdr", "--session", session, "terminal", "attach", term_id, "--takeover"])
    drain(fd, 1.0)
    for _ in range(n):
        time.sleep(random.uniform(0.03, 0.08))
        drain(fd, 0.0)
        t0 = time.perf_counter()
        os.write(fd, b"x")
        t1 = wait_for(fd, b"x", 1.0)
        print(f"{(t1 - t0) * 1000:.3f}" if t1 else "", flush=True)
        os.write(fd, b"\x7f")
        drain(fd, 0.03)
    os.kill(pid, 15)


main()
