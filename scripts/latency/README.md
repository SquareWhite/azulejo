# Latency bench (PLAN 0.4)

- `run.sh [samples] [rounds]`: in-process bench, configs b (direct `/bin/cat`) and c (`herdr terminal attach`), plus a byte-level herdr round trip. It doesn't post OS input or steal focus, so it's safe to run while you're working.
- `run-os-input.sh [samples]`: adds standalone Ghostty.app (a) through the real OS input path. **It brings windows to the front and posts real keystrokes. Only run it when nobody is using the Mac.**
- `harness/`: a copy of `spikes/macos-terminal` with an in-process bench (`Sources/Bench.swift`). It has a `--foreground` mode for the OS-input run only.
- `osbench.swift`: the OS-input bench. It refuses to run without `--yes-post-real-keys`.
- `herdr_rtt.py`: PTY echo round trip, direct vs `herdr terminal attach`.
- `summarise.py`: prints the markdown tables.

Results and method: [docs/measurements.md](../../docs/measurements.md).
