# App icon layers for Icon Composer

The app icon (`>_` on a tile pattern, A8-2) as separate layers, the source
for a layered `AppIcon.icon`. The shipping app uses the flat
`apps/macos/Resources/AppIcon.icns`, which needs no Xcode. `just icon` writes
these SVGs from `scripts/icon/draw.swift` (`--composer`) along with the
`.icns`, so they always match. Don't edit
them by hand.

| File | Layer |
| --- | --- |
| `1-pattern.svg` | A quarter ring centred on each corner (radius 0.36 to 0.44 of the half-tile) and a half dot at each edge midpoint (radius 0.09), one cobalt |
| `2-prompt.svg` | `>_`: a chevron and a bar, round caps and a round join, 0.104 of the half-tile wide (1.3 × the ring band), in the ring's colour |

Each one is drawn full bleed on a 1024 × 1024 canvas: the tile's edge is the canvas edge, and the system's mask cuts the corners. The flat `.icns` puts the tile in an 824 px square with a 100 px margin instead; that's the older template, not what Icon Composer wants.

## Assembling it

In Icon Composer (it ships with Xcode 26):

1. New document. Background: a linear gradient, top to bottom, the colours below.
2. Add the layers bottom to top: pattern, then prompt, each in its own group.
3. Leave out the effects the `.icns` bakes in: the sheen and the glass edge. Liquid Glass draws its own light.
4. The other appearances come from the system; check them in Icon Composer's preview.
5. Save as `apps/macos/AppIcon.icon`.

| Layer | Colour |
| --- | --- |
| Background | `#FFFEFB` → `#EEE8DA` |
| Pattern | `#1C46A8` |
| Prompt | `#1C46A8` |

The `.icns` also bakes in macOS 26's bright glass edge (`Bevel` in `draw.swift`); Icon Composer draws that itself, so leave it out too.

## Compiling it into the bundle

`actool`, which ships with Xcode but not the Command Line Tools, compiles the
`.icon` document into `Contents/Resources/Assets.car` under the name in
`CFBundleIconName` (already `AppIcon`). The build scripts don't do this yet.
To add it, have `apps/macos/bundle.sh` and `scripts/release/build-mac.sh` run
something like this when `xcrun --find actool` succeeds (check the flags
against your Xcode):

```sh
xcrun actool apps/macos/AppIcon.icon --compile <bundle>/Contents/Resources \
  --app-icon AppIcon --platform macosx --minimum-deployment-target 14.0 \
  --output-partial-info-plist <tmp>/partial.plist
```

Keep `AppIcon.icns` as the fallback for macOS 14 and 15 and for builds without
Xcode.
