// Draws Azulejo's app icon: eight candidates (A–H), nine minimal marks (I–Q)
// in four styles, the chosen one's dark and tinted variants, the .iconset that
// scripts/icon/build.sh packs into AppIcon.icns, a contact sheet and
// side-by-sides of the candidates and the marks. Pure CoreGraphics, so it's
// reproducible on any Mac with the Command Line Tools (docs/design/icon/README.md).
//
//   swift scripts/icon/draw.swift --iconset DIR   # the 10 PNGs iconutil wants
//   swift scripts/icon/draw.swift --docs DIR      # candidates, variants, sheet.png, options.png, minimal.png
//   swift scripts/icon/draw.swift --minimal DIR   # minimal.png only
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
  /// Rim light strength, and the darkening at the foot of the glass. 0 and 0
  /// with no sheen is a flat icon (the minimal marks' fourth style).
  var rim: CGFloat = 1
  var shade: CGFloat = 1
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

/// What ships: one of the candidates A–H, or a minimal mark I–Q in one of its
/// four styles, e.g. `.mark(.woven, .glaze)`. See docs/design/icon/README.md
/// for why it's A.
let chosen: Icon = .candidate(.classic)

enum Icon: Equatable {
  case candidate(Candidate)
  case mark(Mark, Style)

  var title: String {
    switch self {
    case .candidate(let c): c.title
    case .mark(let m, let s): "\(m.letter) · \(m.name.lowercased()), \(s.name)"
    }
  }
  var palette: Palette {
    switch self {
    case .candidate(let c): c.palette
    case .mark(_, let s): s.palette
    }
  }
  func image(_ p: Palette, px: Int) -> CGImage {
    switch self {
    case .candidate(let c): render(c, palette: p, px: px)
    case .mark(let m, let s): renderMark(m, palette: p, bleeds: s.bleeds, px: px)
    }
  }
}

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

// MARK: - Minimal marks (I–Q)
//
// One confident mark in one blue, drawn from a few repeated primitives with
// one even stroke weight, after A, B and D. Each draws in a single colour
// (`ink`) and cuts its negative space with `punch`, so a style is just a
// palette: cobalt on glaze, white on cobalt and so on.

/// The four styles a mark comes in.
enum Style: Int, CaseIterable {
  case glaze = 1  // cobalt on the warm tin glaze, with A's sheen and ink bleed
  case cobalt  // white on a cobalt tile
  case gradient  // white on cobalt fading to ultramarine, the glass kept faint
  case flat  // cobalt on white, no sheen, no rim, no bleed

  var name: String {
    switch self {
    case .glaze: "cobalt on glaze"
    case .cobalt: "white on cobalt"
    case .gradient: "ultramarine glass"
    case .flat: "flat"
    }
  }
  var palette: Palette {
    switch self {
    case .glaze: tinGlaze
    case .cobalt: cobaltGround
    case .gradient: ultramarineGlaze
    case .flat: flatGlaze
    }
  }
  /// Only cobalt soaks into a tin glaze.
  var bleeds: Bool { self == .glaze }
}

/// Style 3: cobalt at the top deepening to ultramarine, a white mark, the
/// glass a whisper.
let ultramarineGlaze = Palette(
  groundTop: rgb(0x2D62C8), groundBottom: rgb(0x2B3AA8),
  ink: rgb(0xFFFFFF), line: rgb(0x0E2D6B), wash: rgb(0x8FB0E2),
  grout: rgb(0x173F88),
  multiply: false, sheen: 0.16, shadow: 0.5, rim: 0.3, shade: 0.4)

/// Style 4: one white, one cobalt, nothing else. Multiplied so the hairline
/// round the edge stays as faint as on the glaze.
let flatGlaze = Palette(
  groundTop: rgb(0xFBFAF7), groundBottom: rgb(0xFBFAF7),
  ink: rgb(0x1F4FA3), line: rgb(0x123578), wash: rgb(0x86A8DC),
  grout: rgb(0xDCD4C2),
  multiply: true, sheen: 0, shadow: 0.5, rim: 0, shade: 0)

