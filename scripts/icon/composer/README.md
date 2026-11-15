# Y, the prompt key, as Icon Composer layers

The source for a layered `AppIcon.icon`, for the day Xcode is installed. `just icon` writes the SVGs from `scripts/icon/draw.swift` (`--composer`), so they always match the shipping `.icns`. Don't edit them by hand.

| File | Layer |
| --- | --- |
| `1-rings.svg` | A quarter ring centred on each corner (radius 0.32 to 0.47 of the half-tile), `#899DC7`, cobalt at 50 % on the glaze made opaque |
| `2-dots.svg` | A quarter dot in each corner (the mask hides it) and a half dot at each edge midpoint |
| `3-key.svg` | The key: a continuous-corner rounded square, a little over half the tile across |
| `4-prompt.svg` | `>_` |

Each one is drawn full bleed on a 1024 × 1024 canvas: the tile's edge is the canvas edge, and the system's mask cuts the corners. The flat `.icns` puts the tile in an 824 px square with a 100 px margin instead; that's the older template, not what Icon Composer wants.

## Assembling it

In Icon Composer (it ships with Xcode 26):

1. New document. Background: a linear gradient, top to bottom, the colours below.
2. Add the layers bottom to top: rings, dots, key, prompt. Put rings and dots in one group, the key and the prompt in another, so the key casts its own shadow onto the pattern.
3. Leave out the effects the `.icns` bakes in: the sheen, the glass edge, the key's gradient, glow and shadow, and the prompt's halo. Liquid Glass draws its own light.
4. Set the dark appearance's colours on each layer from the table. Clear and tinted come from the system; check them in Icon Composer's preview.
5. Save as `apps/macos/AppIcon.icon`.

| Layer | Light | Dark |
| --- | --- | --- |
| Background | `#FFFEFB` → `#EEE8DA` | `#2C2E35` → `#0F1013` |
| Rings | `#8EA1CB` | `#2C3859` |
| Dots | `#1C46A8` | `#5272C4` |
| Key | `#E2E6F0` → `#C6CBD7` | `#3B404D` → `#1E2128` |
| Prompt | `#1C46A8` | `#94B2FF`, `#D2DEFF` at the top |

The `.icns` also bakes in macOS 26's bright glass edge (`Bevel` in `draw.swift`); Icon Composer draws that itself, so leave it out too.

Then compile it into the bundle with `actool` (docs/design/icon/README.md, "What Xcode would add"). Once `Assets.car` ships, macOS draws the dark, clear and tinted looks itself in Finder, Launchpad, Spotlight and the Dock, and `DockIcon.swift`'s run-time swap can go.
