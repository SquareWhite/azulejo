"""Summarise latency CSVs: per-run and pooled per-config stats, plus c − b."""
import csv
import glob
import os
import statistics
import sys

out = sys.argv[1]
runs: dict[str, list[float]] = {}
misses: dict[str, int] = {}
for path in sorted(glob.glob(os.path.join(out, "*.csv"))):
    label = os.path.basename(path)[:-4]
    vals, miss = [], 0
    with open(path) as f:
        for row in csv.DictReader(f):
            if row["latency_ms"]:
                vals.append(float(row["latency_ms"]))
            else:
                miss += 1
    runs[label], misses[label] = vals, miss


def pct(v: list[float], q: float) -> float:
    s = sorted(v)
    return s[min(len(s) - 1, round(q * (len(s) - 1)))]


def line(name: str, v: list[float], miss: int) -> str:
    return (f"| {name} | {len(v)} | {miss} | {min(v):.1f} | {statistics.median(v):.1f} | "
            f"{pct(v, .95):.1f} | {pct(v, .99):.1f} | {max(v):.1f} |")


print("| run | n | misses | min | median | p95 | p99 | max |")
print("| --- | --- | --- | --- | --- | --- | --- | --- |")
for k, v in runs.items():
    if v:
        print(line(k, v, misses[k]))
pooled = {}
for cfg in ("a", "b", "c"):
    v = [x for k, vs in runs.items() if k.split("-")[0] == cfg for x in vs]
    if v:
        pooled[cfg] = v
        print(line(f"**{cfg} pooled**", v, sum(m for k, m in misses.items() if k.split("-")[0] == cfg)))
if "b" in pooled and "c" in pooled:
    d = statistics.median(pooled["c"]) - statistics.median(pooled["b"])
    d95 = pct(pooled["c"], .95) - pct(pooled["b"], .95)
    print(f"\nc − b: median {d:+.1f} ms, p95 {d95:+.1f} ms (target ≤ 8.3 ms median)")

rtt = {}
for name in ("direct", "attach"):
    path = os.path.join(out, f"rtt-{name}.txt")
    if os.path.exists(path):
        lines = open(path).read().split("\n")
        rtt[name] = [float(x) for x in lines if x.strip()]
if rtt:
    print("\nEcho round trip, bytes only (no rendering):")
    print("| path | n | min | median | p95 | p99 | max |")
    print("| --- | --- | --- | --- | --- | --- | --- |")
    for k, v in rtt.items():
        print(f"| {k} | {len(v)} | {min(v):.3f} | {statistics.median(v):.3f} | {pct(v, .95):.3f} | {pct(v, .99):.3f} | {max(v):.3f} |")
    if len(rtt) == 2:
        print(f"\nattach − direct: median {statistics.median(rtt['attach']) - statistics.median(rtt['direct']):+.3f} ms, "
              f"p95 {pct(rtt['attach'], .95) - pct(rtt['direct'], .95):+.3f} ms")

if "a" in pooled:
    for cfg in ("b", "c"):
        if cfg in pooled:
            print(f"{cfg} − a (standalone Ghostty): median "
                  f"{statistics.median(pooled[cfg]) - statistics.median(pooled['a']):+.1f} ms, "
                  f"p95 {pct(pooled[cfg], .95) - pct(pooled['a'], .95):+.1f} ms")