/// The one stroke weight a mark uses at a given size, and the gap cut round a
/// strand where it passes over another. Each grows as the icon shrinks so a
/// line stays over a pixel and a half wide, and a gap over one pixel. At 16 px
/// there's no room for a gap, so strands don't weave there.
struct Pen {
  let lod: Int  // 0 from 128 px up, 1 at 64, 2 at 32, 3 at 16
  let px: Int
  var w: CGFloat
  var gap: CGFloat
  init(px: Int) {
    self.px = px
    lod = px <= 16 ? 3 : (px <= 32 ? 2 : (px <= 64 ? 1 : 0))
    w = [0.1, 0.11, 0.14, 0.24][lod]
    gap = [0.05, 0.06, 0.085, 0][lod]
  }
  var small: Bool { lod >= 2 }
  /// `n` pixels at this size, in tile space. The tile's centre sits on a
  /// pixel corner at every size, so whole pixels from it land on the grid.
  func pixels(_ n: CGFloat) -> CGFloat { n * 1024 / 412 / CGFloat(px) }
}

/// A closed strand flattened to points, with the arc length to each.
struct Polyline {
  var points: [CGPoint] = []
  var at: [CGFloat] = []  // arc length from the start to each point
  var length: CGFloat = 0

  init(_ path: CGPath) {
    var pts: [CGPoint] = []
    path.applyWithBlock { el in
      let e = el.pointee
      let p0 = pts.last ?? .zero
      switch e.type {
      case .moveToPoint, .addLineToPoint:
        pts.append(e.points[0])
      case .addQuadCurveToPoint:
        let (c, p) = (e.points[0], e.points[1])
        for i in 1...24 {
          let t = CGFloat(i) / 24, u = 1 - t
          pts.append(CGPoint(x: u * u * p0.x + 2 * u * t * c.x + t * t * p.x, y: u * u * p0.y + 2 * u * t * c.y + t * t * p.y))
        }
      case .addCurveToPoint:
        let (c1, c2, p) = (e.points[0], e.points[1], e.points[2])
        for i in 1...24 {
          let t = CGFloat(i) / 24, u = 1 - t
          let (a, b, c, d) = (u * u * u, 3 * u * u * t, 3 * u * t * t, t * t * t)
          pts.append(CGPoint(x: a * p0.x + b * c1.x + c * c2.x + d * p.x, y: a * p0.y + b * c1.y + c * c2.y + d * p.y))
        }
      default: break
      }
    }
    if let f = pts.first, let l = pts.last, pts.count > 1, hypot(f.x - l.x, f.y - l.y) < 1e-9 { pts.removeLast() }
    points = pts
    for (i, p) in pts.enumerated() {
      at.append(length)
      let q = pts[(i + 1) % pts.count]
      length += hypot(q.x - p.x, q.y - p.y)
    }
  }

  /// The strand as drawn: its points, closed.
  var path: CGPath {
    let p = CGMutablePath()
    p.addLines(between: points)
    p.closeSubpath()
    return p
  }

  /// The point at arc length `s`, wrapping round.
  func point(_ s: CGFloat) -> CGPoint {
    var s = s.truncatingRemainder(dividingBy: length)
    if s < 0 { s += length }
    var i = 0
    while i + 1 < points.count && at[i + 1] <= s { i += 1 }
    let p = points[i], q = points[(i + 1) % points.count]
    let seg = (i + 1 < points.count ? at[i + 1] : length) - at[i]
    let t = seg > 0 ? (s - at[i]) / seg : 0
    return CGPoint(x: p.x + (q.x - p.x) * t, y: p.y + (q.y - p.y) * t)
  }

  /// The stretch from arc length `from` to `to` (later, less than a lap on).
  func stretch(_ from: CGFloat, _ to: CGFloat) -> CGPath {
    let n = points.count
    var start = from.truncatingRemainder(dividingBy: length)
    if start < 0 { start += length }
    let span = to - from
    let first = at.firstIndex { $0 > start } ?? n
    let path = CGMutablePath()
    path.move(to: point(start))
    for step in 0..<n {
      let i = (first + step) % n
      let d = at[i] + (first + step >= n ? length : 0) - start
      if d >= span { break }
      path.addLine(to: points[i])
    }
    path.addLine(to: point(start + span))
    return path
  }
}

