// Draws Azulejo's app icon: three candidates, the chosen one's dark and tinted
// variants, the .iconset that scripts/icon/build.sh packs into AppIcon.icns,
// and a contact sheet. Pure CoreGraphics, so it's reproducible on any Mac
// with the Command Line Tools (docs/design/icon/README.md).
//
//   swift scripts/icon/draw.swift --iconset DIR   # the 10 PNGs iconutil wants
//   swift scripts/icon/draw.swift --docs DIR      # candidates, variants, sheet.png
//
// Geometry follows Apple's macOS icon template: a 1024 px canvas, an 824 px
// rounded square (continuous corners, radius 185.4) centred in it, fully
// opaque inside, and the template's drop shadow (black 50 %, blur 28, 12 px
// down) in the margin. macOS 26 clips icons of that shape with its own mask,
// rather than shrinking them onto a grey "legacy" plate.

import AppKit
import CoreGraphics
import CoreImage
import Foundation
import ImageIO
import UniformTypeIdentifiers

// MARK: - Colour

func rgb(_ hex: UInt32, _ a: CGFloat = 1) -> CGColor {
  CGColor(
    srgbRed: CGFloat((hex >> 16) & 0xff) / 255, green: CGFloat((hex >> 8) & 0xff) / 255,
    blue: CGFloat(hex & 0xff) / 255, alpha: a)
}

/// One appearance of the icon. Roles, not hues: `ink` is the main cobalt,
/// `line` the darker outlines, `wash` the lighter blue fill, `grout` the
/// joints between tiles. White in a motif is bare ground (`punch`).
struct Palette {
  var groundTop: CGColor
  var groundBottom: CGColor
  var ink: CGColor
  var line: CGColor
  var wash: CGColor
  var grout: CGColor
  /// Blue soaks into a white glaze (multiply); light ink on a dark ground can't.
  var multiply: Bool
  var sheen: CGFloat  // top glaze sheen strength
  var shadow: CGFloat  // drop shadow opacity
}

let tinGlaze = Palette(
  groundTop: rgb(0xFBF8F1), groundBottom: rgb(0xEDE6D6),
  ink: rgb(0x1F4FA3), line: rgb(0x123578), wash: rgb(0x86A8DC),
  grout: rgb(0xDCD4C2),
  multiply: true, sheen: 0.55, shadow: 0.5)

/// Candidate C: the glaze and the cobalt swap places.
let cobaltGround = Palette(
  groundTop: rgb(0x2C63BE), groundBottom: rgb(0x16408E),
  ink: rgb(0xF7F2E6), line: rgb(0x0E2D6B), wash: rgb(0x8FB0E2),
  grout: rgb(0x173F88),
  multiply: false, sheen: 0.45, shadow: 0.5)

/// Dark appearance: a night glaze, the blue lifted so it still reads.
let dark = Palette(
  groundTop: rgb(0x262C3A), groundBottom: rgb(0x161A24),
  ink: rgb(0x5E93EC), line: rgb(0x9CC0FA), wash: rgb(0x34507F),
  grout: rgb(0x10131A),
  multiply: false, sheen: 0.22, shadow: 0.6)

/// Tinted appearance: a monochrome source; macOS colours it with the tint.
let tinted = Palette(
  groundTop: rgb(0x2A2A2C), groundBottom: rgb(0x1A1A1C),
  ink: rgb(0xEDEDED), line: rgb(0xFFFFFF), wash: rgb(0x77777A),
  grout: rgb(0x111112),
  multiply: false, sheen: 0.22, shadow: 0.6)

// MARK: - Shapes

