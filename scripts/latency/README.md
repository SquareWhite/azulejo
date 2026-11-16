# Latency bench

Keystroke-to-glyph latency for a libghostty surface, direct and through
`herdr terminal attach`.

| Script | What | Safe while you work? |
| --- | --- | --- |
| `run.sh [samples] [rounds]` | In-process bench: config b (the surface runs `/bin/cat` directly) and c (`herdr terminal attach` to a pane running `/bin/cat`), plus a byte-level herdr round trip. Results land in `build/results/*.csv`. | Yes: no OS input, the window never activates |
| `run-os-input.sh [samples]` | Adds standalone Ghostty.app (config a) through the real OS input path | **No.** It brings windows to the front and posts real keystrokes for several minutes. Only run it when nobody is using the Mac. Needs Screen Recording and Accessibility for your terminal |
| `resize-probe.sh` | PTY resize → SIGWINCH in a herdr pane → redraw back at the attach client, without libghostty, in a throwaway herdr session (`--stop` deletes it) | Yes |

Pieces:

- `harness/`: a minimal libghostty terminal app with an in-process bench
  (`Sources/Bench.swift`). Its `--foreground` mode is for the OS-input run
  only.
- `osbench.swift`: the OS-input bench. It refuses to run without
  `--yes-post-real-keys` and stops posting as soon as the target app isn't
  frontmost.
- `herdr_rtt.py`: PTY echo round trip, direct against `herdr terminal attach`.
- `resize_pane.py`, `resize_client.py`: the pane and client sides of
  `resize-probe.sh`.
- `summarise.py`: per-run and pooled stats as markdown tables.

For latency inside the real app, use `scripts/nfr/latency.sh`.
