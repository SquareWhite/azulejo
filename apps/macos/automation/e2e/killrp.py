#!/usr/bin/env python3
"""killrp.py APP_PID PATTERN [--dry]: SIGKILL the processes matching PATTERN
whose *responsible* process is APP_PID (WebKit and QuickLook XPC services
the dev app spawned). Never touches another app's helpers: each candidate's
responsible pid is checked with responsibility_get_pid_responsible_for_pid.
Prints what it killed (or would kill)."""
import ctypes, os, signal, subprocess, sys

app, pattern = int(sys.argv[1]), sys.argv[2]
dry = "--dry" in sys.argv
lib = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
rp = lib.responsibility_get_pid_responsible_for_pid
rp.restype, rp.argtypes = ctypes.c_int, [ctypes.c_int]
pids = subprocess.run(["pgrep", "-i", "-f", pattern], capture_output=True, text=True).stdout.split()
def comm(p):
    return subprocess.run(["ps", "-o", "comm=", "-p", str(p)], capture_output=True, text=True).stdout.strip()
skip = {os.getpid(), os.getppid()}
ours = [int(p) for p in pids
        if int(p) not in skip and rp(int(p)) == app
        and os.path.basename(comm(p)) not in ("sh", "bash", "zsh", "fish", "python3", "Python")]
for p in ours:
    name = subprocess.run(["ps", "-o", "comm=", "-p", str(p)], capture_output=True, text=True).stdout.strip()
    print(("would kill" if dry else "killed"), p, name)
    if not dry:
        os.kill(p, signal.SIGKILL)
if not ours:
    print("none of", pattern, "belongs to", app)