/// Apple's continuous-corner rounded rect (the "squircle"), as UIKit and
/// Icon Composer draw it: each corner is three Béziers that start 1.528 r
/// from the corner, so curvature eases in instead of jumping.
func squircle(_ rect: CGRect, _ radius: CGFloat) -> CGPath {
  let r = min(radius, min(rect.width, rect.height) / 2 / 1.52866483)
  let p = CGMutablePath()
  // (a, b): distance from the corner along x, along y.
  let seq: [(CGFloat, CGFloat)] = [
    (1.08849323, 0), (0.86840689, 0), (0.63149399, 0.07491100),
    (0.37282392, 0.16905899), (0.16905899, 0.37282392), (0.07491100, 0.63149399),
    (0, 0.86840689), (0, 1.08849323), (0, 1.52866483),
  ]
  // Corners clockwise in y-up space from the top-left; each with its inward
  // directions and whether we arrive along x (horizontal edge) first.
  let corners: [(CGPoint, CGFloat, CGFloat, Bool)] = [
    (CGPoint(x: rect.maxX, y: rect.maxY), -1, -1, true),
    (CGPoint(x: rect.maxX, y: rect.minY), -1, 1, false),
    (CGPoint(x: rect.minX, y: rect.minY), 1, 1, true),
    (CGPoint(x: rect.minX, y: rect.maxY), 1, -1, false),
  ]
  p.move(to: CGPoint(x: rect.minX + 1.52866483 * r, y: rect.maxY))
  for (c, dx, dy, xFirst) in corners {
    func pt(_ ab: (CGFloat, CGFloat)) -> CGPoint {
      let (a, b) = xFirst ? ab : (ab.1, ab.0)
      return CGPoint(x: c.x + dx * a * r, y: c.y + dy * b * r)
    }
    p.addLine(to: pt((1.52866483, 0)))
    for i in stride(from: 0, to: 9, by: 3) {
      p.addCurve(to: pt(seq[i + 2]), control1: pt(seq[i]), control2: pt(seq[i + 1]))
    }
  }
  p.closeSubpath()
  return p
}

func disc(_ c: CGPoint, _ r: CGFloat) -> CGPath {
  CGPath(ellipseIn: CGRect(x: c.x - r, y: c.y - r, width: 2 * r, height: 2 * r), transform: nil)
}

/// A pointed petal from `from` outwards at `angle`, `length` long and
/// `width` across at its widest.
func petal(at from: CGPoint = .zero, angle: CGFloat, length l: CGFloat, width w: CGFloat)
  -> CGPath
{
  let p = CGMutablePath()
  let h = w / 2
  p.move(to: .zero)
  p.addCurve(
    to: CGPoint(x: l, y: 0), control1: CGPoint(x: l * 0.18, y: h * 1.25),
    control2: CGPoint(x: l * 0.72, y: h * 1.05))
  p.addCurve(
    to: .zero, control1: CGPoint(x: l * 0.72, y: -h * 1.05),
    control2: CGPoint(x: l * 0.18, y: -h * 1.25))
  p.closeSubpath()
  var t = CGAffineTransform(translationX: from.x, y: from.y).rotated(by: angle)
  return p.copy(using: &t)!
}

/// A diamond (square on its point) with half-diagonal `r`.
func diamond(_ c: CGPoint, _ r: CGFloat) -> CGPath {
  let p = CGMutablePath()
  p.addLines(between: [
    CGPoint(x: c.x, y: c.y + r), CGPoint(x: c.x + r, y: c.y), CGPoint(x: c.x, y: c.y - r),
    CGPoint(x: c.x - r, y: c.y),
  ])
  p.closeSubpath()
  return p
}

/// A four-pointed star with concave sides, points `r` out.
func star4(_ c: CGPoint, _ r: CGFloat, pinch: CGFloat = 0.28) -> CGPath {
  let p = CGMutablePath()
  let pts = (0..<4).map { i -> CGPoint in
    let a = CGFloat(i) * .pi / 2 + .pi / 2
    return CGPoint(x: c.x + r * cos(a), y: c.y + r * sin(a))
  }
  p.move(to: pts[0])
  for i in 0..<4 {
    p.addQuadCurve(to: pts[(i + 1) % 4], control: CGPoint(x: c.x + (pts[i].x + pts[(i + 1) % 4].x - 2 * c.x) * pinch, y: c.y + (pts[i].y + pts[(i + 1) % 4].y - 2 * c.y) * pinch))
  }
  p.closeSubpath()
  return p
}