/// Even-weight strands, woven: closed paths that cross each other or
/// themselves, passing alternately over and under at each crossing they meet
/// in turn. Every strand is drawn, then at each crossing the passage on top
/// gets a gap cut either side of it through the one underneath. The cut is
/// held to the two passages (each runs about halfway to the crossings either
/// side), so it never touches anything else.
func weave(_ g: CGContext, _ strands: [CGPath], _ ink: CGColor, _ pen: Pen) {
  // Strands are drawn as the same polylines the crossings are found on, so a
  // redrawn passage lands exactly on the strand under it.
  let lines = strands.map(Polyline.init)
  for l in lines { g.fill(l.path.copy(strokingWithWidth: pen.w, lineCap: .round, lineJoin: .round, miterLimit: 4), ink) }
  guard pen.gap > 0 else { return }

  // Every crossing is two passes, one along each strand (or two along one).
  struct Pass { let s: CGFloat; let crossing: Int }
  var passes: [[Pass]] = Array(repeating: [], count: lines.count)
  var count = 0
  for i in lines.indices {
    for j in i..<lines.count {
      let (a, b) = (lines[i], lines[j])
      for m in a.points.indices {
        let p = a.points[m], r = a.points[(m + 1) % a.points.count] - p
        for n in stride(from: i == j ? m + 2 : 0, to: b.points.count, by: 1) {
          if i == j && m == 0 && n == b.points.count - 1 { continue }  // neighbours round the join
          let q = b.points[n], v = b.points[(n + 1) % b.points.count] - q
          if max(p.x, p.x + r.x) < min(q.x, q.x + v.x) || max(q.x, q.x + v.x) < min(p.x, p.x + r.x)
            || max(p.y, p.y + r.y) < min(q.y, q.y + v.y) || max(q.y, q.y + v.y) < min(p.y, p.y + r.y)
          { continue }
          let den = r.x * v.y - r.y * v.x
          if abs(den) < 1e-12 { continue }
          let d = q - p
          let t = (d.x * v.y - d.y * v.x) / den
          let u = (d.x * r.y - d.y * r.x) / den
          if t >= 0 && t < 1 && u >= 0 && u < 1 {
            passes[i].append(Pass(s: a.at[m] + t * hypot(r.x, r.y), crossing: count))
            passes[j].append(Pass(s: b.at[n] + u * hypot(v.x, v.y), crossing: count))
            count += 1
          }
        }
      }
    }
  }
  // Along each strand the passes go over at even places and under at odd,
  // after a per-strand offset chosen so a crossing's two passes disagree.
  // Closed curves in a plane always allow one.
  var ends: [[(strand: Int, place: Int)]] = Array(repeating: [], count: count)
  for i in passes.indices {
    passes[i].sort { $0.s < $1.s }
    for (k, p) in passes[i].enumerated() { ends[p.crossing].append((i, k)) }
  }
  var offset: [Int?] = Array(repeating: nil, count: lines.count)
  for root in lines.indices where offset[root] == nil {
    offset[root] = 0
    var queue = [root]
    while let i = queue.popLast() {
      for (k, p) in passes[i].enumerated() {
        let other = ends[p.crossing].first { !($0.strand == i && $0.place == k) }!
        let want = (offset[i]! + k + other.place + 1) % 2
        if let o = offset[other.strand] {
          precondition(o == want, "these strands can't alternate over and under")
        } else {
          offset[other.strand] = want
          queue.append(other.strand)
        }
      }
    }
  }

  /// A pass's passage: `reach` of the way back to the previous pass and on
  /// to the next.
  func passage(_ strand: Int, _ k: Int, _ reach: CGFloat) -> CGPath {
    let line = lines[strand]
    let list = passes[strand]
    let s = list[k].s
    var prev = list[(k + list.count - 1) % list.count].s
    var next = list[(k + 1) % list.count].s
    if prev >= s { prev -= line.length }
    if next <= s { next += line.length }
    return line.stretch(s - (s - prev) * reach, s + (next - s) * reach)
  }
  for e in ends {
    let over = e.first { (offset[$0.strand]! + $0.place) % 2 == 0 }!
    let under = e.first { !($0.strand == over.strand && $0.place == over.place) }!
    let top = passage(over.strand, over.place, 0.5)
    let region = passage(under.strand, under.place, 0.6)
      .copy(strokingWithWidth: pen.w + 2 * pen.gap + 0.04, lineCap: .butt, lineJoin: .round, miterLimit: 4)
    g.saveGState()
    g.addPath(region)
    g.clip()
    // Cut just the gap either side of the top passage: its halo less its
    // own band, built in a layer and knocked out of the strand below. The
    // top strand itself is never touched, so nothing has to be redrawn.
    g.setBlendMode(.destinationOut)
    g.beginTransparencyLayer(auxiliaryInfo: nil)
    g.setBlendMode(.normal)
    g.fill(top.copy(strokingWithWidth: pen.w + 2 * pen.gap, lineCap: .butt, lineJoin: .round, miterLimit: 4), rgb(0x000000))
    g.setBlendMode(.clear)
    g.fill(top.copy(strokingWithWidth: pen.w, lineCap: .round, lineJoin: .round, miterLimit: 4), rgb(0x000000))
    g.endTransparencyLayer()
    g.restoreGState()
  }
}

