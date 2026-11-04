# App icon

![All eight candidates](options.png)

the maintainer's brief (Nov 4, 2026): "a pretty azulejo tile, slightly stylized to match the macOS Liquid Glass aesthetic". Cobalt on a warm tin glaze, a symmetric floral-geometric motif, corner quarter-motifs that would join up with the next tile, and a fine border.

## Candidates

[`options.png`](options.png) puts all eight side by side: 256 px on a light row and a dark row, with the 32 and 16 px renders under each at 1:1 and blown up. [`sheet.png`](sheet.png) is the deeper look at A–C, with the dark and tinted variants and system icons for scale. It stays at A–C because all eight would push it to about 4 MB.

| | File | What it is |
| --- | --- | --- |
| **A · classic tile (default)** | [`candidate-a-classic.png`](candidate-a-classic.png) | One tile: an eight-petal rosette, quarter flowers in the corners, buds at the edge midpoints strung on a fine border |
| B · 2×2 cluster | [`candidate-b-cluster.png`](candidate-b-cluster.png) | Four tiles, cobalt and white alternating like the Alfama pattern in the mockup's browser tile; the joints echo the split layout |
| C · minimal, cobalt glaze | [`candidate-c-minimal.png`](candidate-c-minimal.png) | Inverted: one white quatrefoil with a star on a cobalt glaze, pale quarter rings in the corners |
| D · Lisbon blue & ochre | [`candidate-d-ochre.png`](candidate-d-ochre.png) | Two pigments, like most Lisbon pattern tiles: an ochre eight-pointed star in a cobalt diamond, ochre quarter discs in the corners, ochre half-buds at the edges |
| E · prompt rosette | [`candidate-e-prompt.png`](candidate-e-prompt.png) | A cobalt medallion on an eight-petal flower in a ring of beads, with `>_` left in bare glaze at its heart. Tile first, terminal second |
| F · glass stack | [`candidate-f-glass.png`](candidate-f-glass.png) | Three tiles stacked in depth on a sky glaze, pale blue, cobalt and frosted white. Each pane blurs what's behind it, casts its own shadow and has a rim light |
| G · split layout | [`candidate-g-split.png`](candidate-g-split.png) | One tile cut by grout into the Tile layout, master left and two stacked right. One rosette runs across all three panels and swaps glaze in each |
| H · night glaze | [`candidate-h-night.png`](candidate-h-night.png) | Ink-blue glaze, a white daisy with a gold heart, a ring of gold beads, white and gold quarter rings in the corners. Made for a dark Dock |

**Small sizes, D–H.** D and H are the strongest new ones at 16 px: D keeps its two colours (a yellow star in a blue diamond), H stays a white flower on navy. E reads as a blue disc with `>_` at 32. At 16 the prompt is a hint, and from across the Dock it could pass for a Terminal-style badge. F is stacked panes at 32 and pale on pale at 16. At 32 and below its front tile shows a diamond, not the rosette, which shrank into a "+" button. G is a blue master beside two light panes at both sizes. Its rosette becomes one disc split three ways when small, because the petals read as an "×".

**How to choose.** Set `chosen` in `scripts/icon/draw.swift` (for example `let chosen = Candidate.night`), then run `just icon`. That rebuilds `AppIcon.icns`, the dark and tinted variants, sheet.png and the stamp `just lint` checks. Commit them together.

**Why A.** It's the one that reads as an azulejo: the kind of tile you'd see on a Lisbon facade, not a pattern that just has the colours. It also holds up small. At 32 and 16 px it drops the border, buds and outlines (see "Small sizes") and stays a blue flower on a white tile. B makes a nice point about tiling, but at 16 px its four tiles blur into a checkerboard. C is the boldest at 16 px, but a blue square with a white blob reads as a generic badge in a row of Dock icons, and it loses the warm glaze. (This was written before D–H existed; A stays the default until the maintainer picks.)

## Liquid Glass, baked in

Without Xcode there's no `actool`, so the icon ships as a flat `AppIcon.icns`. The glass is drawn into it, following Apple's macOS icon template:

