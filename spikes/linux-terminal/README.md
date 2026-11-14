# Spike 0.7: Linux terminal path

GTK4 + VTE window running `herdr terminal attach <id>`, plus a libghostty-vt replay of herdr's observer stream. Findings: [docs/linux-renderer.md](../../docs/linux-renderer.md).

```sh
./run.sh         # build the image (first run compiles libghostty-vt, ~5 min) and run everything
./run.sh shell   # poke around inside the container
```

| Path | What |
| --- | --- |
| `Dockerfile` | Ubuntu 24.04 + GTK4/VTE/libadwaita, Rust, herdr v0.9.5, Zig 0.17, libghostty-vt at a pinned commit |
| `in-container.sh` | Starts herdr + Xvfb, launches the app, drives vim/htop/scroll/takeover, takes screenshots, replays the stream |
| `vte/` | gtk4-rs + vte4 app: `azj-vte-spike -- <program> [args]` |
| `vtdump/` | `vtdump.c` (libghostty-vt grid dump) and `decode_frames.py` (observer NDJSON → raw ANSI) |
| `out/` | Screenshots and the grid dump from the last run |
