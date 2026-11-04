// Draws Azulejo's app icon: eight candidates (A–H), the chosen one's dark and
// tinted variants, the .iconset that scripts/icon/build.sh packs into
// AppIcon.icns, a contact sheet and a side-by-side of the candidates. Pure CoreGraphics, so it's reproducible on any Mac
// with the Command Line Tools (docs/design/icon/README.md).
//
//   swift scripts/icon/draw.swift --iconset DIR   # the 10 PNGs iconutil wants
//   swift scripts/icon/draw.swift --docs DIR      # candidates, variants, sheet.png, options.png
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
  /// A second pigment, for the candidates that use one (D's ochre, H's gold).
  var accent: CGColor = rgb(0xE0A42A)
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

/// Candidate F: a pale sky glaze, so the glass tiles over it have something
/// to be translucent against. Not multiplied: frosted white has to stay white.
let skyGlaze = Palette(
  groundTop: rgb(0xEEF3FB), groundBottom: rgb(0xB9CDEB),
  ink: rgb(0x1F4FA3), line: rgb(0x123578), wash: rgb(0x86A8DC),
  grout: rgb(0xDCD4C2),
  multiply: false, sheen: 0.5, shadow: 0.5)

/// Candidate H: an ink-blue night glaze with white and pale gold on it.
let nightGlaze = Palette(
  groundTop: rgb(0x1C2D66), groundBottom: rgb(0x0B1435),
  ink: rgb(0xF6F1E4), line: rgb(0x0B1435), wash: rgb(0x3B5BA8),
  grout: rgb(0x0A1230),
  multiply: false, sheen: 0.32, shadow: 0.55, accent: rgb(0xE8CB7E))

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

/// An eight-pointed star: two squares, one turned 45°, as one outline.
func octagram(_ c: CGPoint, _ r: CGFloat) -> CGPath {
  let p = CGMutablePath()
  let inner = r * cos(.pi / 4) / cos(.pi / 8)
  p.addLines(between: (0..<16).map { i -> CGPoint in
    let a = CGFloat(i) * .pi / 8
    let d = i % 2 == 0 ? r : inner
    return CGPoint(x: c.x + d * cos(a), y: c.y + d * sin(a))
  })
  p.closeSubpath()
  return p
}

/// An oval petal centred `at` from the origin along `angle`.
func oval(angle: CGFloat, at r: CGFloat, length l: CGFloat, width w: CGFloat) -> CGPath {
  var t = CGAffineTransform(rotationAngle: angle)
  return CGPath(ellipseIn: CGRect(x: r - l / 2, y: -w / 2, width: l, height: w), transform: &t)
}

/// A polyline turned into a fillable outline, round caps and joins.
func strokeShape(_ points: [CGPoint], _ width: CGFloat) -> CGPath {
  let p = CGMutablePath()
  p.addLines(between: points)
  return p.copy(strokingWithWidth: width, lineCap: .round, lineJoin: .round, miterLimit: 4)
}