/// A spirograph curve: a circle of radius `r` rolling inside one of radius
/// `R`, the pen `d` from its centre, scaled to reach `reach` and drawn as one
/// closed polyline. With R:r = 8:3 it's eight interlocking petals.
func hypotrochoid(R: CGFloat, r: CGFloat, d: CGFloat, reach: CGFloat, steps: Int) -> CGPath {
  let k = (R - r) / r
  let scale = reach / (R - r + d)
  let turns = r  // R and r whole and coprime: it closes after r turns
  let pts = (0..<steps).map { i -> CGPoint in
    let t = CGFloat(i) / CGFloat(steps) * 2 * .pi * turns
    return CGPoint(x: scale * ((R - r) * cos(t) + d * cos(k * t)), y: scale * ((R - r) * sin(t) - d * sin(k * t)))
  }
  let p = CGMutablePath()
  p.addLines(between: pts)
  p.closeSubpath()
  return p
}

extension CGPoint {
  static func - (a: CGPoint, b: CGPoint) -> CGPoint { CGPoint(x: a.x - b.x, y: a.y - b.y) }
}

/// A teardrop along `angle`: its point `tip` from the centre, its round end
/// radius `r` centred `d` beyond the point.
func teardrop(angle: CGFloat, tip: CGFloat, d: CGFloat, r: CGFloat) -> CGPath {
  let p = CGMutablePath()
  let beta = acos(r / d)  // at the round end's centre, between the point and a tangent
  p.move(to: CGPoint(x: tip, y: 0))
  p.addArc(center: CGPoint(x: tip + d, y: 0), radius: r, startAngle: .pi - beta, endAngle: beta - .pi, clockwise: true)
  p.closeSubpath()
  var t = CGAffineTransform(rotationAngle: angle)
  return p.copy(using: &t)!
}

/// A rounded square, half-width `h`, corner radius `r`, turned by `angle`.
func roundSquare(_ c: CGPoint, _ h: CGFloat, _ r: CGFloat, angle: CGFloat = 0) -> CGPath {
  var t = CGAffineTransform(translationX: c.x, y: c.y).rotated(by: angle)
  return CGPath(roundedRect: CGRect(x: -h, y: -h, width: 2 * h, height: 2 * h), cornerWidth: r, cornerHeight: r, transform: &t)
}

/// An eight-pointed star (two squares) with its points and notches rounded
/// by `round`.
func softStar(_ r: CGFloat, round: CGFloat) -> [CGPath] {
  let star = octagram(.zero, r - round / 2)
  return [star, star.copy(strokingWithWidth: round, lineCap: .round, lineJoin: .round, miterLimit: 4)]
}

enum Mark: String, CaseIterable {
  case knot = "i-knot"
  case petals = "j-petals"
  case framed = "k-framed"
  case joined = "l-joined"
  case linked = "m-linked"
  case counter = "n-counter"
  case woven = "o-woven"
  case diamond = "p-diamond"
  case cut = "q-cut"

  var letter: String { String(rawValue.prefix(1)).uppercased() }
  var parent: Candidate {
    switch self {
    case .knot, .petals, .framed: .classic
    case .joined, .linked, .counter: .cluster
    case .woven, .diamond, .cut: .ochre
    }
  }
  var name: String {
    switch self {
    case .knot: "Knot rosette"
    case .petals: "Eight petals"
    case .framed: "Framed rosette"
    case .joined: "Joined circle"
    case .linked: "Linked tiles"
    case .counter: "Counterchange"
    case .woven: "Woven star"
    case .diamond: "Star diamond"
    case .cut: "Cut star"
    }
  }
  /// One line for minimal.png.
  var blurb: String {
    switch self {
    case .knot: "one line, 8 petals, woven"
    case .petals: "8 teardrops, open centre"
    case .framed: "4 circles, border, corner arcs"
    case .joined: "4 tiles meet in a circle"
    case .linked: "a ring woven through 4 tiles"
    case .counter: "solid and outline tiles swap a disc"
    case .woven: "2 rounded squares, woven"
    case .diamond: "star cut from a diamond, corners"
    case .cut: "star cut from a cobalt tile"
    }
  }