let quarter: [CGFloat] = [0, .pi / 2, .pi, 3 * .pi / 2]
let diagonals: [CGFloat] = quarter.map { $0 + .pi / 4 }
let corners = [CGPoint(x: 1, y: 1), CGPoint(x: -1, y: 1), CGPoint(x: -1, y: -1), CGPoint(x: 1, y: -1)]
let edgeMids = [CGPoint(x: 1, y: 0), CGPoint(x: 0, y: 1), CGPoint(x: -1, y: 0), CGPoint(x: 0, y: -1)]

// MARK: - Motifs
//
// Motifs draw in tile space: the 824 px rounded square spans -1...1 on both
// axes. `lod` is the level of detail: 0 from 128 px up, 1 at 64 px, 2 at 32 px
// and below, where fine lines would turn to mush.

extension CGContext {
  func fill(_ path: CGPath, _ color: CGColor) {
    setFillColor(color)
    addPath(path)
    fillPath()
  }
  func stroke(_ path: CGPath, _ color: CGColor, _ width: CGFloat) {
    setStrokeColor(color)
    setLineWidth(width)
    setLineJoin(.round)
    addPath(path)
    strokePath()
  }
  /// Cuts `path` out of what's drawn so far in the motif's layer, so the
  /// glaze shows through: white in a motif is bare glaze, as on a real tile.
  func punch(_ path: CGPath) {
    saveGState()
    setBlendMode(.destinationOut)
    fill(path, rgb(0x000000))
    restoreGState()
  }
}

enum Candidate: String, CaseIterable {
  case classic = "a-classic"
  case cluster = "b-cluster"
  case minimal = "c-minimal"

  var title: String {
    switch self {
    case .classic: "A · classic tile"
    case .cluster: "B · 2×2 cluster"
    case .minimal: "C · minimal, cobalt glaze"
    }
  }
  var palette: Palette { self == .minimal ? cobaltGround : tinGlaze }
}

/// The default: see docs/design/icon/README.md for why.
let chosen = Candidate.classic

/// A: one tile. Rosette in the middle, quarter medallions in the corners that
/// complete with the neighbouring tiles, half-drops at the edge midpoints, a
/// fine border.
func drawClassic(_ g: CGContext, _ p: Palette, lod: Int) {
  // Fine border just inside the tile edge, strung through the edge buds and
  // into the corner medallions.
  if lod < 2 {
    let inset: CGFloat = 0.1
    let border = CGMutablePath()
    border.addRect(CGRect(x: -1 + inset, y: -1 + inset, width: 2 - 2 * inset, height: 2 - 2 * inset))
    g.stroke(border, p.ink, lod == 0 ? 0.028 : 0.05)
  }
  // Corner medallions: quarters of a flower shared by four tiles, three
  // petals fanning in from each corner over a wash.
  for (i, c) in corners.enumerated() {
    let inward = diagonals[i] + .pi
    if lod == 2 {
      g.fill(disc(c, 0.5), p.ink)
      continue
    }
    g.punch(disc(c, 0.64))
    g.fill(disc(c, 0.6), p.ink)
    g.punch(disc(c, 0.52))
    g.fill(disc(c, 0.46), p.wash)
    if lod == 0 {
      g.stroke(disc(c, 0.46), p.line, 0.03)
      for d in [-0.5, 0, 0.5] as [CGFloat] {
        g.fill(petal(at: c, angle: inward + d, length: 0.42, width: 0.14), p.ink)
      }
    }
  }
  // Buds at the edge midpoints: halves that meet the neighbour's.
  if lod < 2 {
    for (i, m) in edgeMids.enumerated() {
      g.fill(petal(at: m, angle: quarter[i] + .pi, length: 0.26, width: 0.22), p.ink)
    }
  }
  // Rosette: four broad petals on the axes, four narrow ones on the diagonals.
  for a in diagonals {
    g.fill(petal(angle: a, length: 0.58, width: 0.2), p.ink)
  }
  for a in quarter {
    let leaf = petal(angle: a, length: 0.74, width: lod == 2 ? 0.42 : 0.36)
    if lod < 2 { g.punch(leaf) }
    g.fill(leaf, lod == 2 ? p.ink : p.wash)
    if lod < 2 { g.stroke(leaf, p.line, lod == 0 ? 0.035 : 0.05) }
  }
  g.fill(disc(.zero, 0.2), p.line)
  g.punch(disc(.zero, lod == 2 ? 0.09 : 0.08))
}