/// A circle's outline as a fillable ring.
func ring(_ c: CGPoint, _ r: CGFloat, _ width: CGFloat) -> CGPath {
  disc(c, r).copy(strokingWithWidth: width, lineCap: .butt, lineJoin: .round, miterLimit: 4)
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
  case ochre = "d-ochre"
  case prompt = "e-prompt"
  case glass = "f-glass"
  case split = "g-split"
  case night = "h-night"

  /// The contact sheet's row title (sheet.png keeps A–C as they were).
  var title: String {
    switch self {
    case .classic: "A · classic tile"
    case .cluster: "B · 2×2 cluster"
    case .minimal: "C · minimal, cobalt glaze"
    default: letter + " · " + name
    }
  }
  var letter: String { String(rawValue.prefix(1)).uppercased() }
  /// Two to four words, for options.png.
  var name: String {
    switch self {
    case .classic: "Classic tile"
    case .cluster: "2×2 cluster"
    case .minimal: "Minimal cobalt"
    case .ochre: "Lisbon blue & ochre"
    case .prompt: "Prompt rosette"
    case .glass: "Glass stack"
    case .split: "Split layout"
    case .night: "Night glaze"
    }
  }
  var palette: Palette {
    switch self {
    case .minimal: cobaltGround
    case .glass: skyGlaze
    case .night: nightGlaze
    default: tinGlaze
    }
  }
  /// Whether the ink bleeds (a faint blurred copy under the motif). F's glass
  /// draws its own depth instead.
  var bleeds: Bool { self != .glass }
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

/// D: a Lisbon pattern tile in two pigments. A cobalt diamond holds an ochre
/// eight-pointed star; ochre quarter discs in the corners and half-buds at the
/// edges join up with the neighbouring tiles.
func drawOchre(_ g: CGContext, _ p: Palette, lod: Int) {
  for c in corners {
    g.fill(disc(c, 0.5), p.accent)
    if lod < 2 {
      g.fill(ring(c, 0.5, lod == 0 ? 0.04 : 0.06), p.ink)
      g.fill(disc(c, 0.24), p.ink)
    }
    if lod == 0 {
      g.punch(ring(c, 0.36, 0.035))
      g.punch(disc(c, 0.1))
    }
  }
  if lod < 2 {
    for m in edgeMids {
      g.fill(disc(m, 0.13), p.accent)
      g.fill(ring(m, 0.13, lod == 0 ? 0.03 : 0.05), p.ink)
    }
  }
  g.fill(diamond(.zero, 0.86), p.ink)
  if lod < 2 {
    // A white line inside the diamond, and white petals on its axes.
    let inner = diamond(.zero, 0.76)
    g.punch(inner.copy(strokingWithWidth: lod == 0 ? 0.03 : 0.045, lineCap: .butt, lineJoin: .miter, miterLimit: 4))
  }
  if lod == 0 {
    for a in quarter {
      g.punch(petal(at: CGPoint(x: 0.54 * cos(a), y: 0.54 * sin(a)), angle: a, length: 0.17, width: 0.09))
    }
  }
  let star = octagram(.zero, lod == 2 ? 0.56 : 0.5)
  g.fill(star, p.accent)
  if lod < 2 { g.stroke(star, p.line, lod == 0 ? 0.025 : 0.04) }
  g.fill(disc(.zero, lod == 2 ? 0.2 : 0.19), p.ink)
  g.punch(disc(.zero, lod == 2 ? 0.08 : 0.1))
  if lod == 0 { g.fill(disc(.zero, 0.045), p.accent) }
}

/// E: a tile first. A scalloped cobalt medallion in a ring of beads, with the
/// prompt, `>_`, left in bare glaze at its heart; in the corners, pale-blue
/// quarters with chevrons pointing into them, which meet the next tile's.
func drawPrompt(_ g: CGContext, _ p: Palette, lod: Int) {
  for c in corners {
    if lod == 2 {
      g.fill(disc(c, 0.5), p.wash)
      continue
    }
    g.fill(disc(c, 0.48), p.wash)
    g.fill(ring(c, 0.48, lod == 0 ? 0.045 : 0.06), p.ink)
    if lod == 0 { g.fill(disc(c, 0.3), p.ink) }
  }
  if lod == 0 {
    for i in 0..<16 {
      let a = CGFloat(i) * .pi / 8 + .pi / 16
      g.fill(disc(CGPoint(x: 0.82 * cos(a), y: 0.82 * sin(a)), 0.035), p.ink)
    }
  }
  if lod < 2 {
    for (i, m) in edgeMids.enumerated() {
      g.fill(petal(at: m, angle: quarter[i] + .pi, length: 0.12, width: 0.16), p.ink)
    }
  }
  // The medallion: a disc on an eight-petal flower, long petals on the axes.
  if lod < 2 {
    for a in quarter {
      g.fill(petal(angle: a, length: 0.8, width: 0.42), p.wash)
      g.stroke(petal(angle: a, length: 0.8, width: 0.42), p.ink, lod == 0 ? 0.03 : 0.045)
    }
    for a in diagonals {
      g.fill(petal(angle: a, length: 0.74, width: 0.3), p.ink)
    }
  }
  g.fill(disc(.zero, lod == 2 ? 0.66 : 0.53), p.ink)
  if lod == 0 { g.punch(ring(.zero, 0.46, 0.022)) }
  // The prompt, in bare glaze.
  let w: CGFloat = lod == 2 ? 0.19 : (lod == 1 ? 0.12 : 0.1)
  let k: CGFloat = lod == 2 ? 1.2 : 1  // bigger and bolder when small
  let h: CGFloat = 0.2 * k
  let glyph = CGMutablePath()
  glyph.addPath(strokeShape([CGPoint(x: -0.29 * k, y: h), CGPoint(x: -0.05 * k, y: 0), CGPoint(x: -0.29 * k, y: -h)], w))
  glyph.addPath(strokeShape([CGPoint(x: 0.07 * k, y: -h), CGPoint(x: 0.3 * k, y: -h)], w))
  g.punch(glyph)
}

/// F: three azulejo tiles stacked in depth, glass over glass. The back two are
/// tinted panes, the front one frosted: each blurs what's behind it, casts its
/// own shadow and catches the light along its rim.
func drawGlass(_ g: CGContext, _ p: Palette, lod: Int) {
  let unit = g.ctm.a / 412  // device pixels per 1024-canvas pixel
  let side: CGFloat = 0.98
  let step: CGFloat = 0.3
  let cs = CGColorSpace(name: CGColorSpace.sRGB)!

  func pane(_ c: CGPoint, tint: CGColor, motif: () -> Void) {
    let rect = CGRect(x: c.x - side / 2, y: c.y - side / 2, width: side, height: side)
    let shape = squircle(rect, side * 0.2)
    // What's behind, before the pane's own shadow lands on it.
    let behind = g.makeImage()!
    // Shadow only: draw the pane opaque with a shadow, then cut the pane out.
    g.saveGState()
    g.setShadow(offset: CGSize(width: 0, height: -14 * unit), blur: 34 * unit, color: rgb(0x0B1E4A, 0.38))
    g.fill(shape, rgb(0x000000))
    g.restoreGState()
    g.punch(shape)
    // Frosting: what's behind, blurred, then the pane's tint.
    g.saveGState()
    g.addPath(shape)
    g.clip()
    g.saveGState()
    g.concatenate(g.ctm.inverted())
    g.draw(blurred(behind, sigma: 16 * unit), in: CGRect(x: 0, y: 0, width: g.width, height: g.height))
    g.restoreGState()
    g.fill(shape, tint)
    g.drawLinearGradient(
      CGGradient(
        colorsSpace: cs, colors: [rgb(0xFFFFFF, 0.35), rgb(0xFFFFFF, 0.05), rgb(0xFFFFFF, 0)] as CFArray,
        locations: [0, 0.5, 1])!,
      start: CGPoint(x: rect.minX, y: rect.maxY), end: CGPoint(x: rect.maxX, y: rect.minY), options: [])
    motif()
    g.restoreGState()
    // Rim light: bright top-left, faint, then catching again bottom-right.
    g.saveGState()
    g.addPath(shape)
    g.setLineWidth(lod == 2 ? 0.06 : 0.025)
    g.replacePathWithStrokedPath()
    g.clip()
    g.drawLinearGradient(
      CGGradient(
        colorsSpace: cs,
        colors: [rgb(0xFFFFFF, 0.95), rgb(0xFFFFFF, 0.2), rgb(0xFFFFFF, 0.1), rgb(0xFFFFFF, 0.6)] as CFArray,
        locations: [0, 0.45, 0.6, 1])!,
      start: CGPoint(x: rect.minX, y: rect.maxY), end: CGPoint(x: rect.maxX, y: rect.minY), options: [])
    g.restoreGState()
  }

  let back = CGPoint(x: -step, y: step)
  let front = CGPoint(x: step, y: -step)
  func quarters(_ c: CGPoint, _ r: CGFloat, _ color: CGColor) {
    for k in corners {
      g.fill(disc(CGPoint(x: c.x + k.x * side / 2, y: c.y + k.y * side / 2), r), color)
    }
  }
  // Back: pale blue, white corner quarters and a ring.
  pane(back, tint: rgb(0x86A8DC, lod == 2 ? 0.9 : 0.72)) {
    if lod < 2 {
      quarters(back, 0.15, rgb(0xFFFFFF, 0.75))
      g.fill(ring(back, 0.26, 0.05), rgb(0xFFFFFF, 0.75))
    }
  }
  // Middle: cobalt, white corner quarters and a four-petal flower (mostly
  // seen blurred through the front pane).
  pane(.zero, tint: rgb(0x1F4FA3, lod == 2 ? 0.95 : 0.84)) {
    if lod < 2 {
      quarters(.zero, 0.15, rgb(0xFFFFFF, 0.9))
      for a in diagonals {
        g.fill(petal(angle: a, length: 0.36, width: 0.17), rgb(0xFFFFFF, 0.9))
      }
    }
  }
  // Front: frosted white, a cobalt rosette and corner quarters.
  pane(front, tint: rgb(0xFFFFFF, lod == 2 ? 0.78 : 0.6)) {
    quarters(front, lod == 2 ? 0.2 : 0.17, p.ink)
    if lod == 2 {
      // Small, a four-petal rosette is a "+" button. A diamond with a white
      // heart stays a tile.
      g.fill(diamond(front, 0.27), p.ink)
      g.fill(disc(front, 0.08), rgb(0xFFFFFF))
      return
    }
    for a in quarter {
      g.fill(petal(at: front, angle: a, length: 0.34, width: 0.15), p.ink)
    }
    for a in diagonals {
      g.fill(petal(at: front, angle: a, length: 0.22, width: 0.09), p.wash)
    }
    g.fill(disc(front, 0.075), p.line)
  }
}

/// G: one tile split by grout like the app's Tile layout, master on the left,
/// a stack of two on the right. One rosette runs across all three panels and
/// changes glaze with each: white on the cobalt master, cobalt on the white
/// pane, dark blue on the pale-blue one.
func drawSplit(_ g: CGContext, _ p: Palette, lod: Int) {
  let panels: [(CGRect, CGColor?, CGColor?)] = [
    (CGRect(x: -1.2, y: -1.2, width: 1.2, height: 2.4), p.ink, nil),  // master: the figure is bare glaze
    (CGRect(x: 0, y: 0, width: 1.2, height: 1.2), nil, p.ink),
    (CGRect(x: 0, y: -1.2, width: 1.2, height: 1.2), p.wash, p.line),
  ]
  for (rect, ground, ink) in panels {
    g.saveGState()
    g.clip(to: rect)
    if let ground { g.fill(CGPath(rect: rect, transform: nil), ground) }
    let figure: (CGPath) -> Void = { path in
      if let ink { g.fill(path, ink) } else { g.punch(path) }
    }
    for c in corners {
      figure(disc(c, lod == 2 ? 0.36 : 0.3))
      if lod < 2 { figure(ring(c, 0.44, lod == 0 ? 0.035 : 0.05)) }
    }
    // The rosette, centred where the three panels meet. Small, its diagonal
    // petals read as an "×", so it's one disc split three ways.
    if lod == 2 {
      figure(disc(.zero, 0.46))
    } else {
      figure(ring(.zero, 0.7, lod == 0 ? 0.035 : 0.05))
      for a in diagonals {
        figure(petal(angle: a, length: 0.62, width: 0.27))
      }
      for a in quarter {
        figure(petal(angle: a, length: 0.44, width: 0.14))
      }
      figure(disc(.zero, 0.16))
    }
    if lod == 0 {
      for m in edgeMids { figure(disc(m, 0.07)) }
    }
    g.restoreGState()
  }
  // Grout: bare glaze joints with a hairline of grout in them.
  let w: CGFloat = lod == 2 ? 0.09 : 0.055
  let joints = CGMutablePath()
  joints.addRect(CGRect(x: -w / 2, y: -1.2, width: w, height: 2.4))
  joints.addRect(CGRect(x: 0, y: -w / 2, width: 1.2, height: w))
  g.punch(joints)
  if lod < 2 {
    let hair = CGMutablePath()
    hair.addRect(CGRect(x: -w / 6, y: -1.2, width: w / 3, height: 2.4))
    hair.addRect(CGRect(x: w / 6, y: -w / 6, width: 1.2, height: w / 3))
    g.fill(hair, p.grout)
  }
}

/// H: a night glaze. A white daisy of eight round petals with a gold heart,
/// a ring of gold beads, white quarter rings round gold quarter discs in the
/// corners.
func drawNight(_ g: CGContext, _ p: Palette, lod: Int) {
  for c in corners {
    g.fill(disc(c, lod == 2 ? 0.34 : 0.24), p.accent)
    if lod < 2 { g.fill(ring(c, 0.42, lod == 0 ? 0.06 : 0.08), p.ink) }
    if lod == 0 { g.fill(ring(c, 0.56, 0.025), p.accent) }
  }
  if lod == 0 {
    for i in 0..<24 {
      let a = CGFloat(i) * .pi / 12
      g.fill(disc(CGPoint(x: 0.76 * cos(a), y: 0.76 * sin(a)), 0.028), p.accent)
    }
  } else if lod == 1 {
    g.fill(ring(.zero, 0.76, 0.04), p.accent)
  }
  if lod < 2 {
    for m in edgeMids { g.fill(diamond(m, 0.1), p.ink) }
  }
  for a in quarter + diagonals {
    g.fill(oval(angle: a, at: 0.4, length: lod == 2 ? 0.5 : 0.46, width: lod == 2 ? 0.26 : 0.22), p.ink)
    if lod == 0 {
      g.fill(oval(angle: a, at: 0.44, length: 0.2, width: 0.05), p.wash)
    }
  }
  g.fill(disc(.zero, lod == 2 ? 0.24 : 0.2), p.accent)
  if lod < 2 {
    g.fill(ring(.zero, 0.2, 0.03), p.line)
    g.fill(disc(.zero, 0.07), p.line)
  }
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
  case .ochre: drawOchre(g, p, lod: lod)
  case .prompt: drawPrompt(g, p, lod: lod)
  case .glass: drawGlass(g, p, lod: lod)
  case .split: drawSplit(g, p, lod: lod)
  case .night: drawNight(g, p, lod: lod)
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
  if lod == 0 && candidate.bleeds {
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
  // A–C only: with all eight plus the system rows it'd be ~4 MB. options.png
  // shows every candidate side by side.
  var rows: [Row] = [Candidate.classic, .cluster, .minimal].map { c in
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

/// Every candidate side by side: a light row and a dark row, each icon at
/// 256 px with its 32 and 16 px renders under it at 1:1, then the same two
/// blown up (3× and 4×, nearest neighbour) to judge the pixels.
func writeOptions(_ url: URL) {
  let cell = 256
  let gap = 40
  let margin = 40
  let header = 74
  let labelH = 44
  let smallH = 96  // the 3× zoom of the 32 px render
  let rowH = labelH + cell + 22 + smallH + 40
  let all = Candidate.allCases
  let width = margin * 2 + all.count * cell + (all.count - 1) * gap
  let height = header + rowH * 2
  let ctx = CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
  let rows: [(CGColor, NSColor, NSColor)] = [
    (rgb(0xECECEF), NSColor(white: 0.12, alpha: 1), NSColor(white: 0.42, alpha: 1)),
    (rgb(0x1E1F23), NSColor(white: 0.9, alpha: 1), NSColor(white: 0.6, alpha: 1)),
  ]
  // Header on the light band.
  ctx.setFillColor(rows[0].0)
  ctx.fill(CGRect(x: 0, y: height - header, width: width, height: header))
  label(
    ctx, "Azulejo app icon candidates · 256 px, then 32 and 16 px at 1:1, then 32 at 3× and 16 at 4×",
    at: CGPoint(x: margin, y: height - 50), size: 22, color: rows[0].1, bold: true)
  for (r, (bg, ink, dim)) in rows.enumerated() {
    let top = height - header - r * rowH
    ctx.setFillColor(bg)
    ctx.fill(CGRect(x: 0, y: top - rowH, width: width, height: rowH))
    for (i, c) in all.enumerated() {
      let x = CGFloat(margin + i * (cell + gap))
      let iconTop = CGFloat(top - labelH)
      label(ctx, "\(c.letter)  \(c.name)", at: CGPoint(x: x + 4, y: iconTop + 8), size: 21, color: ink, bold: true)
      if c == chosen {
        label(ctx, "default", at: CGPoint(x: x + CGFloat(cell) - 66, y: iconTop + 10), size: 16, color: dim)
      }
      ctx.interpolationQuality = .high
      ctx.draw(render(c, palette: c.palette, px: cell), in: CGRect(x: x, y: iconTop - CGFloat(cell), width: CGFloat(cell), height: CGFloat(cell)))
      let smallTop = iconTop - CGFloat(cell) - 22
      let i32 = render(c, palette: c.palette, px: 32)
      let i16 = render(c, palette: c.palette, px: 16)
      ctx.interpolationQuality = .none
      var sx = x + 8
      ctx.draw(i32, in: CGRect(x: sx, y: smallTop - 32, width: 32, height: 32))
      sx += 32 + 14
      ctx.draw(i16, in: CGRect(x: sx, y: smallTop - 16, width: 16, height: 16))
      sx += 16 + 30
      ctx.draw(i32, in: CGRect(x: sx, y: smallTop - 96, width: 96, height: 96))
      sx += 96 + 14
      ctx.draw(i16, in: CGRect(x: sx, y: smallTop - 64, width: 64, height: 64))
    }
  }
  writePNG(ctx.makeImage()!, url)
}

func writeDocs(_ dir: URL) {
  try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
  for c in Candidate.allCases {
    writePNG(render(c, palette: c.palette, px: 1024), dir.appendingPathComponent("candidate-\(c.rawValue).png"))
  }
  writePNG(render(chosen, palette: dark, px: 1024), dir.appendingPathComponent("appearance-dark.png"))
  writePNG(render(chosen, palette: tinted, px: 1024), dir.appendingPathComponent("appearance-tinted.png"))
  writeSheet(dir.appendingPathComponent("sheet.png"))
  writeOptions(dir.appendingPathComponent("options.png"))
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