  func draw(_ g: CGContext, _ ink: CGColor, _ pen: Pen) {
    switch self {
    case .knot: drawKnot(g, ink, pen)
    case .petals: drawPetals(g, ink, pen)
    case .framed: drawFramed(g, ink, pen)
    case .joined: drawJoined(g, ink, pen)
    case .linked: drawLinked(g, ink, pen)
    case .counter: drawCounter(g, ink, pen)
    case .woven: drawWoven(g, ink, pen)
    case .diamond: drawStarDiamond(g, ink, pen)
    case .cut: drawCutStar(g, ink, pen)
    }
  }
}

/// I (from A): A's eight petals as one continuous line of even weight (a
/// spirograph curve), woven over and under itself round an open centre.
func drawKnot(_ g: CGContext, _ ink: CGColor, _ pen: Pen) {
  if pen.lod == 3 {
    // At 16 px a loop's hole is under a pixel, eight solid petals round a
    // hole make a cog and four make an ×. Eight spokes from the centre stay
    // a flower.
    for a in quarter + diagonals { g.fill(oval(angle: a, at: 0.4, length: 0.66, width: 0.24), ink) }
    return
  }
  weave(g, [hypotrochoid(R: 8, r: 3, d: 1.5, reach: 0.74, steps: 1440)], ink, pen)
}

/// J (from A): the rosette as eight identical teardrops round an open centre,
/// with quarter discs in the corners that meet the next tile's.
func drawPetals(_ g: CGContext, _ ink: CGColor, _ pen: Pen) {
  // At 16 px slimmer petals from nearer the centre, so they don't merge.
  for a in quarter + diagonals {
    if pen.lod == 3 {
      g.fill(teardrop(angle: a, tip: 0.04, d: 0.46, r: 0.12), ink)
    } else {
      g.fill(teardrop(angle: a, tip: 0.16, d: 0.39, r: 0.155), ink)
    }
  }
  for c in corners { g.fill(disc(c, pen.small ? 0.42 : 0.36), ink) }
}

/// K (from A): four circles through the centre make the rosette; a fine
/// border with quarter arcs in its corners makes it a tile.
func drawFramed(_ g: CGContext, _ ink: CGColor, _ pen: Pen) {
  let r: CGFloat = pen.small ? 0.25 : 0.26
  let h: CGFloat = 0.74
  // Four circles at 16 px are one blob: a ring in a frame instead.
  var lines = pen.lod == 3 ? [disc(.zero, 0.3)] : edgeMids.map { m in disc(CGPoint(x: m.x * r, y: m.y * r), r) }
  // The border: two squares, filled even-odd. CoreGraphics strokes a square
  // at this scale with its outer edge askew.
  let border = CGMutablePath()
  for e in [h + pen.w / 2, h - pen.w / 2] { border.addRect(CGRect(x: -e, y: -e, width: 2 * e, height: 2 * e)) }
  g.setFillColor(ink)
  g.addPath(border)
  g.fillPath(using: .evenOdd)
  if pen.lod < 3 {
    for k in corners {
      let inward = atan2(-k.y, -k.x)
      let arc = CGMutablePath()
      arc.addArc(center: CGPoint(x: k.x * h, y: k.y * h), radius: 0.24, startAngle: inward - .pi / 4, endAngle: inward + .pi / 4, clockwise: false)
      lines.append(arc)
    }
  }
  for l in lines {
    g.fill(l.copy(strokingWithWidth: pen.w, lineCap: .round, lineJoin: .miter, miterLimit: 4), ink)
  }
}

/// L (from B): four tiles, each with a quarter circle at its inner corner, so
/// a circle opens where they meet, a four-petal rosette inside it.
func drawJoined(_ g: CGContext, _ ink: CGColor, _ pen: Pen) {
  let joint: CGFloat = pen.lod == 3 ? 0.16 : (pen.small ? 0.13 : 0.09)
  let h = (1.5 - joint) / 4  // each tile's half-width; the four span ±0.75
  for k in corners {
    g.fill(roundSquare(CGPoint(x: k.x * (joint / 2 + h), y: k.y * (joint / 2 + h)), h, h * 0.32), ink)
  }
  g.punch(disc(.zero, pen.lod == 3 ? 0.38 : 0.44))
  if pen.lod == 3 { return }
  for a in diagonals {
    g.fill(teardrop(angle: a, tip: joint / 2 * 1.41 + 0.04, d: 0.16, r: pen.small ? 0.1 : 0.085), ink)
  }
}