/// B: four tiles, alternating cobalt and white like the Alfama pattern in the
/// mockup's browser tile, with joints echoing the app's split layout. The
/// quarter discs at each tile's corners meet across the joints.
func drawCluster(_ g: CGContext, _ p: Palette, lod: Int) {
  let quads: [(CGPoint, Bool)] = [
    (CGPoint(x: -0.5, y: 0.5), true), (CGPoint(x: 0.5, y: 0.5), false),
    (CGPoint(x: -0.5, y: -0.5), false), (CGPoint(x: 0.5, y: -0.5), true),
  ]
  for (c, blue) in quads {
    g.saveGState()
    g.clip(to: CGRect(x: c.x - 0.5, y: c.y - 0.5, width: 1, height: 1))
    // On a blue tile the figure is bare glaze; on a white one it's cobalt.
    let figure: (CGPath) -> Void = blue ? { g.punch($0) } : { g.fill($0, p.ink) }
    let ground: (CGPath) -> Void = blue ? { g.fill($0, p.ink) } : { g.punch($0) }
    if blue {
      g.fill(CGPath(rect: CGRect(x: c.x - 0.5, y: c.y - 0.5, width: 1, height: 1), transform: nil), p.ink)
    }
    for k in corners {
      figure(disc(CGPoint(x: c.x + k.x * 0.5, y: c.y + k.y * 0.5), 0.17))
    }
    figure(disc(c, 0.33))
    if lod < 2 {
      ground(diamond(c, 0.235))
      figure(disc(c, 0.085))
    }
    if lod == 0 && !blue {
      // The lighter blue a painter outlines with, round the white tiles' diamonds.
      g.stroke(diamond(c, 0.235), p.wash, 0.03)
    }
    g.restoreGState()
  }
  // Joints between the four tiles: bare glaze with a little grout in them.
  let w: CGFloat = lod == 2 ? 0.075 : 0.05
  let joints = CGMutablePath()
  joints.addRect(CGRect(x: -w / 2, y: -1.2, width: w, height: 2.4))
  joints.addRect(CGRect(x: -1.2, y: -w / 2, width: 2.4, height: w))
  g.punch(joints)
  if lod < 2 {
    let hair = CGMutablePath()
    hair.addRect(CGRect(x: -w / 6, y: -1.2, width: w / 3, height: 2.4))
    hair.addRect(CGRect(x: -1.2, y: -w / 6, width: 2.4, height: w / 3))
    g.fill(hair, p.grout)
  }
}

/// C: cobalt glaze, one white quatrefoil, a star at its heart and pale
/// quarter rings in the corners. Fewest shapes; boldest at 16 px.
func drawMinimal(_ g: CGContext, _ p: Palette, lod: Int) {
  for c in corners {
    g.fill(disc(c, 0.5), p.wash)
    if lod < 2 { g.punch(disc(c, 0.34)) }
  }
  let lobes = CGMutablePath()
  for m in edgeMids {
    lobes.addPath(disc(CGPoint(x: m.x * 0.33, y: m.y * 0.33), 0.36))
  }
  lobes.addPath(disc(.zero, 0.4))
  g.fill(lobes, p.ink)
  g.punch(star4(.zero, 0.5, pinch: 0.12))
  g.fill(disc(.zero, lod == 2 ? 0.1 : 0.08), p.ink)
}

// MARK: - Glass

