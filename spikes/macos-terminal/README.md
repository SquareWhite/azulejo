# Spike 0.3: libghostty surface running `herdr terminal attach`

One AppKit window, one libghostty surface. The surface's command is
`herdr --session azj-spike terminal attach <terminal-id> --takeover`, so the
process lives in herdr and survives the app quitting.

```sh
../../scripts/build-ghostty.sh   # GhosttyKit.xcframework (zig 0.17.0 via mise)
./build.sh                       # build/AzjSpike.app (swiftc, no Xcode needed)
./run.sh                         # start the azj-spike herdr session if needed, attach to its last pane
./run.sh --new                   # new herdr tab, attach to it
./run.sh --stop                  # stop the azj-spike session
./hs.sh pane list                # herdr CLI pinned to the azj-spike session
```

Notes

- `--takeover` must come **after** the terminal ID; `attach --takeover <id>` fails with "unknown option".
- Without Xcode there's no `metal` compiler, so `scripts/patches/ghostty-no-xcode.patch` makes Ghostty compile
  its shaders at runtime (`newLibraryWithSource`) and install `libghostty.a` directly
  (`AZJ_RUNTIME_METAL=1`). With Xcode the build script uses Ghostty's own xcframework flow.
- `run.sh` drops inherited `HERDR_*` variables and pins `HERDR_SOCKET_PATH`, so nothing reaches the
  user's live herdr session.
- Spike-only input: no IME / `NSTextInputClient`, text/plain clipboard only, no scrollbar or link handling.
- `build/shots/` has the screenshots from the verification run.