/// M (from B): a 2×2 of identical rounded squares, a ring woven through all
/// four, over one side of each tile and under the other, like a chain.
///
/// At 64 px and below it's set in whole pixels so the outlines land on the
/// grid: the tiles a little bigger and bolder, the ring a little wider so
/// each link reaches further into its tiles. At 16 px there's no room for a
/// weave (and a ring there made a target, or a ⌘), so it's four bold outlined
/// tiles.
func drawLinked(_ g: CGContext, _ ink: CGColor, _ pen: Pen) {
  var pen = pen
  var o: CGFloat = 0.42  // tile centres at (±o, ±o)
  var h: CGFloat = 0.28
  var rc: CGFloat = 0.1
  var r: CGFloat = 0.4  // the ring cuts across each tile's inner corner
  if pen.lod > 0 {
    // In pixels from the centre: the outlines' inner and outer edges, their
    // weight, the corner radius and ring radius on the stroke's centre line,
    // and the gap either side of a strand on top.
    let (inner, outer, w, corner, ring, gap): (CGFloat, CGFloat, CGFloat, CGFloat, CGFloat, CGFloat) =
      [(2, 20, 3, 3, 11, 1.5), (1, 11, 2, 1.5, 6.5, 1), (1, 6, 2, 1, 0, 0)][pen.lod - 1]
    o = pen.pixels(inner + outer) / 2
    h = pen.pixels(outer - inner - w) / 2
    rc = pen.pixels(corner)
    r = pen.pixels(ring)
    pen.w = pen.pixels(w)
    pen.gap = pen.pixels(gap)
  }
  let tiles = corners.map { k in roundSquare(CGPoint(x: k.x * o, y: k.y * o), h, rc) }
  if pen.lod == 3 {
    for t in tiles { g.fill(t.copy(strokingWithWidth: pen.w, lineCap: .round, lineJoin: .round, miterLimit: 4), ink) }
    return
  }
  // Going round, ring and tile take turns on top: a chain of four links.
  weave(g, tiles + [disc(.zero, r)], ink, pen)
}

/// N (from B): B's alternating tiles as solid and outline squares, and a disc
/// across the joint that swaps with them: bare on the solid tiles, solid in
/// the outlines.
func drawCounter(_ g: CGContext, _ ink: CGColor, _ pen: Pen) {
  let joint: CGFloat = pen.lod == 3 ? 0.2 : (pen.small ? 0.14 : 0.1)
  let h = (1.5 - joint) / 4
  let r: CGFloat = 0.42
  for (i, k) in corners.enumerated() {
    let c = CGPoint(x: k.x * (joint / 2 + h), y: k.y * (joint / 2 + h))
    let tile = roundSquare(c, h, h * 0.32)
    g.saveGState()
    g.addPath(tile)
    g.clip()
    if i % 2 == 0 {
      g.fill(tile, ink)
      g.punch(disc(.zero, r))
    } else {
      // The outline sits inside the tile so solid and outline match in size.
      g.fill(roundSquare(c, h - pen.w / 2, h * 0.32 - pen.w / 2).copy(strokingWithWidth: pen.w, lineCap: .butt, lineJoin: .round, miterLimit: 4), ink)
      g.fill(disc(.zero, r), ink)
    }
    g.restoreGState()
  }
}

/// O (from D): D's eight-pointed star as two rounded squares of one even
/// line, woven over and under like a khatam.
func drawWoven(_ g: CGContext, _ ink: CGColor, _ pen: Pen) {
  if pen.lod == 3 {
    // Woven, or as an outline, it's a ring at 16 px. Solid, it keeps its points.
    for p in softStar(0.78, round: 0.06) { g.fill(p, ink) }
    return
  }
  let s: CGFloat = 0.55
  let rc: CGFloat = s * 0.12
  let strands = [roundSquare(.zero, s, rc), roundSquare(.zero, s, rc, angle: .pi / 4)]
  weave(g, strands, ink, pen)
}

/// P (from D): D in one blue. The eight-pointed star cut from the diamond,
/// quarter discs in the corners. (A dot at the star's heart made it a cog.)
func drawStarDiamond(_ g: CGContext, _ ink: CGColor, _ pen: Pen) {
  let d: CGFloat = 0.8
  g.fill(diamond(.zero, d), ink)
  g.fill(diamond(.zero, d).copy(strokingWithWidth: 0.12, lineCap: .round, lineJoin: .round, miterLimit: 4), ink)
  for p in softStar(pen.small ? 0.5 : 0.48, round: 0.05) { g.punch(p) }
  for c in corners { g.fill(disc(c, pen.small ? 0.4 : 0.36), ink) }
}

/// Q (from D): a cobalt tile with the eight-pointed star cut out of it.
func drawCutStar(_ g: CGContext, _ ink: CGColor, _ pen: Pen) {
  let h: CGFloat = pen.small ? 0.74 : 0.7
  g.fill(roundSquare(.zero, h, h * 0.24), ink)
  for p in softStar(pen.small ? 0.58 : 0.54, round: 0.05) { g.punch(p) }
}