func bitmap(_ px: Int) -> CGContext {
  CGContext(
    data: nil, width: px, height: px, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
}

/// The motif alone, on transparency, in its own bitmap: its knock-outs
/// (`punch`) only cut the motif, never the glaze under it.
func motifLayer(_ candidate: Candidate, _ p: Palette, px: Int, lod: Int) -> CGImage {
  let g = bitmap(px)
  let s = CGFloat(px) / 1024
  g.scaleBy(x: s, y: s)
  g.addPath(squircle(CGRect(x: 100, y: 100, width: 824, height: 824), 185.4))
  g.clip()
  g.translateBy(x: 512, y: 512)
  g.scaleBy(x: 412, y: 412)
  switch candidate {
  case .classic: drawClassic(g, p, lod: lod)
  case .cluster: drawCluster(g, p, lod: lod)
  case .minimal: drawMinimal(g, p, lod: lod)
  }
  return g.makeImage()!
}

/// Gaussian blur on the CPU renderer, so the output doesn't depend on the GPU.
func blurred(_ image: CGImage, sigma: CGFloat) -> CGImage {
  let sRGB = CGColorSpace(name: CGColorSpace.sRGB)!
  let ctx = CIContext(options: [.useSoftwareRenderer: true, .workingColorSpace: sRGB, .outputColorSpace: sRGB])
  let input = CIImage(cgImage: image)
  let out = input.clampedToExtent().applyingGaussianBlur(sigma: Double(sigma)).cropped(to: input.extent)
  return ctx.createCGImage(out, from: input.extent)!
}

/// Renders one icon at `px` × `px` pixels.
func render(_ candidate: Candidate, palette p: Palette, px: Int) -> CGImage {
  let g = CGContext(
    data: nil, width: px, height: px, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
  let s = CGFloat(px) / 1024
  let lod = px <= 32 ? 2 : (px <= 64 ? 1 : 0)
  g.scaleBy(x: s, y: s)
  g.interpolationQuality = .high

  let body = CGRect(x: 100, y: 100, width: 824, height: 824)
  let shape = squircle(body, 185.4)

  // Drop shadow (Apple's template: black 50 %, blur 28, 12 down). At 16 px
  // it's a smudge, so it fades out with size.
  g.saveGState()
  let shadowScale: CGFloat = px <= 32 ? 0.5 : 1
  g.setShadow(
    offset: CGSize(width: 0, height: -12 * s), blur: 28 * s,
    color: rgb(0x000000, p.shadow * shadowScale))
  g.fill(shape, p.groundBottom)
  g.restoreGState()

  g.saveGState()
  g.addPath(shape)
  g.clip()

  // Glaze: a soft vertical gradient.
  let cs = CGColorSpace(name: CGColorSpace.sRGB)!
  g.drawLinearGradient(
    CGGradient(colorsSpace: cs, colors: [p.groundTop, p.groundBottom] as CFArray, locations: [0, 1])!,
    start: CGPoint(x: 512, y: 924), end: CGPoint(x: 512, y: 100), options: [])

  // The pattern. On a white glaze the blue is multiplied in, over a faint
  // blurred copy of itself: cobalt bleeding into tin glaze.
  let art = motifLayer(candidate, p, px: px, lod: lod)
  let canvas = CGRect(x: 0, y: 0, width: 1024, height: 1024)
  g.saveGState()
  if p.multiply { g.setBlendMode(.multiply) }
  if lod == 0 {
    g.setAlpha(0.45)
    g.draw(blurred(art, sigma: 5 * s), in: canvas)
    g.setAlpha(1)
  }
  g.draw(art, in: canvas)
  g.restoreGState()

  // The glass over it. A sheen across the top half...
  g.setBlendMode(.normal)
  g.drawLinearGradient(
    CGGradient(
      colorsSpace: cs,
      colors: [rgb(0xFFFFFF, 0.42 * p.sheen), rgb(0xFFFFFF, 0.08 * p.sheen), rgb(0xFFFFFF, 0), rgb(0x000000, 0.10)] as CFArray,
      locations: [0, 0.38, 0.62, 1])!,
    start: CGPoint(x: 512, y: 924), end: CGPoint(x: 512, y: 100), options: [])
  // ...a soft specular bloom under the top edge...
  g.saveGState()
  g.translateBy(x: 512, y: 880)
  g.scaleBy(x: 1, y: 0.16)
  g.drawRadialGradient(
    CGGradient(
      colorsSpace: cs, colors: [rgb(0xFFFFFF, 0.55 * p.sheen), rgb(0xFFFFFF, 0)] as CFArray,
      locations: [0, 1])!,
    startCenter: .zero, startRadius: 0, endCenter: .zero, endRadius: 360, options: [])
  g.restoreGState()
  // ...and a rim of light, brightest top-left, catching again bottom-right.
  g.addPath(shape)
  g.setLineWidth(lod == 2 ? 28 : 14)
  g.replacePathWithStrokedPath()
  g.clip()
  g.drawLinearGradient(
    CGGradient(
      colorsSpace: cs,
      colors: [rgb(0xFFFFFF, 0.95), rgb(0xFFFFFF, 0.15), rgb(0xFFFFFF, 0.08), rgb(0xFFFFFF, 0.45)] as CFArray,
      locations: [0, 0.4, 0.6, 1])!,
    start: CGPoint(x: 160, y: 900), end: CGPoint(x: 870, y: 120), options: [])
  g.restoreGState()

  // A hairline just inside the edge so a white tile holds its shape on a
  // white background.
  g.saveGState()
  g.addPath(shape)
  g.clip()
  g.addPath(shape)
  g.setLineWidth(lod == 2 ? 24 : 4)
  g.setStrokeColor(rgb(0x000000, p.multiply ? 0.10 : 0.18))
  g.strokePath()
  g.restoreGState()

  return g.makeImage()!
}

// MARK: - Output

func writePNG(_ image: CGImage, _ url: URL) {
  let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)!
  CGImageDestinationAddImage(dest, image, nil)
  guard CGImageDestinationFinalize(dest) else { fatalError("can't write \(url.path)") }
}

/// iconutil's names: (file, pixels).
let iconsetSizes: [(String, Int)] = [16, 32, 128, 256, 512].flatMap { pt in
  [("icon_\(pt)x\(pt).png", pt), ("icon_\(pt)x\(pt)@2x.png", pt * 2)]
}

func writeIconset(_ dir: URL) {
  try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
  for (name, px) in iconsetSizes {
    writePNG(render(chosen, palette: chosen.palette, px: px), dir.appendingPathComponent(name))
  }
}

// MARK: - Contact sheet

func label(_ g: CGContext, _ text: String, at p: CGPoint, size: CGFloat, color: NSColor, bold: Bool = false) {
  NSGraphicsContext.saveGraphicsState()
  NSGraphicsContext.current = NSGraphicsContext(cgContext: g, flipped: false)
  let font = bold ? NSFont.systemFont(ofSize: size, weight: .semibold) : NSFont.systemFont(ofSize: size)
  NSAttributedString(string: text, attributes: [.font: font, .foregroundColor: color]).draw(at: p)
  NSGraphicsContext.restoreGraphicsState()
}

func drawNSImage(_ g: CGContext, _ image: NSImage, in rect: CGRect) {
  NSGraphicsContext.saveGraphicsState()
  NSGraphicsContext.current = NSGraphicsContext(cgContext: g, flipped: false)
  image.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: false, hints: [.interpolation: NSImageInterpolation.high])
  NSGraphicsContext.restoreGraphicsState()
}

