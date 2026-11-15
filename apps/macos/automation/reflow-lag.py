#!/usr/bin/env python3
"""Reflow lag: how far a terminal's text trails (or leads) its tile's frame.

  reflow-lag.py STATS.json WINCH_DIR [--draw-ms 27]

STATS.json is `anim stats` output (reflow_resizes: [pane, asked, frame time,
cols, rows]; reflow_ends). WINCH_DIR holds winch-<pane>.log from
winch-log.py. Both use the same clock (mach absolute time).

Each SIGWINCH a pane saw is matched to the app's latest resize of that pane
to that grid. The text for it shows at SIGWINCH + draw-ms (the program's
redraw through herdr to libghostty: herdr draws about 19 ms after a resize,
plus up to a frame to present; see docs/measurements.md). Lag = that time -
when the tile's frame had that size: positive trails, negative leads.
"""
import json
import os
import statistics as st
import sys


def pct(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round((len(xs) - 1) * p)))]


def line(name, xs, unit="ms"):
    if not xs:
        return f"{name}: n=0"
    return (f"{name}: n={len(xs)} median={st.median(xs):.0f} p10={pct(xs, .1):.0f} "
            f"p90={pct(xs, .9):.0f} min={min(xs):.0f} max={max(xs):.0f} {unit}")


def main() -> None:
    stats_path, winch_dir, *rest = sys.argv[1:]
    draw = float(rest[rest.index("--draw-ms") + 1]) / 1000 if "--draw-ms" in rest else 0.027
    stats = json.load(open(stats_path))
    resizes = stats.get("reflow_resizes", [])
    if not resizes:
        print("no reflow resizes in", stats_path)
        return
    ends = sorted(set(round(e, 3) for e in stats.get("reflow_ends", [])))
    lo = min(r[1] for r in resizes) - 0.05
    hi = max(ends + [max(r[1] for r in resizes)]) + 0.6
    by_pane: dict[str, list] = {}
    for pane, asked, frame, cols, rows in resizes:
        by_pane.setdefault(pane.replace(":", "_"), []).append((asked, frame, cols, rows))
    lags, lasts, winches, unmatched = [], [], 0, 0
    per_motion_req, per_motion_winch = [], []
    for pane, recs in by_pane.items():
        path = os.path.join(winch_dir, f"winch-{pane}.log")
        if not os.path.exists(path):
            continue
        seen = []
        for raw in open(path):
            t, c, r = raw.split()
            t = float(t)
            if lo <= t <= hi:
                seen.append((t, int(c), int(r)))
        matched = []  # (shown at, frame time, asked)
        for t, c, r in seen:
            winches += 1
            cand = [x for x in recs if x[0] <= t and x[2] == c and x[3] == r]
            if not cand:
                unmatched += 1
                continue
            asked, frame, _, _ = max(cand)
            matched.append((t + draw, frame, asked))
            lags.append((t + draw - frame) * 1000)
        prev = lo
        for e in ends:
            reqs = [x for x in recs if prev < x[0] <= e + 0.002]
            mine = [m for m in matched if prev < m[2] <= e + 0.002]
            if reqs:
                per_motion_req.append(len(reqs))
                per_motion_winch.append(len(mine))
                if mine:
                    lasts.append((max(m[0] for m in mine) - e) * 1000)
            prev = e + 0.002
    print(f"lead {stats.get('reflow_lead_ms')} ms, interval {stats.get('reflow_interval_ms')} ms, "
          f"draw {draw * 1000:.0f} ms; {len(ends)} motion end(s), {len(resizes)} app resizes, "
          f"{winches} SIGWINCH ({unmatched} unmatched)")
    print(line("text vs frame (+ trails)", lags))
    print(line("last redraw vs motion end", lasts))
    if per_motion_req:
        print(f"per terminal per motion: app resizes {st.mean(per_motion_req):.1f}, "
              f"SIGWINCH in the pane {st.mean(per_motion_winch):.1f}")


main()