// MARK: - Glass

func bitmap(_ px: Int) -> CGContext {
  CGContext(
    data: nil, width: px, height: px, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
}

/// The motif alone, on transparency, in its own bitmap: its knock-outs
/// (`punch`) only cut the motif, never the glaze under it. `draw` works in
/// tile space.
func motifLayer(px: Int, _ draw: (CGContext) -> Void) -> CGImage {
  let g = bitmap(px)
  let s = CGFloat(px) / 1024
  g.scaleBy(x: s, y: s)
  g.addPath(squircle(CGRect(x: 100, y: 100, width: 824, height: 824), 185.4))
  g.clip()
  g.translateBy(x: 512, y: 512)
  g.scaleBy(x: 412, y: 412)
  draw(g)
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

/// Renders one of the candidates A–H at `px` × `px` pixels.
func render(_ candidate: Candidate, palette p: Palette, px: Int) -> CGImage {
  let lod = px <= 32 ? 2 : (px <= 64 ? 1 : 0)
  return renderTile(p, px: px, bleeds: candidate.bleeds) { g in
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
  }
}

/// Renders a minimal mark at `px` × `px` pixels, in `p.ink` on `p`'s glaze.
func renderMark(_ mark: Mark, palette p: Palette, bleeds: Bool, px: Int) -> CGImage {
  renderTile(p, px: px, bleeds: bleeds) { g in mark.draw(g, p.ink, Pen(px: px)) }
}

/// The tile around a motif: shadow, glaze, the motif, the glass over it.
func renderTile(_ p: Palette, px: Int, bleeds: Bool, motif: (CGContext) -> Void) -> CGImage {
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
  let art = motifLayer(px: px, motif)
  let canvas = CGRect(x: 0, y: 0, width: 1024, height: 1024)
  g.saveGState()
  if p.multiply { g.setBlendMode(.multiply) }
  if lod == 0 && bleeds {
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
      colors: [rgb(0xFFFFFF, 0.42 * p.sheen), rgb(0xFFFFFF, 0.08 * p.sheen), rgb(0xFFFFFF, 0), rgb(0x000000, 0.10 * p.shade)] as CFArray,
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
      colors: [rgb(0xFFFFFF, 0.95 * p.rim), rgb(0xFFFFFF, 0.15 * p.rim), rgb(0xFFFFFF, 0.08 * p.rim), rgb(0xFFFFFF, 0.45 * p.rim)] as CFArray,
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
    writePNG(chosen.image(chosen.palette, px: px), dir.appendingPathComponent(name))
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
    Row(name: c.title + (.candidate(c) == chosen ? "  (default)" : ""), images: { render(c, palette: c.palette, px: $0) }, ns: nil)
  }
  rows.append(Row(name: chosen.title + " · dark", images: { chosen.image(dark, px: $0) }, ns: nil))
  rows.append(Row(name: chosen.title + " · tinted (mono source)", images: { chosen.image(tinted, px: $0) }, ns: nil))
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
      if .candidate(c) == chosen {
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

/// The minimal marks: A, B and D on top for reference, then I–Q grouped under
/// their parent, one row each. Columns: the four styles at 192 px, then the
/// 32 and 16 px renders of styles 1 and 2 at 1:1 and blown up (3× and 4×,
/// nearest neighbour).
func writeMinimal(_ dir: URL) {
  try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
  let cell = 192
  let gap = 24
  let margin = 40
  let labelW = 250
  let header = 84
  let colHead = 40
  let rowH = cell + 26
  let groupH = 48
  let parentsH = 40 + cell + 34
  let stripStyles: [Style] = [.glaze, .cobalt]
  let stripW = 32 + 12 + 16 + 22 + 96 + 12 + 64
  let stripGap = 34
  let iconsX = margin + labelW
  let stripX = iconsX + 4 * cell + 3 * gap + 44
  let width = stripX + stripStyles.count * stripW + (stripStyles.count - 1) * stripGap + margin
  let groups: [Candidate] = [.classic, .cluster, .ochre]
  let height = header + parentsH + colHead + groups.count * (groupH + 3 * rowH) + 24
  let g = CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
  let ink = NSColor(white: 0.12, alpha: 1)
  let dim = NSColor(white: 0.42, alpha: 1)
  g.setFillColor(rgb(0xECECEF))
  g.fill(CGRect(x: 0, y: 0, width: width, height: height))
  var y = CGFloat(height - 50)
  label(g, "Azulejo app icon · minimal marks I–Q, after A, B and D · 192 px in four styles, then 32 and 16 px at 1:1 and zoomed", at: CGPoint(x: CGFloat(margin), y: y), size: 22, color: ink, bold: true)
  y = CGFloat(height - header)

  // The parents, for reference.
  label(g, "Parents", at: CGPoint(x: CGFloat(margin), y: y - 30), size: 20, color: ink, bold: true)
  for (i, c) in groups.enumerated() {
    let x = CGFloat(iconsX + i * (cell + gap))
    label(g, "\(c.letter)  \(c.name)", at: CGPoint(x: x + 4, y: y - 30), size: 17, color: dim, bold: true)
    g.interpolationQuality = .high
    g.draw(render(c, palette: c.palette, px: cell), in: CGRect(x: x, y: y - 40 - CGFloat(cell), width: CGFloat(cell), height: CGFloat(cell)))
  }
  y -= CGFloat(parentsH)

  // Column heads.
  g.setFillColor(rgb(0xC9C9CF))
  g.fill(CGRect(x: CGFloat(margin), y: y - 2, width: CGFloat(width - 2 * margin), height: 2))
  for (i, s) in Style.allCases.enumerated() {
    label(g, "\(s.rawValue) · \(s.name)", at: CGPoint(x: CGFloat(iconsX + i * (cell + gap)) + 4, y: y - 32), size: 17, color: ink, bold: true)
  }
  for (i, s) in stripStyles.enumerated() {
    label(g, "\(s.rawValue) · 32 and 16 px", at: CGPoint(x: CGFloat(stripX + i * (stripW + stripGap)), y: y - 32), size: 17, color: ink, bold: true)
  }
  y -= CGFloat(colHead)

  for parent in groups {
    label(g, "From \(parent.letter) · \(parent.name)", at: CGPoint(x: CGFloat(margin), y: y - 36), size: 20, color: dim, bold: true)
    y -= CGFloat(groupH)
    for m in Mark.allCases where m.parent == parent {
      let top = y
      label(g, "\(m.letter)  \(m.name)", at: CGPoint(x: CGFloat(margin), y: top - 40), size: 22, color: ink, bold: true)
      label(g, m.blurb, at: CGPoint(x: CGFloat(margin), y: top - 66), size: 15, color: dim)
      for (i, s) in Style.allCases.enumerated() {
        let x = CGFloat(iconsX + i * (cell + gap))
        g.interpolationQuality = .high
        g.draw(renderMark(m, palette: s.palette, bleeds: s.bleeds, px: cell), in: CGRect(x: x, y: top - CGFloat(cell), width: CGFloat(cell), height: CGFloat(cell)))
      }
      for (i, s) in stripStyles.enumerated() {
        let i32 = renderMark(m, palette: s.palette, bleeds: s.bleeds, px: 32)
        let i16 = renderMark(m, palette: s.palette, bleeds: s.bleeds, px: 16)
        g.interpolationQuality = .none
        var x = CGFloat(stripX + i * (stripW + stripGap))
        let base = top - CGFloat(cell) / 2
        g.draw(i32, in: CGRect(x: x, y: base - 16, width: 32, height: 32))
        x += 32 + 12
        g.draw(i16, in: CGRect(x: x, y: base - 8, width: 16, height: 16))
        x += 16 + 22
        g.draw(i32, in: CGRect(x: x, y: base - 48, width: 96, height: 96))
        x += 96 + 12
        g.draw(i16, in: CGRect(x: x, y: base - 32, width: 64, height: 64))
      }
      y -= CGFloat(rowH)
    }
  }
  writePNG(g.makeImage()!, dir.appendingPathComponent("minimal.png"))
}

func writeDocs(_ dir: URL) {
  try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
  for c in Candidate.allCases {
    writePNG(render(c, palette: c.palette, px: 1024), dir.appendingPathComponent("candidate-\(c.rawValue).png"))
  }
  writePNG(chosen.image(dark, px: 1024), dir.appendingPathComponent("appearance-dark.png"))
  writePNG(chosen.image(tinted, px: 1024), dir.appendingPathComponent("appearance-tinted.png"))
  writeSheet(dir.appendingPathComponent("sheet.png"))
  writeOptions(dir.appendingPathComponent("options.png"))
  writeMinimal(dir)
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
  case "--minimal": writeMinimal(url)
  default: fatalError("unknown option \(flag)")
  }
}