/// Rows: each candidate, the chosen one's dark and tinted variants, then
/// system apps for scale. Columns: the 1024 master (shown at 512), 128, 64,
/// 32, 16 at their real pixel sizes, then 32 and 16 blown up 4× (nearest
/// neighbour) to judge the pixels. Light background left, dark right.
func writeSheet(_ url: URL) {
  struct Row { let name: String; let images: (Int) -> CGImage? ; let ns: NSImage? }
  var rows: [Row] = Candidate.allCases.map { c in
    Row(name: c.title + (c == chosen ? "  (default)" : ""), images: { render(c, palette: c.palette, px: $0) }, ns: nil)
  }
  rows.append(Row(name: chosen.title + " · dark", images: { render(chosen, palette: dark, px: $0) }, ns: nil))
  rows.append(Row(name: chosen.title + " · tinted (mono source)", images: { render(chosen, palette: tinted, px: $0) }, ns: nil))
  let system = [
    "/System/Applications/Utilities/Terminal.app", "/System/Applications/Notes.app",
    "/System/Applications/Freeform.app", "/System/Applications/System Settings.app",
  ].filter { FileManager.default.fileExists(atPath: $0) }
  for path in system {
    rows.append(Row(name: "macOS: " + (path as NSString).lastPathComponent.replacingOccurrences(of: ".app", with: ""), images: { _ in nil }, ns: NSWorkspace.shared.icon(forFile: path)))
  }

  let sizes = [512, 128, 64, 32, 16]  // display sizes; the first is the 1024 master at half
  let gap = 28
  let half = sizes.reduce(0, +) + gap * (sizes.count + 2) + 128 + 64 + 40
  let labelH = 44
  let rowH = 512 + gap + labelH
  let width = half * 2
  let height = rows.count * rowH + 90
  let g = CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
  // Backgrounds: macOS 26's light and dark Dock-ish greys.
  let backgrounds = [(rgb(0xECECEF), NSColor(white: 0.15, alpha: 1)), (rgb(0x1E1F23), NSColor(white: 0.85, alpha: 1))]
  for (i, (bg, _)) in backgrounds.enumerated() {
    g.setFillColor(bg)
    g.fill(CGRect(x: i * half, y: 0, width: half, height: height))
  }
  for (i, (_, ink)) in backgrounds.enumerated() {
    let ox = CGFloat(i * half + gap)
    label(g, "Azulejo app icon · \(i == 0 ? "light" : "dark") background · 1024 shown at 512, then 128 / 64 / 32 / 16 at 1:1, then 32 and 16 at 4×", at: CGPoint(x: ox, y: CGFloat(height - 52)), size: 22, color: ink, bold: true)
    for (r, row) in rows.enumerated() {
      let top = CGFloat(height - 90 - r * rowH)
      label(g, row.name, at: CGPoint(x: ox, y: top - 34), size: 20, color: ink, bold: true)
      var x = ox
      let base = top - CGFloat(labelH) - 512
      for (k, d) in sizes.enumerated() {
        let px = k == 0 ? 1024 : d
        let rect = CGRect(x: x, y: base, width: CGFloat(d), height: CGFloat(d))
        if let img = row.images(px) {
          g.interpolationQuality = .high
          g.draw(img, in: rect)
        } else if let ns = row.ns {
          drawNSImage(g, ns, in: rect)
        }
        x += CGFloat(d + gap)
      }
      // Pixel zoom of the real 32 and 16 px renders.
      for d in [32, 16] {
        let rect = CGRect(x: x, y: base, width: CGFloat(d * 4), height: CGFloat(d * 4))
        g.interpolationQuality = .none
        if let img = row.images(d) {
          g.draw(img, in: rect)
        } else if let ns = row.ns {
          // Rasterise at the real size first, then zoom.
          let small = CGContext(data: nil, width: d, height: d, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
          drawNSImage(small, ns, in: CGRect(x: 0, y: 0, width: d, height: d))
          g.draw(small.makeImage()!, in: rect)
        }
        x += CGFloat(d * 4 + gap)
      }
    }
  }
  writePNG(g.makeImage()!, url)
}

func writeDocs(_ dir: URL) {
  try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
  for c in Candidate.allCases {
    writePNG(render(c, palette: c.palette, px: 1024), dir.appendingPathComponent("candidate-\(c.rawValue).png"))
  }
  writePNG(render(chosen, palette: dark, px: 1024), dir.appendingPathComponent("appearance-dark.png"))
  writePNG(render(chosen, palette: tinted, px: 1024), dir.appendingPathComponent("appearance-tinted.png"))
  writeSheet(dir.appendingPathComponent("sheet.png"))
}

// MARK: - Main

var args = Array(CommandLine.arguments.dropFirst())
if args.isEmpty {
  FileHandle.standardError.write("usage: draw.swift [--iconset DIR] [--docs DIR]\n".data(using: .utf8)!)
  exit(2)
}
while !args.isEmpty {
  let flag = args.removeFirst()
  guard let path = args.first else { fatalError("\(flag) needs a directory") }
  args.removeFirst()
  let url = URL(fileURLWithPath: path)
  switch flag {
  case "--iconset": writeIconset(url)
  case "--docs": writeDocs(url)
  default: fatalError("unknown option \(flag)")
  }
}
