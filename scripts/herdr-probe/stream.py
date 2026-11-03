#!/usr/bin/env python3
"""Run `herdr terminal session observe|control` for a while and capture its frames.

Usage:
  stream.py observe TARGET SECONDS OUT_PREFIX [--cols N --rows N]
  stream.py control TARGET SECONDS OUT_PREFIX [--takeover] [--cols N --rows N] [--cmds FILE]

Writes OUT_PREFIX.ndjson (raw records, each with a receive timestamp added as "_t")
and OUT_PREFIX.bin (all decoded frame bytes concatenated).
--cmds FILE: NDJSON lines {"after": seconds, "cmd": {...}} written to the controller's stdin.
herdr is invoked through $HERDR (default "herdr"); set HERDR_SESSION to pick a session.
"""
import base64
import json
import os
import subprocess
import sys
import threading
import time


def main() -> None:
    mode, target, seconds, prefix, *rest = sys.argv[1:]
    seconds = float(seconds)
    cmds = []
    extra = []
    i = 0
    while i < len(rest):
        if rest[i] == "--cmds":
            cmds = [json.loads(l) for l in open(rest[i + 1]) if l.strip()]
            i += 2
        else:
            extra.append(rest[i])
            i += 1
    argv = [os.environ.get("HERDR", "herdr")]
    if os.environ.get("HERDR_SESSION"):
        argv += ["--session", os.environ["HERDR_SESSION"]]
    argv += ["terminal", "session", mode, target, *extra]
    env = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_") or k == "HERDR_SESSION"}
    env.pop("HERDR_SESSION", None)
    proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    start = time.time()

    def feeder() -> None:
        for c in sorted(cmds, key=lambda c: c["after"]):
            delay = c["after"] - (time.time() - start)
            if delay > 0:
                time.sleep(delay)
            try:
                proc.stdin.write((json.dumps(c["cmd"]) + "\n").encode())
                proc.stdin.flush()
            except BrokenPipeError:
                return

    threading.Thread(target=feeder, daemon=True).start()
    timer = threading.Timer(seconds, proc.terminate)
    timer.start()
    with open(prefix + ".ndjson", "w") as nd, open(prefix + ".bin", "wb") as bn:
        for line in proc.stdout:
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                nd.write(json.dumps({"_t": time.time() - start, "_raw": line.decode(errors="replace")}) + "\n")
                continue
            rec["_t"] = round(time.time() - start, 4)
            nd.write(json.dumps(rec) + "\n")
            nd.flush()
            for key in ("data", "bytes", "ansi", "frame"):
                val = rec.get(key) if isinstance(rec.get(key), str) else None
                if val is None and isinstance(rec.get("data"), dict):
                    val = rec["data"].get(key) if isinstance(rec["data"].get(key), str) else None
                if val:
                    try:
                        bn.write(base64.b64decode(val))
                        bn.flush()
                        break
                    except Exception:
                        pass
    timer.cancel()
    err = proc.stderr.read().decode(errors="replace")
    if err:
        sys.stderr.write(err)


if __name__ == "__main__":
    main()
