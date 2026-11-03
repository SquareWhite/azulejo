#!/usr/bin/env python3
"""Turn `herdr terminal session observe` NDJSON into raw ANSI bytes on stdout.

Also prints a one-line summary of record types to stderr, so we learn the
stream's shape."""
import base64, collections, json, sys

counts = collections.Counter()
keys = {}
out = sys.stdout.buffer
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        rec = json.loads(line)
    except json.JSONDecodeError:
        counts["<non-json>"] += 1
        continue
    kind = rec.get("type") or rec.get("event") or rec.get("method") or "?"
    counts[kind] += 1
    keys.setdefault(kind, sorted(rec.keys()) if isinstance(rec, dict) else [])
    payload = rec
    for k in ("data", "params", "frame"):
        if isinstance(payload, dict) and isinstance(payload.get(k), dict):
            payload = payload[k]
    for k in ("bytes", "data", "ansi", "base64", "frame"):
        v = payload.get(k) if isinstance(payload, dict) else None
        if isinstance(v, str):
            try:
                out.write(base64.b64decode(v))
            except Exception:
                pass
            break
out.flush()
print(f"records={dict(counts)} keys={keys}", file=sys.stderr)