- **Shape and grid.** A 1024 px canvas, with an 824 × 824 rounded square centred in it (100 px margin). The corners are Apple's continuous curve (radius 185.4, the same Béziers UIKit uses), not a plain rounded rect. Everything inside is fully opaque.
- **Shadow.** The template's drop shadow: black at 50 %, blur 28, 12 px down. It's halved at 32 px and below, where it would only smudge.
- **Glaze.** A warm white gradient. The blue is multiplied into it over a faint blurred copy of itself, so the cobalt seems to sit in the glaze, not on it.
- **Glass.** A sheen over the top half, a soft specular bloom under the top edge, and a rim light that's brightest top-left and catches again bottom-right. A faint hairline inside the edge keeps a white tile's shape on a white background.

**Small sizes.** Each size is drawn at its own pixel size, not scaled down from 1024. From 128 px up you get every detail. At 64 px the corner petals and the ink bleed go and the lines get thicker. At 32 and 16 px the border, the buds and the petal outlines go too, and the rosette turns solid cobalt.

**macOS 26 doesn't jail it.** Tahoe shrinks icons that don't fill its squircle onto a grey plate. [`tahoe-check.png`](tahoe-check.png) shows what IconServices (`NSWorkspace.icon(forFile:)`) made of three bundles on Nov 4, 2026, on macOS 26. Left: the built `target/macos/Azulejo.app`, full size in the system squircle. Middle: a control bundle whose icon is a plain circle, shrunk onto the grey plate. Right: Terminal, for scale. (The control's ⃠ badge only means that bundle had no executable.) `CFBundleIconName` without an `Assets.car` falls back to `CFBundleIconFile` cleanly.

## Variants

- [`appearance-dark.png`](appearance-dark.png): a night glaze with the blue lifted, for the dark icon style.
- [`appearance-tinted.png`](appearance-tinted.png): a monochrome source of the kind macOS colours for the tinted style.

An `.icns` holds one appearance, so neither ships yet. macOS 26 derives its own dark and tinted versions of a flat icon.

## What Xcode would add

Icon Composer (with Xcode 26) saves a layered `.icon` document. `actool` compiles it into `Contents/Resources/Assets.car` under the name in `CFBundleIconName` (already `AppIcon`). Layered, the system draws the glass live: specular highlights that follow the light, refraction, translucency, and our own default, dark, clear and tinted appearances instead of derived ones. There's no supported way to write `Assets.car` without `actool`, and the Command Line Tools don't include it. The way in later:

1. Export the motif from `draw.swift` as layers: the glaze background as a gradient, the cobalt motif as a foreground layer without the baked sheen, rim or shadow. Build `AppIcon.icon` in Icon Composer and set the dark and tinted colours from the variants above.
2. In `bundle.sh` and `scripts/release/build-mac.sh`, when `xcrun --find actool` succeeds, compile it into `Contents/Resources` (along the lines of `xcrun actool AppIcon.icon --compile <Resources> --app-icon AppIcon --platform macosx --minimum-deployment-target 14.0 --output-partial-info-plist …`; check the flags against the Xcode you have). Keep `AppIcon.icns` as the fallback for macOS 14 and 15 and for machines without Xcode.

## Files and commands

- `scripts/icon/draw.swift`: the source. Pure CoreGraphics, plus CoreImage's software renderer for the blur, so output is byte-identical from run to run on the same macOS.
- `just icon` (`scripts/icon/build.sh`): renders the iconset (16–512 at @1x and @2x) and packs `apps/macos/Resources/AppIcon.icns` with `/usr/bin/iconutil`. It also writes these PNGs and `scripts/icon/AppIcon.sha256`.
- `just lint` runs `scripts/icon/build.sh --check`. It fails when `draw.swift` or the `.icns` changed without a `just icon`. It compares hashes and doesn't re-render, so a different macOS version's antialiasing can't trip it.
- `apps/macos/bundle.sh` (`just mac`) and `scripts/release/build-mac.sh` (`just release-mac`, `just dogfood`) copy the `.icns` into `Contents/Resources`. `Info.plist` sets `CFBundleIconFile` and `CFBundleIconName` to `AppIcon`.
