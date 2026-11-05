// Draws Azulejo's app icon: eight candidates (A–H), nine minimal marks (I–Q)
// in four styles, six woven marks (R–W) in seven, seven pinwheel marks (X,
// X1–X6) grown from R at 32 px, Y (the prompt key, the maintainer's own design) in
// light, dark and clear, the chosen one's variants, the .iconsets that
// scripts/icon/build.sh packs into AppIcon.icns and AppIconDark.icns, a
// contact sheet and side-by-sides of the candidates and the marks. Pure
// CoreGraphics, so it's reproducible on any Mac with the Command Line Tools
// (docs/design/icon/README.md).
//
//   swift scripts/icon/draw.swift --iconset DIR   # the 10 PNGs iconutil wants
//   swift scripts/icon/draw.swift --iconset-dark DIR  # the same, dark look
//   swift scripts/icon/draw.swift --key DIR       # prompt.png and prompt-vs-mockup.png (reads DIR/prompt-mockup.png)
//   swift scripts/icon/draw.swift --composer DIR  # Y's layers as SVG, for Icon Composer
//   swift scripts/icon/draw.swift --docs DIR      # everything below, plus candidates, variants, sheet.png, options.png
//   swift scripts/icon/draw.swift --minimal DIR   # minimal.png only
//   swift scripts/icon/draw.swift --woven DIR     # woven.png and woven-hero.png only
//   swift scripts/icon/draw.swift --pinwheel DIR  # pinwheel.png only (reads DIR/pinwheel-liked-32px.png)
//   swift scripts/icon/draw.swift --chosen DIR    # chosen.png only: the shipping icon beside Terminal and Notes
//
// Geometry follows Apple's macOS icon template: a 1024 px canvas, an 824 px
// rounded square (continuous corners, radius 185.4; Y uses macOS 26's own
// 214, `tahoeRadius`) centred in it, fully opaque inside, and the template's
// drop shadow (black 50 %, blur 28, 12 px down) in the margin. macOS 26 clips
// icons of that shape with its own mask, rather than shrinking them onto a
// grey "legacy" plate.

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
  /// The woven marks' second blue (two-tone only): their floral strands, and
  /// the pale wash some of them fill their petals with. Nil is one colour.
  var second: CGColor? = nil
  var fill: CGColor? = nil
  /// The dark hairline inside the edge; nil is 0.10 on a multiplied glaze
  /// and 0.18 otherwise.
  var hairline: CGFloat? = nil
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

/// What ships: one of the candidates A–H, or a mark (I–X6) in one of the
/// styles, e.g. `.mark(.woven, .glaze)`, or `.key`. the maintainer picked X,
/// pinwheel, white on cobalt (Nov 4, 2026), then drew Y himself (Nov 5);
/// docs/design/icon/README.md.
let chosen: Icon = .key

enum Icon: Equatable {
  case candidate(Candidate)
  case mark(Mark, Style)
  /// Y, the prompt key: the maintainer's own design, drawn in its three looks.
  case key

  var title: String {
    switch self {
    case .candidate(let c): c.title
    case .mark(let m, let s): "\(m.letter) · \(m.name.lowercased()), \(s.name)"
    case .key: "Y · prompt key"
    }
  }
  var palette: Palette {
    switch self {
    case .candidate(let c): c.palette
    case .mark(_, let s): s.palette
    case .key: keyLook(.light).tile
    }
  }
  func image(_ p: Palette, px: Int) -> CGImage {
    switch self {
    case .candidate(let c): render(c, palette: p, px: px)
    case .mark(let m, let s): renderMark(m, palette: p, bleeds: s.bleeds, px: px)
    case .key: renderKey(.light, px: px)
    }
  }
  /// One appearance. Y draws its own dark and clear; the others recolour
  /// with the shared `dark` palette, and clear falls back to the `tinted`
  /// mono source.
  func image(_ look: Look, px: Int) -> CGImage {
    switch (self, look) {
    case (.key, _): renderKey(look, px: px)
    case (_, .light): image(palette, px: px)
    case (_, .dark): image(dark, px: px)
    case (_, .clear): image(tinted, px: px)
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

/// The styles a mark comes in: the minimal marks' four (minimal.png), and
/// the woven marks' seven (woven.png). Any mark takes any style.
enum Style: Int, CaseIterable {
  case glaze = 1  // cobalt on the warm tin glaze, with A's sheen and ink bleed
  case cobalt  // white on a cobalt tile
  case gradient  // white on cobalt fading to ultramarine, the glass kept faint
  case flat  // cobalt on white, no sheen, no rim, no bleed
  case indigo  // navy ink on a cool porcelain white
  case sky  // white on ultramarine lifting to sky blue, a little more glass
  case manganese  // aubergine on the warm glaze, the old Portuguese purple
  case verdigris  // green-blue on the warm glaze, like older tiles
  case twoTone  // cobalt and a lighter sky blue on the warm glaze

  /// minimal.png's columns, and woven.png's, in order.
  static let minimal: [Style] = [.glaze, .cobalt, .gradient, .flat]
  static let woven: [Style] = [.glaze, .indigo, .cobalt, .sky, .manganese, .verdigris, .twoTone]

  var name: String {
    switch self {
    case .glaze: "cobalt on glaze"
    case .cobalt: "white on cobalt"
    case .gradient: "ultramarine glass"
    case .flat: "flat"
    case .indigo: "indigo on porcelain"
    case .sky: "sky glass"
    case .manganese: "manganese on glaze"
    case .verdigris: "verdigris on glaze"
    case .twoTone: "two-tone blue"
    }
  }
  var palette: Palette {
    switch self {
    case .glaze: tinGlaze
    case .cobalt: cobaltGround
    case .gradient: ultramarineGlaze
    case .flat: flatGlaze
    case .indigo: porcelain
    case .sky: skyGlass
    case .manganese: manganeseGlaze
    case .verdigris: verdigrisGlaze
    case .twoTone: twoToneGlaze
    }
  }
  /// Only a pigment fired into a white glaze bleeds.
  var bleeds: Bool { palette.multiply && self != .flat }
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

/// Woven style 2: a darker, inkier navy on a cooler porcelain white.
let porcelain = Palette(
  groundTop: rgb(0xF9F9F6), groundBottom: rgb(0xE5E7E9),
  ink: rgb(0x1B2A5E), line: rgb(0x0F183D), wash: rgb(0x8290BC),
  grout: rgb(0xD6D9DD),
  multiply: true, sheen: 0.55, shadow: 0.5)

/// Woven style 4: ultramarine at the foot lifting to sky blue at the top, a
/// white mark, the sheen kept subtle.
let skyGlass = Palette(
  groundTop: rgb(0x5BA8EE), groundBottom: rgb(0x2638A8),
  ink: rgb(0xFFFFFF), line: rgb(0x0E2D6B), wash: rgb(0x8FB0E2),
  grout: rgb(0x173F88),
  multiply: false, sheen: 0.3, shadow: 0.5, rim: 0.55, shade: 0.5)

/// Woven style 5: manganese, the aubergine purple-brown of 16th- and
/// 17th-century Portuguese tiles, on the warm glaze.
let manganeseGlaze = Palette(
  groundTop: rgb(0xFBF8F1), groundBottom: rgb(0xEDE6D6),
  ink: rgb(0x5A2B4E), line: rgb(0x3C1A33), wash: rgb(0xB898AE),
  grout: rgb(0xDCD4C2),
  multiply: true, sheen: 0.55, shadow: 0.5)

/// Woven style 6: verdigris, the green-blue of older tiles, on the warm glaze.
let verdigrisGlaze = Palette(
  groundTop: rgb(0xFBF8F1), groundBottom: rgb(0xEDE6D6),
  ink: rgb(0x1C6675), line: rgb(0x10434D), wash: rgb(0x8CC2C4),
  grout: rgb(0xDCD4C2),
  multiply: true, sheen: 0.55, shadow: 0.5)

/// Woven style 7: cobalt, with the floral strands in a lighter sky blue and
/// the petals washed paler still.
let twoToneGlaze = Palette(
  groundTop: rgb(0xFBF8F1), groundBottom: rgb(0xEDE6D6),
  ink: rgb(0x1F4FA3), line: rgb(0x123578), wash: rgb(0x86A8DC),
  grout: rgb(0xDCD4C2),
  multiply: true, sheen: 0.55, shadow: 0.5,
  second: rgb(0x5E93DB), fill: rgb(0xC6D9F2))

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
  weave(g, strands, inks: strands.map { _ in ink }, pen)
}

/// The same, each strand in its own colour (`inks` lines up with `strands`).
///
/// `reach` is how far each cut runs along the strand on top, as a share of
/// the way to the next crossing, and how far along the one underneath it may
/// touch.
func weave(_ g: CGContext, _ strands: [CGPath], inks: [CGColor], _ pen: Pen, reach: (top: CGFloat, under: CGFloat) = (0.5, 0.6)) {
  // Strands are drawn as the same polylines the crossings are found on, so a
  // redrawn passage lands exactly on the strand under it.
  let lines = strands.map(Polyline.init)
  for (l, ink) in zip(lines, inks) {
    g.fill(l.path.copy(strokingWithWidth: pen.w, lineCap: .round, lineJoin: .round, miterLimit: 4), ink)
  }
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
    let top = passage(over.strand, over.place, reach.top)
    let region = passage(under.strand, under.place, reach.under)
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
  // The woven marks (R–W), grown from M.
  case bloom = "r-bloom"
  case quatrefoil = "s-quatrefoil"
  case daisy = "t-daisy"
  case petalTiles = "u-petal-tiles"
  case fourTiles = "v-four-tiles"
  case starFlower = "w-star-flower"
  // Pinwheel marks (X): R's 32 px rendering grown up, and the spiral in other tilings.
  case pinwheel = "x-pinwheel"
  case pinwheelTiling = "x1-pinwheel-tiling"
  case masterStack = "x2-master-stack"
  case steps = "x3-steps"
  case astroid = "x4-four-point-star"
  case pinwheelStar = "x5-pinwheel-star"
  case bigSmall = "x6-big-small"

  /// minimal.png's rows, and woven.png's (M first, for reference).
  static let minimal: [Mark] = [.knot, .petals, .framed, .joined, .linked, .counter, .woven, .diamond, .cut]
  static let wovenSet: [Mark] = [.linked, .bloom, .quatrefoil, .daisy, .petalTiles, .fourTiles, .starFlower]
  /// pinwheel.png's rows.
  static let pinwheelSet: [Mark] = [.pinwheel, .pinwheelTiling, .masterStack, .steps, .astroid, .pinwheelStar, .bigSmall]

  var letter: String { String(rawValue.prefix { $0 != "-" }).uppercased() }
  var parent: Candidate {
    switch self {
    case .knot, .petals, .framed: .classic
    case .joined, .linked, .counter: .cluster
    case .woven, .diamond, .cut: .ochre
    case .bloom, .quatrefoil, .daisy, .petalTiles, .fourTiles, .starFlower: .cluster
    case .pinwheel, .pinwheelTiling, .masterStack, .steps, .astroid, .pinwheelStar, .bigSmall: .cluster
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
    case .bloom: "Linked bloom"
    case .quatrefoil: "Woven quatrefoil"
    case .daisy: "Woven daisy"
    case .petalTiles: "Petal tiles"
    case .fourTiles: "Flower of four tiles"
    case .starFlower: "Woven star-flower"
    case .pinwheel: "Pinwheel"
    case .pinwheelTiling: "Pinwheel tiling"
    case .masterStack: "Master and stack"
    case .steps: "Steps"
    case .astroid: "Four-point star"
    case .pinwheelStar: "Pinwheel star"
    case .bigSmall: "Big and small"
    }
  }
  /// One line for minimal.png and woven.png.
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
    case .bloom: "M's ring as two woven 4-lobed loops"
    case .quatrefoil: "4 circles woven, in a tile"
    case .daisy: "8 petal loops, each woven through the next"
    case .petalTiles: "M's tiles as petals, the ring through them"
    case .fourTiles: "L's tiles hold one woven flower"
    case .starFlower: "2 four-lobed loops woven, 8 petals"
    case .pinwheel: "R at 32 px, grown up: 4 tiles, 4 hooks"
    case .pinwheelTiling: "4 rectangles turn round an open square"
    case .masterStack: "1 tall tile and 2 stacked, like the app"
    case .steps: "tiles grow clockwise, small to large"
    case .astroid: "rounded inner corners: a 4-point star gap"
    case .pinwheelStar: "tiles turned 15°: a pinwheel-star gap"
    case .bigSmall: "two 9 × 9 tiles and two 7 × 7, checkered"
    }
  }

  func draw(_ g: CGContext, _ p: Palette, _ pen: Pen) {
    let ink = p.ink
    let inks = Inks(p)
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
    case .bloom: drawBloom(g, inks, pen)
    case .quatrefoil: drawQuatrefoil(g, inks, pen)
    case .daisy: drawDaisy(g, inks, pen)
    case .petalTiles: drawPetalTiles(g, inks, pen)
    case .fourTiles: drawFourTiles(g, inks, pen)
    case .starFlower: drawStarFlower(g, inks, pen)
    case .pinwheel, .pinwheelTiling, .masterStack, .steps, .astroid, .pinwheelStar, .bigSmall: drawPinwheel(g, inks, pen, self)
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

// MARK: - Woven marks (R–W)
//
// M grown towards flowers: still one even weight and few primitives, but
// more of them woven, and the loops turned into petals. They draw in two
// roles, so the two-tone style can tell them apart: `main` for tiles and
// frames, `second` for the floral strands, and an optional pale `fill`
// washed into the petals. In every other style all three are the one ink
// (and there's no fill).

struct Inks {
  let main: CGColor
  let second: CGColor
  let fill: CGColor?
  init(_ p: Palette) {
    main = p.ink
    second = p.second ?? p.ink
    fill = p.fill
  }
}

/// A closed loop through `n` points of `f`, for t from 0 to 2π.
func loop(_ n: Int = 1440, _ f: (CGFloat) -> CGPoint) -> CGPath {
  let p = CGMutablePath()
  p.addLines(between: (0..<n).map { f(CGFloat($0) / CGFloat(n) * 2 * .pi) })
  p.closeSubpath()
  return p
}

/// A four-lobed loop, r = R(1 + a cos 4θ + b cos 8θ), lobes at `rot` and
/// every quarter turn on. Two with opposite `a` cross eight times, halfway
/// between their lobes: an eight-petal flower.
func lobes(_ R: CGFloat, _ a: CGFloat, _ b: CGFloat = 0, rot: CGFloat = 0) -> CGPath {
  loop { t in
    let r = R * (1 + a * cos(4 * (t - rot)) + b * cos(8 * (t - rot)))
    return CGPoint(x: r * cos(t), y: r * sin(t))
  }
}

/// A petal loop along `angle`, from `r0` to `r1` out, `w` across at its
/// widest. Both ends round; `egg` above 0 swells the outer half.
func petalLoop(angle: CGFloat, from r0: CGFloat, to r1: CGFloat, width w: CGFloat, egg k: CGFloat) -> CGPath {
  let (c, a) = ((r0 + r1) / 2, (r1 - r0) / 2)
  return loop(720) { t in
    let u = c + a * cos(t)
    let v = w / 2 * sin(t) * (1 + k * cos(t)) / (1 + k)
    return CGPoint(x: u * cos(angle) - v * sin(angle), y: u * sin(angle) + v * cos(angle))
  }
}

/// A rounded square centred `c` (off the axes), half-width `h`, as a petal:
/// tight corners `rs` on the diagonal through it, full ones `rb` either side.
func leafTile(_ c: CGPoint, _ h: CGFloat, tight rs: CGFloat, full rb: CGFloat) -> CGPath {
  let (sx, sy): (CGFloat, CGFloat) = (c.x >= 0 ? 1 : -1, c.y >= 0 ? 1 : -1)
  let ks: [(CGFloat, CGFloat)] = [(1, -1), (1, 1), (-1, 1), (-1, -1)]
  func pt(_ k: (CGFloat, CGFloat)) -> CGPoint { CGPoint(x: c.x + k.0 * h, y: c.y + k.1 * h) }
  let p = CGMutablePath()
  p.move(to: CGPoint(x: c.x, y: c.y - h))
  for (i, k) in ks.enumerated() {
    let (a, b) = (pt(k), pt(ks[(i + 1) % 4]))
    p.addArc(tangent1End: a, tangent2End: CGPoint(x: (a.x + b.x) / 2, y: (a.y + b.y) / 2), radius: k.0 * sx == k.1 * sy ? rs : rb)
  }
  p.closeSubpath()
  return p
}

/// A rounded square centred `c` (off the axes), half-width `h`, its inner
/// corner bitten out by a circle of radius `rho` round the centre, as in L.
func notchedTile(_ c: CGPoint, _ h: CGFloat, _ rc: CGFloat, rho: CGFloat) -> CGPath {
  let (a, b) = (abs(c.x) - h, abs(c.x) + h)
  let s = (rho * rho - a * a).squareRoot()
  let p = CGMutablePath()
  p.move(to: CGPoint(x: s, y: a))
  p.addArc(tangent1End: CGPoint(x: b, y: a), tangent2End: CGPoint(x: b, y: (a + b) / 2), radius: rc)
  p.addArc(tangent1End: CGPoint(x: b, y: b), tangent2End: CGPoint(x: (a + b) / 2, y: b), radius: rc)
  p.addArc(tangent1End: CGPoint(x: a, y: b), tangent2End: CGPoint(x: a, y: (a + b) / 2), radius: rc)
  p.addLine(to: CGPoint(x: a, y: s))
  p.addArc(center: .zero, radius: rho, startAngle: atan2(s, a), endAngle: atan2(a, s), clockwise: true)
  p.closeSubpath()
  var t = CGAffineTransform(scaleX: c.x >= 0 ? 1 : -1, y: c.y >= 0 ? 1 : -1)
  return p.copy(using: &t)!
}

/// Washes petals in `color` under strands about to be woven, held a gap
/// clear of every strand so the wash never touches a line. `regions` fills
/// the areas with the current fill colour.
func wash(_ g: CGContext, _ color: CGColor?, clear strands: [CGPath], _ pen: Pen, _ regions: () -> Void) {
  guard let color else { return }
  g.saveGState()
  g.beginTransparencyLayer(auxiliaryInfo: nil)
  g.setFillColor(color)
  regions()
  g.setBlendMode(.clear)
  let clearance = pen.w + 2 * max(pen.gap, pen.pixels(1))
  for s in strands {
    g.fill(Polyline(s).path.copy(strokingWithWidth: clearance, lineCap: .round, lineJoin: .round, miterLimit: 4), rgb(0x000000))
  }
  g.endTransparencyLayer()
  g.restoreGState()
}

/// M's tiles, pixel-snapped from 64 px down as in `drawLinked`: centres
/// (±o, ±o), half-width h, corner radius rc.
func linkedTiles(_ pen: inout Pen) -> (o: CGFloat, h: CGFloat, rc: CGFloat) {
  if pen.lod == 0 { return (0.42, 0.28, 0.1) }
  let (inner, outer, w, corner, gap): (CGFloat, CGFloat, CGFloat, CGFloat, CGFloat) =
    [(2, 20, 3, 3, 1.5), (1, 11, 2, 1.5, 1), (1, 6, 2, 1, 0)][pen.lod - 1]
  pen.w = pen.pixels(w)
  pen.gap = pen.pixels(gap)
  return (pen.pixels(inner + outer) / 2, pen.pixels(outer - inner - w) / 2, pen.pixels(corner))
}

/// Strands drawn plain, without a weave (at 16 px, where there's no room
/// for a gap).
func outline(_ g: CGContext, _ strands: [CGPath], _ ink: CGColor, _ w: CGFloat) {
  for s in strands { g.fill(s.copy(strokingWithWidth: w, lineCap: .round, lineJoin: .round, miterLimit: 4), ink) }
}

/// R (from M): M's four tiles; the ring becomes two four-lobed loops woven
/// through each other, one with its lobes in the tiles, one in the joints.
/// Eight petals, linked through all four tiles.
///
/// At 32 px sixteen crossings are mush, so it's M's 32 px tiles with one
/// four-lobed loop, its lobes reaching into the tiles: the ring in bloom.
/// At 16 px it's M's four bold outlined tiles.
func drawBloom(_ g: CGContext, _ ink: Inks, _ pen: Pen) {
  var pen = pen
  let (o, h, rc) = linkedTiles(&pen)
  let tiles = corners.map { k in roundSquare(CGPoint(x: k.x * o, y: k.y * o), h, rc) }
  let garland: [CGPath]
  switch pen.lod {
  case 3: return outline(g, tiles, ink.main, pen.w)
  case 2: garland = [lobes(pen.pixels(6), 0.32, rot: .pi / 4)]
  default: garland = [lobes(0.4, 0.22, rot: .pi / 4), lobes(0.4, -0.22, rot: .pi / 4)]
  }
  weave(g, tiles + garland, inks: tiles.map { _ in ink.main } + garland.map { _ in ink.second }, pen)
}

/// S: four circles woven into a quatrefoil, inside a rounded tile outline.
/// Two-tone washes the four lens-shaped petals where neighbours overlap.
///
/// From 64 px down the frame is set in whole pixels. At 32 px the circles
/// pull in to clear it by a pixel. At 16 px the weave can't show, so it's a
/// solid quatrefoil in the frame.
func drawQuatrefoil(_ g: CGContext, _ ink: Inks, _ pen: Pen) {
  var pen = pen
  var (d, r, edge, corner, fw): (CGFloat, CGFloat, CGFloat, CGFloat, CGFloat) = (0.24, 0.38, 0.74, 0.2, pen.w)
  if pen.lod > 0 {
    // In pixels from the centre: the frame's centre line, weight and corner;
    // the circles' offset, radius and weight; the gap. The circles never
    // reach the frame, so at 32 px they can be finer than it.
    let (f, w, fc, cd, cr, cw, gap): (CGFloat, CGFloat, CGFloat, CGFloat, CGFloat, CGFloat, CGFloat) =
      [(19, 3, 5, 4.6, 7.4, 3, 1.5), (10, 2, 3, 2.7, 4.6, 1.5, 1), (5, 2, 1.5, 1.8, 1.5, 0, 0)][pen.lod - 1]
    (d, r, edge, corner) = (pen.pixels(cd), pen.pixels(cr), pen.pixels(f), pen.pixels(fc))
    fw = pen.pixels(w)
    pen.w = pen.pixels(cw)
    pen.gap = pen.pixels(gap)
  }
  let circles = edgeMids.map { disc(CGPoint(x: $0.x * d, y: $0.y * d), r) }
  let frame = roundSquare(.zero, edge, corner)
  outline(g, [frame], ink.main, fw)
  if pen.lod == 3 {
    // A solid quatrefoil: the weave can't show at 16 px.
    for c in circles { g.fill(c, ink.second) }
    return
  }
  wash(g, ink.fill, clear: circles, pen) {
    for i in 0..<4 {
      g.saveGState()
      g.addPath(circles[i])
      g.clip()
      g.addPath(circles[(i + 1) % 4])
      g.fillPath()
      g.restoreGState()
    }
  }
  weave(g, circles, ink.second, pen)
}

/// T: eight petal loops round an open centre, each woven through the next,
/// a daisy. Two-tone washes the petals.
///
/// At 32 px the line thins to a pixel and a half so sixteen crossings still
/// show. At 16 px it's eight solid round petals round an open centre.
func drawDaisy(_ g: CGContext, _ ink: Inks, _ pen: Pen) {
  var pen = pen
  if pen.lod == 3 {
    for a in quarter + diagonals { g.fill(oval(angle: a, at: 0.4, length: 0.64, width: 0.26), ink.main) }
    return
  }
  if pen.lod == 2 {
    pen.w = pen.pixels(1.5)
    pen.gap = pen.pixels(1)
  }
  let petals = (quarter + diagonals).map { petalLoop(angle: $0, from: 0.16, to: pen.small ? 0.8 : 0.76, width: 0.42, egg: 0.2) }
  wash(g, ink.fill, clear: petals, pen) {
    for p in petals { g.addPath(p) }
    g.fillPath()
  }
  weave(g, petals, ink.main, pen)
}

/// U (from M): M with each tile drawn as a petal (tight corners on its
/// diagonal, full ones either side) and the ring woven through all four.
/// Two-tone washes the petals.
///
/// Set in whole pixels from 64 px down, like M. At 16 px there's no ring:
/// four bold outlined petals.
func drawPetalTiles(_ g: CGContext, _ ink: Inks, _ pen: Pen) {
  var pen = pen
  var (o, h, rs, rb, r): (CGFloat, CGFloat, CGFloat, CGFloat, CGFloat) = (0.42, 0.29, 0.06, 0.26, 0.4)
  if pen.lod > 0 {
    // As in M: the outlines' inner and outer edges, their weight, the tight
    // and full corners, the ring radius and the gap, in pixels.
    let (inner, outer, w, tight, full, ring, gap): (CGFloat, CGFloat, CGFloat, CGFloat, CGFloat, CGFloat, CGFloat) =
      [(2, 20, 3, 1.5, 7, 11, 1.5), (1, 11, 2, 1, 3.5, 6.5, 1), (1, 6, 2, 0.5, 1.5, 0, 0)][pen.lod - 1]
    o = pen.pixels(inner + outer) / 2
    h = pen.pixels(outer - inner - w) / 2
    (rs, rb, r) = (pen.pixels(tight), pen.pixels(full), pen.pixels(ring))
    pen.w = pen.pixels(w)
    pen.gap = pen.pixels(gap)
  }
  let tiles = corners.map { k in leafTile(CGPoint(x: k.x * o, y: k.y * o), h, tight: rs, full: rb) }
  if pen.lod == 3 { return outline(g, tiles, ink.main, pen.w) }
  let ring = disc(.zero, r)
  wash(g, ink.fill, clear: tiles + [ring], pen) {
    for t in tiles { g.addPath(t) }
    g.fillPath()
  }
  weave(g, tiles + [ring], inks: tiles.map { _ in ink.main } + [ink.second], pen)
}

/// V (from L): L's four tiles, a circle bitten from where they meet, and in
/// it R's eight-petal flower, each tile holding a quarter of it: the lobes
/// on the diagonals reach through the bite and weave with their tile.
///
/// Set in whole pixels from 64 px down. At 32 px any loop through the bites
/// tangles, so the flower comes loose: four solid petals in the circle, one
/// pointing into each tile. At 16 px it is L at 16: four solid tiles round a
/// hole.
func drawFourTiles(_ g: CGContext, _ ink: Inks, _ pen: Pen) {
  if pen.lod == 3 { return drawJoined(g, ink.main, pen) }
  var pen = pen
  var (o, h, rc, rho): (CGFloat, CGFloat, CGFloat, CGFloat) = (0.44, 0.28, 0.1, 0.42)
  if pen.lod > 0 {
    // The outlines' inner and outer edges, weight, corner, bite and gap.
    let (inner, outer, w, corner, bite, gap): (CGFloat, CGFloat, CGFloat, CGFloat, CGFloat, CGFloat) =
      [(2, 20, 3, 3, 10.5, 1.5), (1, 11, 2, 1.5, 6, 1)][pen.lod - 1]
    o = pen.pixels(inner + outer) / 2
    h = pen.pixels(outer - inner - w) / 2
    (rc, rho) = (pen.pixels(corner), pen.pixels(bite))
    pen.w = pen.pixels(w)
    pen.gap = pen.pixels(gap)
  }
  let tiles = corners.map { k in notchedTile(CGPoint(x: k.x * o, y: k.y * o), h, rc, rho: rho) }
  if pen.lod == 2 {
    outline(g, tiles, ink.main, pen.w)
    for a in diagonals {
      g.fill(teardrop(angle: a, tip: pen.pixels(0.3), d: pen.pixels(2.4), r: pen.pixels(1.3)), ink.second)
    }
    return
  }
  let garland = [lobes(0.4, 0.24, rot: .pi / 4), lobes(0.4, -0.24, rot: .pi / 4)]
  weave(g, tiles + garland, inks: tiles.map { _ in ink.main } + garland.map { _ in ink.second }, pen)
}

/// W (from O): O's two rounded squares, woven, as two four-lobed loops: an
/// eight-pointed star whose points are round petals. Two-tone washes the
/// whole flower inside the line.
///
/// At 32 px it's a pixel smaller, at a bolder 2 px. At 16 px it's solid, a
/// little eight-petalled flower with no hole (eight bumps round a hole is a
/// cog).
func drawStarFlower(_ g: CGContext, _ ink: Inks, _ pen: Pen) {
  var pen = pen
  var reach: CGFloat = 0.54
  if pen.lod == 2 {
    (pen.w, pen.gap, reach) = (pen.pixels(2), pen.pixels(1), pen.pixels(6.6))
  }
  let loops = [lobes(reach, 0.3, 0.06), lobes(reach, -0.3, 0.06)]
  if pen.lod == 3 {
    // Solid, with no hole: eight bumps round a hole is a cog.
    let solid = [lobes(pen.pixels(4.2), 0.36, 0.06), lobes(pen.pixels(4.2), -0.36, 0.06)]
    for l in solid { g.fill(l, ink.main) }
    return
  }
  wash(g, ink.fill, clear: loops, pen) {
    for l in loops { g.addPath(l) }
    g.fillPath()
  }
  weave(g, loops, ink.main, pen)
}

// MARK: - Pinwheel marks (X)
//
// the maintainer's favourite of R–W was R's 32 px rendering: four chunky outlined
// tiles and one four-lobed loop through them, woven so each lobe reads as a
// hook and the four hooks as a pinwheel. X is that rendering grown up, so its
// geometry is set in pixels of a 32 px icon (`u32`) at every size. X1–X6 keep
// the spiral exactly and change the tiles round it.

/// One pixel of a 32 px icon, in tile space.
let u32: CGFloat = 1024 / 412 / 32

/// The spiral: R's 32 px loop, its lobes on the diagonals 7.9 px out and its
/// waist on the axes 4.1 px out. The same in every X. At 16 px it opens a
/// little to follow the tiles out half a pixel.
func pinwheelSpiral(_ pen: Pen) -> CGPath {
  pen.lod == 3 ? lobes(6.6 * u32, 0.3, rot: .pi / 4) : lobes(6 * u32, 0.32, rot: .pi / 4)
}

/// A tile outline on its stroke's centre line, from (x0, y0) to (x1, y1) in
/// 32 px pixels, corner radius `r`, turned by `turn` round its own centre.
func tile32(_ x0: CGFloat, _ y0: CGFloat, _ x1: CGFloat, _ y1: CGFloat, r: CGFloat = 1.5, turn: CGFloat = 0) -> CGPath {
  let (cx, cy) = ((x0 + x1) / 2 * u32, (y0 + y1) / 2 * u32)
  let (w, h) = (abs(x1 - x0) * u32, abs(y1 - y0) * u32)
  var t = CGAffineTransform(translationX: cx, y: cy).rotated(by: turn)
  return CGPath(roundedRect: CGRect(x: -w / 2, y: -h / 2, width: w, height: h), cornerWidth: r * u32, cornerHeight: r * u32, transform: &t)
}

/// A closed outline through `points` (x, y and corner radius, in 32 px
/// pixels).
func outline32(_ points: [(CGFloat, CGFloat, CGFloat)]) -> CGPath {
  let pts = points.map { CGPoint(x: $0.0 * u32, y: $0.1 * u32) }
  let n = pts.count
  let p = CGMutablePath()
  p.move(to: CGPoint(x: (pts[n - 1].x + pts[0].x) / 2, y: (pts[n - 1].y + pts[0].y) / 2))
  for i in 0..<n {
    let a = pts[i], b = pts[(i + 1) % n]
    p.addArc(tangent1End: a, tangent2End: CGPoint(x: (a.x + b.x) / 2, y: (a.y + b.y) / 2), radius: points[i].2 * u32)
  }
  p.closeSubpath()
  return p
}

/// The four quarter turns of a tile drawn in the top-right quadrant.
func quarterTurns(_ tile: CGPath) -> [CGPath] {
  quarter.map { a in
    var t = CGAffineTransform(rotationAngle: a)
    return tile.copy(using: &t)!
  }
}

/// The tiles round the spiral, in 32 px pixels on the stroke's centre line.
/// R's tiles run 2 to 10 px out (1 to 11 to the outline's edges), so the
/// spiral crosses each tile's two inner sides about 5 px out. At 16 px every
/// coordinate moves out to the next odd one, so a 1 px line lands on a
/// pixel's centre.
func pinwheelTiles(_ mark: Mark, _ pen: Pen) -> [CGPath] {
  let tiny = pen.lod == 3
  func s(_ v: CGFloat) -> CGFloat {
    guard tiny, v != 0 else { return v }
    return (v < 0 ? -1 : 1) * (2 * ((abs(v) - 1) / 2).rounded(.up) + 1)
  }
  let r: CGFloat = tiny ? 1 : 1.5
  func t(_ x0: CGFloat, _ y0: CGFloat, _ x1: CGFloat, _ y1: CGFloat) -> CGPath { tile32(s(x0), s(y0), s(x1), s(y1), r: r) }
  let (a, b) = (s(2), s(10))  // a quarter tile's inner and outer sides
  switch mark {
  case .pinwheelTiling:
    // Four 7 × 9 rectangles, each running on past the next one's end, so
    // the outline turns like a windmill round the spiral.
    return quarterTurns(t(2, 2, 9, 11))
  case .masterStack:
    // One tall tile holding two lobes, two tiles stacked beside it.
    return [t(-10, -10, -2, 10), t(2, 2, 10, 10), t(2, -10, 10, -2)]
  case .steps:
    // Each tile's inner corner stays put; the tiles grow clockwise from top
    // left, 7 × 7, 8 × 7, 8 × 8, 9 × 8.
    return [t(-9, 2, -2, 9), t(2, 2, 10, 9), t(2, -10, 10, -2), t(-11, -10, -2, -2)]
  case .astroid:
    // Inner corners rounded wide, so the joints open into a four-point star.
    return quarterTurns(outline32([(a, a, tiny ? 3 : 4.5), (a, b, r), (b, b, r), (b, a, r)]))
  case .pinwheelStar:
    // Each tile a touch smaller and turned 15° anticlockwise on its centre:
    // the joints become blades, and the gap a four-point pinwheel star.
    // (Turned the other way, the spiral crosses each tile at a corner.) At
    // 16 px the turn moves a corner under a pixel and blurs every line, so
    // the tiles stay square.
    if tiny { return quarterTurns(t(2, 2, 10, 10)) }
    return quarterTurns(tile32(2.2, 2.2, 9.8, 9.8, r: r, turn: .pi / 12))
  case .bigSmall:
    // Two sizes on the diagonals: 9 × 9 top left and bottom right, their
    // outer corners rounded wide to sit in the icon's own, and 7 × 7 in
    // between.
    let (big, round) = (s(11), tiny ? CGFloat(3) : 4)
    let large = outline32([(a, a, r), (a, big, round / 2), (big, big, round), (big, a, round / 2)])
    let small = t(2, 2, 9, 9)
    var flip = CGAffineTransform(scaleX: -1, y: 1)
    var half = CGAffineTransform(rotationAngle: .pi)
    let tl = large.copy(using: &flip)!
    return [tl, small, tl.copy(using: &half)!, small.copy(using: &half)!]
  default:
    return quarterTurns(t(2, 2, 10, 10))
  }
}

/// X and X1–X6: the spiral and the tiles woven at one weight, a 2 px line and
/// a 1 px gap in a 32 px icon (64 and 32 at 1024). At 16 px the line is one
/// pixel and so is the gap.
func drawPinwheel(_ g: CGContext, _ ink: Inks, _ pen: Pen, _ mark: Mark) {
  var pen = pen
  pen.w = 2 * u32
  pen.gap = pen.lod == 3 ? 2 * u32 : u32
  let tiles = pinwheelTiles(mark, pen)
  // The spiral goes first, so it sets the over and under: every X's hooks
  // turn the same way whatever the tiles. From 64 px up each cut may run
  // most of the way along the strand underneath, so a slanting crossing is
  // cut clean through, with no sliver at its sharp corner. At 32 px and below
  // that sliver is under a pixel, and R's reach keeps X's 32 px R's own.
  let under: CGFloat = pen.small ? 0.6 : 0.9
  if pen.lod == 3 && mark == .pinwheel {
    // X at 16 px: a 1 px curve can't land on the grid, so the hooks smeared
    // into grey and their weave gaps broke the tiles. Instead each tile is
    // whole, with a 2 × 1 px tongue from its centre to the inner side the
    // hook comes in by at 32 px. The tiles turn clockwise like the hooks.
    pen.gap = 0
    weave(g, tiles, ink.main, pen)
    let p = 2 * u32  // one pixel at 16 px
    for tongue in quarterTurns(CGPath(rect: CGRect(x: -4 * p, y: 3 * p, width: 2 * p, height: p), transform: nil)) {
      g.fill(tongue, ink.second)
    }
    return
  }
  weave(g, [pinwheelSpiral(pen)] + tiles, inks: [ink.second] + tiles.map { _ in ink.main }, pen, reach: (0.5, under))
}

// MARK: - Prompt key (Y)
//
// the maintainer's own design (Nov 5, 2026; prompt-mockup.png): a cream glaze tile
// with a pale blue-grey key in the middle and a cobalt `>_` on the key. The
// edge pattern tiles: every corner holds a quarter ring with a quarter dot
// inside it, every edge midpoint a half dot. Lay four tiles side by side and
// the corners make whole rings round whole dots, the edges whole dots. The
// squircle's corner cuts the quarter dot away on the icon itself; the 2 × 2
// check in prompt.png draws the tiles square, so the dots show there.
//
// Three looks: light (AppIcon.icns), dark (AppIconDark.icns, swapped in at
// run time) and clear (review only; it needs Icon Composer to ship).

/// The corner macOS 26 masks every app icon with. Fitted to what IconServices
/// drew for our bundle on macOS 26: a continuous corner of radius 214 on
/// the 824 px body (Apple's older template says 185.4). Drawing the tile with
/// the same corner means the Dock shows the same outline whether it masks the
/// bundle's icon itself or shows the image the app hands it at run time.
let tahoeRadius: CGFloat = 214

enum Look: String, CaseIterable {
  case light, dark, clear
}

/// Y's geometry in tile units (the 824 px body spans -1...1), measured off
/// the mockup: its tile is 219.5 px across, so one mockup pixel is 0.0091.
enum KeyGeometry {
  static let ringIn: CGFloat = 0.32  // corner ring, centred on the tile's corner
  static let ringOut: CGFloat = 0.47
  static let cornerDot: CGFloat = 0.16  // inside the ring; the squircle hides it
  static let edgeDot: CGFloat = 0.10  // half dots at the edge midpoints
  static let keyHalf: CGFloat = 0.515  // the key, a rounded square
  static let keyRadius: CGFloat = 0.14
  static let stroke: CGFloat = 0.146  // the prompt's weight, about the ring's
  // `>`: two arms meeting at the apex, round caps and join; `_` level with
  // the chevron's foot.
  static let chevronX: CGFloat = -0.31
  static let apexX: CGFloat = -0.064
  static let armY: CGFloat = 0.237
  static let barFrom: CGFloat = 0.064
  static let barTo: CGFloat = 0.337
}

struct KeyLook {
  var tile: Palette  // glaze gradient, shadow, sheen, rim, shade
  var pocket: CGColor?  // glaze inside each corner ring, a shade lighter
  var ring: CGColor
  var dot: CGColor
  var keyTop: CGColor
  var keyBottom: CGColor
  var keyShadow: CGFloat
  var keyRim: CGColor  // hairline catching the light along the key's top
  var glyph: CGColor
  /// The mockup's top-left glare on the pattern: the ring in that corner and
  /// the top dot come out paler.
  var glare: CGFloat
}

func keyLook(_ look: Look) -> KeyLook {
  switch look {
  case .light:
    KeyLook(
      tile: Palette(
        groundTop: rgb(0xFEFDFB), groundBottom: rgb(0xEAE3D3),
        ink: rgb(0x1F4AA8), line: rgb(0x1F4AA8), wash: rgb(0xD3D7E1), grout: rgb(0xD8D2C4),
        multiply: false, sheen: 0.35, shadow: 0.5, rim: 0.8, shade: 0.12, hairline: 0.07),
      pocket: rgb(0xFFFFFF, 0.55), ring: rgb(0x1F4AA8, 0.5), dot: rgb(0x1F4AA8),
      keyTop: rgb(0xD9DDE9), keyBottom: rgb(0xC9CDD6), keyShadow: 0.14,
      keyRim: rgb(0xFFFFFF, 0.55), glyph: rgb(0x1F4AA8), glare: 0.5)
  case .dark:
    KeyLook(
      tile: Palette(
        groundTop: rgb(0x30343E), groundBottom: rgb(0x121419),
        ink: rgb(0x849BCF), line: rgb(0x849BCF), wash: rgb(0x343A49), grout: rgb(0x0C0D11),
        multiply: false, sheen: 0.3, shadow: 0.6, rim: 0.45, shade: 0.4),
      pocket: rgb(0x000000, 0.12), ring: rgb(0x849BCF, 0.5), dot: rgb(0x849BCF),
      keyTop: rgb(0x3C4152), keyBottom: rgb(0x2A2F3D), keyShadow: 0.35,
      keyRim: rgb(0xFFFFFF, 0.1), glyph: rgb(0x8AA1D4), glare: 0.12)
  case .clear:
    KeyLook(
      tile: Palette(
        groundTop: rgb(0x8C8F98, 0.82), groundBottom: rgb(0x4C505B, 0.82),
        ink: rgb(0xFFFFFF), line: rgb(0xFFFFFF), wash: rgb(0xFFFFFF, 0.14), grout: rgb(0x3A3D45),
        multiply: false, sheen: 0.4, shadow: 0.45, rim: 0.9, shade: 0.2),
      pocket: rgb(0xFFFFFF, 0.06), ring: rgb(0xFFFFFF, 0.42), dot: rgb(0xFFFFFF, 0.85),
      keyTop: rgb(0xFFFFFF, 0.2), keyBottom: rgb(0xFFFFFF, 0.1), keyShadow: 0.2,
      keyRim: rgb(0xFFFFFF, 0.3), glyph: rgb(0xFFFFFF), glare: 0.25)
  }
}

/// Y's prompt, `>_`, as one fillable outline, in tile units. Below 32 px it
/// gets bolder so it stays at least ~1.6 px wide.
func keyPrompt(px: Int) -> CGPath {
  typealias K = KeyGeometry
  let unit = 412 * CGFloat(px) / 1024  // pixels per tile unit
  let w = max(K.stroke, 1.6 / unit)
  let h = w / 2
  // The chevron's outline by hand: round caps, but a near-sharp point at the
  // apex, as in the mockup (a round join stops 0.03 short of its tip).
  let a = CGPoint(x: K.chevronX, y: K.armY)
  let v = CGPoint(x: K.apexX, y: 0)
  let b = CGPoint(x: K.chevronX, y: -K.armY)
  func direction(_ p: CGPoint, _ q: CGPoint) -> CGPoint {
    let d = hypot(q.x - p.x, q.y - p.y)
    return CGPoint(x: (q.x - p.x) / d, y: (q.y - p.y) / d)
  }
  let u1 = direction(a, v)
  let u2 = direction(v, b)
  let n1 = CGPoint(x: -u1.y, y: u1.x)  // the outer side of each arm
  let n2 = CGPoint(x: -u2.y, y: u2.x)
  let k = h / (1 + n1.x * n2.x + n1.y * n2.y)  // miter offset along n1 + n2
  let outer = CGPoint(x: v.x + k * (n1.x + n2.x), y: v.y + k * (n1.y + n2.y))
  let inner = CGPoint(x: v.x - k * (n1.x + n2.x), y: v.y - k * (n1.y + n2.y))
  let p = CGMutablePath()
  p.move(to: CGPoint(x: a.x - n1.x * h, y: a.y - n1.y * h))
  p.addLine(to: inner)
  p.addLine(to: CGPoint(x: b.x - n2.x * h, y: b.y - n2.y * h))
  p.addArc(center: b, radius: h, startAngle: atan2(-n2.y, -n2.x), endAngle: atan2(n2.y, n2.x), clockwise: false)
  p.addArc(tangent1End: outer, tangent2End: CGPoint(x: a.x + n1.x * h, y: a.y + n1.y * h), radius: 0.5 * h)
  p.addLine(to: CGPoint(x: a.x + n1.x * h, y: a.y + n1.y * h))
  p.addArc(center: a, radius: h, startAngle: atan2(n1.y, n1.x), endAngle: atan2(-n1.y, -n1.x), clockwise: false)
  p.closeSubpath()
  p.addPath(strokeShape([CGPoint(x: K.barFrom, y: -K.armY), CGPoint(x: K.barTo, y: -K.armY)], w))
  return p
}

/// The edge pattern alone: corner rings and dots, edge half dots. Drawn on
/// the tile's square, so whatever clips it decides how much shows.
func drawKeyPattern(_ g: CGContext, _ k: KeyLook, px: Int) {
  typealias K = KeyGeometry
  g.beginTransparencyLayer(auxiliaryInfo: nil)
  for c in corners {
    if let pocket = k.pocket { g.fill(disc(c, K.ringIn), pocket) }
    let band = CGMutablePath()
    band.addPath(disc(c, K.ringOut))
    band.addPath(disc(c, K.ringIn))
    g.setFillColor(k.ring)
    g.addPath(band)
    g.fillPath(using: .evenOdd)
    g.fill(disc(c, K.cornerDot), k.dot)
  }
  // The half dots are 1.3 px across at 32 px and a smudge at 16: from 32 px
  // down the rings carry the pattern alone, as in the mockup's Dock row.
  if px > 32 {
    for m in edgeMids { g.fill(disc(m, K.edgeDot), k.dot) }
  }
  if k.glare > 0 {
    g.setBlendMode(.sourceAtop)
    let cs = CGColorSpace(name: CGColorSpace.sRGB)!
    g.drawRadialGradient(
      CGGradient(
        colorsSpace: cs,
        colors: [rgb(0xFFFFFF, k.glare * 1.1), rgb(0xFFFFFF, k.glare * 0.9), rgb(0xFFFFFF, 0)] as CFArray,
        locations: [0, 0.5, 1])!,
      startCenter: CGPoint(x: -0.55, y: 1.05), startRadius: 0, endCenter: CGPoint(x: -0.55, y: 1.05), endRadius: 1,
      options: [])
    g.setBlendMode(.normal)
  }
  g.endTransparencyLayer()
}

/// Y at 16 px, set in whole pixels: an 8 × 8 key and a one-pixel prompt.
/// Drawn to scale, the prompt's 1 px strokes ran together into a blob.
func drawKey16(_ g: CGContext, _ k: KeyLook) {
  let u: CGFloat = 412 * 16 / 1024  // pixels per tile unit
  // A pixel's square in tile units; col and row count from the top left.
  func px(_ col: Int, _ row: Int, w: Int = 1) -> CGRect {
    CGRect(x: (CGFloat(col) - 8) / u, y: (8 - CGFloat(row) - 1) / u, width: CGFloat(w) / u, height: 1 / u)
  }
  let key = CGRect(x: -4 / u, y: -4 / u, width: 8 / u, height: 8 / u)
  g.fill(CGPath(roundedRect: key, cornerWidth: 1.2 / u, cornerHeight: 1.2 / u, transform: nil), k.keyBottom)
  g.setFillColor(k.glyph)
  // >
  for (col, row) in [(5, 5), (6, 6), (7, 7), (6, 8), (5, 9)] { g.fill(px(col, row)) }
  // _
  g.fill(px(8, 9, w: 3))
}

/// The key and its prompt.
func drawKey(_ g: CGContext, _ k: KeyLook, px: Int) {
  typealias K = KeyGeometry
  if px <= 16 { return drawKey16(g, k) }
  let s = CGFloat(px) / 1024
  let unit = 412 * s
  let key = squircle(CGRect(x: -K.keyHalf, y: -K.keyHalf, width: 2 * K.keyHalf, height: 2 * K.keyHalf), K.keyRadius)
  // A soft shadow under the key. CG shadows are in device pixels.
  g.saveGState()
  g.setShadow(
    offset: CGSize(width: 0, height: -0.022 * unit), blur: max(0.06 * unit, 1),
    color: rgb(0x000000, k.keyShadow * (px <= 32 ? 0.5 : 1)))
  g.fill(key, k.keyBottom)
  g.restoreGState()
  g.saveGState()
  g.addPath(key)
  g.clip()
  let cs = CGColorSpace(name: CGColorSpace.sRGB)!
  g.drawLinearGradient(
    CGGradient(colorsSpace: cs, colors: [k.keyTop, k.keyBottom] as CFArray, locations: [0, 1])!,
    start: CGPoint(x: 0, y: K.keyHalf), end: CGPoint(x: 0, y: -K.keyHalf), options: [])
  if px > 32 {
    // Light catching the key's top edge, fading down its sides.
    g.addPath(key)
    g.setLineWidth(max(0.016, 2 / unit))
    g.replacePathWithStrokedPath()
    g.clip()
    g.drawLinearGradient(
      CGGradient(colorsSpace: cs, colors: [k.keyRim, rgb(0xFFFFFF, 0)] as CFArray, locations: [0, 1])!,
      start: CGPoint(x: 0, y: K.keyHalf), end: CGPoint(x: 0, y: 0), options: [])
  }
  g.restoreGState()
  g.fill(keyPrompt(px: px), k.glyph)
}

/// Y at `px` × `px`: the tile with macOS 26's corner, the glass over it.
func renderKey(_ look: Look, px: Int) -> CGImage {
  let k = keyLook(look)
  return renderTile(k.tile, px: px, bleeds: false, radius: tahoeRadius) { g in
    drawKeyPattern(g, k, px: px)
    drawKey(g, k, px: px)
  }
}

/// Y's face on a plain square, no squircle and no glass: what a tile would
/// look like set in a wall, for the 2 × 2 check.
func keyFace(_ look: Look, px: Int) -> CGImage {
  let k = keyLook(look)
  let g = bitmap(px)
  let cs = CGColorSpace(name: CGColorSpace.sRGB)!
  g.translateBy(x: CGFloat(px) / 2, y: CGFloat(px) / 2)
  g.scaleBy(x: CGFloat(px) / 2, y: CGFloat(px) / 2)
  g.drawLinearGradient(
    CGGradient(colorsSpace: cs, colors: [k.tile.groundTop, k.tile.groundBottom] as CFArray, locations: [0, 1])!,
    start: CGPoint(x: 0, y: 1), end: CGPoint(x: 0, y: -1), options: [])
  // The motif is drawn at the icon's scale (an 824 px body in 1024), so pass
  // the pixel size it would have as an icon.
  let iconPx = Int((CGFloat(px) * 1024 / 824).rounded())
  drawKeyPattern(g, k, px: iconPx)
  drawKey(g, k, px: iconPx)
  return g.makeImage()!
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
func motifLayer(px: Int, radius: CGFloat = 185.4, _ draw: (CGContext) -> Void) -> CGImage {
  let g = bitmap(px)
  let s = CGFloat(px) / 1024
  g.scaleBy(x: s, y: s)
  g.addPath(squircle(CGRect(x: 100, y: 100, width: 824, height: 824), radius))
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
  renderTile(p, px: px, bleeds: bleeds) { g in mark.draw(g, p, Pen(px: px)) }
}

/// The tile around a motif: shadow, glaze, the motif, the glass over it.
/// `radius` is the corner: 185.4 is Apple's Big Sur template, `tahoeRadius`
/// the mask macOS 26 puts on every app icon.
func renderTile(_ p: Palette, px: Int, bleeds: Bool, radius: CGFloat = 185.4, motif: (CGContext) -> Void) -> CGImage {
  let g = CGContext(
    data: nil, width: px, height: px, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
  let s = CGFloat(px) / 1024
  let lod = px <= 32 ? 2 : (px <= 64 ? 1 : 0)
  g.scaleBy(x: s, y: s)
  g.interpolationQuality = .high

  let body = CGRect(x: 100, y: 100, width: 824, height: 824)
  let shape = squircle(body, radius)

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
  let art = motifLayer(px: px, radius: radius, motif)
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
  g.setStrokeColor(rgb(0x000000, p.hairline ?? (p.multiply ? 0.10 : 0.18)))
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

func writeIconset(_ dir: URL, look: Look = .light) {
  try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
  for (name, px) in iconsetSizes {
    writePNG(chosen.image(look, px: px), dir.appendingPathComponent(name))
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
  rows.append(Row(name: chosen.title + " · dark", images: { chosen.image(Look.dark, px: $0) }, ns: nil))
  rows.append(Row(name: chosen.title + (chosen == .key ? " · clear" : " · tinted (mono source)"), images: { chosen.image(Look.clear, px: $0) }, ns: nil))
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
  for (i, s) in Style.minimal.enumerated() {
    label(g, "\(s.rawValue) · \(s.name)", at: CGPoint(x: CGFloat(iconsX + i * (cell + gap)) + 4, y: y - 32), size: 17, color: ink, bold: true)
  }
  for (i, s) in stripStyles.enumerated() {
    label(g, "\(s.rawValue) · 32 and 16 px", at: CGPoint(x: CGFloat(stripX + i * (stripW + stripGap)), y: y - 32), size: 17, color: ink, bold: true)
  }
  y -= CGFloat(colHead)

  for parent in groups {
    label(g, "From \(parent.letter) · \(parent.name)", at: CGPoint(x: CGFloat(margin), y: y - 36), size: 20, color: dim, bold: true)
    y -= CGFloat(groupH)
    for m in Mark.minimal where m.parent == parent {
      let top = y
      label(g, "\(m.letter)  \(m.name)", at: CGPoint(x: CGFloat(margin), y: top - 40), size: 22, color: ink, bold: true)
      label(g, m.blurb, at: CGPoint(x: CGFloat(margin), y: top - 66), size: 15, color: dim)
      for (i, s) in Style.minimal.enumerated() {
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

/// The woven marks: M for reference, then R–W, one row each. Columns: the
/// seven woven styles at 160 px, then the 32 and 16 px renders of styles 1
/// and 3 (glaze and cobalt) at 1:1 and blown up (3× and 4×, nearest
/// neighbour).
func writeWoven(_ url: URL) {
  let cell = 160
  let gap = 18
  let margin = 40
  let labelW = 250
  let header = 84
  let colHead = 44
  let rowH = cell + 34
  let stripStyles: [Style] = [.glaze, .cobalt]
  let stripW = 32 + 10 + 16 + 18 + 96 + 10 + 64
  let stripGap = 30
  let iconsX = margin + labelW
  let styles = Style.woven
  let stripX = iconsX + styles.count * cell + (styles.count - 1) * gap + 44
  let width = stripX + stripStyles.count * stripW + (stripStyles.count - 1) * stripGap + margin
  let height = header + colHead + Mark.wovenSet.count * rowH + 20
  let g = CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
  let ink = NSColor(white: 0.12, alpha: 1)
  let dim = NSColor(white: 0.42, alpha: 1)
  g.setFillColor(rgb(0xECECEF))
  g.fill(CGRect(x: 0, y: 0, width: width, height: height))
  var y = CGFloat(height - 50)
  label(g, "Azulejo app icon · woven marks R–W, grown from M · 160 px in seven palettes, then 32 and 16 px at 1:1 and zoomed", at: CGPoint(x: CGFloat(margin), y: y), size: 22, color: ink, bold: true)
  y = CGFloat(height - header)

  g.setFillColor(rgb(0xC9C9CF))
  g.fill(CGRect(x: CGFloat(margin), y: y - 2, width: CGFloat(width - 2 * margin), height: 2))
  for (i, s) in styles.enumerated() {
    label(g, "\(i + 1) · \(s.name)", at: CGPoint(x: CGFloat(iconsX + i * (cell + gap)) + 2, y: y - 32), size: 15, color: ink, bold: true)
  }
  for (i, s) in stripStyles.enumerated() {
    let n = styles.firstIndex(of: s)! + 1
    label(g, "\(n) · 32 and 16 px", at: CGPoint(x: CGFloat(stripX + i * (stripW + stripGap)), y: y - 32), size: 15, color: ink, bold: true)
  }
  y -= CGFloat(colHead)

  for m in Mark.wovenSet {
    let top = y - 10
    label(g, "\(m.letter)  \(m.name)", at: CGPoint(x: CGFloat(margin), y: top - 60), size: 21, color: ink, bold: true)
    label(g, m == .linked ? "reference: a ring woven through 4 tiles" : m.blurb, at: CGPoint(x: CGFloat(margin), y: top - 84), size: 13, color: dim)
    for (i, s) in styles.enumerated() {
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
      x += 32 + 10
      g.draw(i16, in: CGRect(x: x, y: base - 8, width: 16, height: 16))
      x += 16 + 18
      g.draw(i32, in: CGRect(x: x, y: base - 48, width: 96, height: 96))
      x += 96 + 10
      g.draw(i16, in: CGRect(x: x, y: base - 32, width: 64, height: 64))
    }
    y -= CGFloat(rowH)
  }
  writePNG(g.makeImage()!, url)
}

/// The quick look: the top four woven mark and palette pairs, best first, at
/// 512 on a light and a dark band, with their 32 and 16 px renders under
/// each at 1:1 and 4×.
let wovenPicks: [(Mark, Style)] = [(.petalTiles, .twoTone), (.bloom, .glaze), (.daisy, .cobalt), (.quatrefoil, .indigo)]

func writeWovenHero(_ url: URL) {
  let cell = 512
  let gap = 48
  let margin = 40
  let header = 80
  let labelH = 48
  let bandH = labelH + cell + 26 + 128 + 40
  let width = margin * 2 + wovenPicks.count * cell + (wovenPicks.count - 1) * gap
  let height = header + bandH * 2
  let g = CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
  let bands: [(CGColor, NSColor)] = [(rgb(0xECECEF), NSColor(white: 0.12, alpha: 1)), (rgb(0x1E1F23), NSColor(white: 0.9, alpha: 1))]
  g.setFillColor(bands[0].0)
  g.fill(CGRect(x: 0, y: height - header, width: width, height: header))
  label(g, "Azulejo app icon · woven marks, top four · 512 px, then 32 and 16 px at 1:1 and 4×, on light and dark", at: CGPoint(x: CGFloat(margin), y: CGFloat(height - 52)), size: 24, color: bands[0].1, bold: true)
  let renders = wovenPicks.map { m, s in
    (renderMark(m, palette: s.palette, bleeds: s.bleeds, px: 1024), renderMark(m, palette: s.palette, bleeds: s.bleeds, px: 32), renderMark(m, palette: s.palette, bleeds: s.bleeds, px: 16))
  }
  for (b, (bg, ink)) in bands.enumerated() {
    let top = height - header - b * bandH
    g.setFillColor(bg)
    g.fill(CGRect(x: 0, y: top - bandH, width: width, height: bandH))
    for (i, (m, s)) in wovenPicks.enumerated() {
      let x = CGFloat(margin + i * (cell + gap))
      let iconTop = CGFloat(top - labelH)
      label(g, "\(i + 1).  \(m.letter) · \(m.name), \(s.name)", at: CGPoint(x: x + 4, y: iconTop + 10), size: 22, color: ink, bold: true)
      let (big, i32, i16) = renders[i]
      g.interpolationQuality = .high
      g.draw(big, in: CGRect(x: x, y: iconTop - CGFloat(cell), width: CGFloat(cell), height: CGFloat(cell)))
      let smallTop = iconTop - CGFloat(cell) - 26
      g.interpolationQuality = .none
      var sx = x + 40
      g.draw(i32, in: CGRect(x: sx, y: smallTop - 32 - 48, width: 32, height: 32))
      sx += 32 + 16
      g.draw(i16, in: CGRect(x: sx, y: smallTop - 16 - 56, width: 16, height: 16))
      sx += 16 + 40
      g.draw(i32, in: CGRect(x: sx, y: smallTop - 128, width: 128, height: 128))
      sx += 128 + 24
      g.draw(i16, in: CGRect(x: sx, y: smallTop - 64 - 32, width: 64, height: 64))
    }
  }
  writePNG(g.makeImage()!, url)
}

/// pinwheel.png's colour rows: X and the two picks among X1–X6.
let pinwheelPicks: [Mark] = [.pinwheel, .pinwheelStar, .pinwheelTiling]

/// pinwheel.png: the maintainer's screenshot of R's 32 px rendering beside X's own
/// 32 px and X at full size; then X and X1–X6 at 256 px with 32 and 16 px
/// under each, on light and dark; then X and the two picks in white on
/// cobalt and two-tone blue. The screenshot is `pinwheel-liked-32px.png` in
/// `dir`; without it, the panel shows R's 32 px rendering instead.
func writePinwheel(_ dir: URL) {
  let margin = 40
  let cell = 256
  let gap = 36
  let width = margin * 2 + Mark.pinwheelSet.count * cell + (Mark.pinwheelSet.count - 1) * gap
  let header = 76
  let heroH = 52 + 512 + 40
  let bandH = 44 + cell + 18 + 96 + 34
  let colourH = 52 + bandH
  let height = header + heroH + bandH * 2 + colourH
  let g = CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
  let light = (rgb(0xECECEF), NSColor(white: 0.12, alpha: 1), NSColor(white: 0.42, alpha: 1))
  let darkBand = (rgb(0x1E1F23), NSColor(white: 0.9, alpha: 1), NSColor(white: 0.62, alpha: 1))
  g.setFillColor(light.0)
  g.fill(CGRect(x: 0, y: 0, width: width, height: height))
  var top = CGFloat(height)
  label(g, "Azulejo app icon · X, pinwheel: R's 32 px rendering grown up, and the spiral in six other tilings", at: CGPoint(x: CGFloat(margin), y: top - 50), size: 24, color: light.1, bold: true)
  top -= CGFloat(header)

  // The match: the screenshot, X at 32 px, X at full size, each 512 high.
  let shot = NSImage(contentsOf: dir.appendingPathComponent("pinwheel-liked-32px.png"))
  let x32 = renderMark(.pinwheel, palette: tinGlaze, bleeds: true, px: 32)
  let panels: [(String, String)] = [
    (shot == nil ? "R · linked bloom at 32 px, zoomed" : "the maintainer's screenshot: R at 32 px, zoomed", "the one he liked, from woven-hero.png"),
    ("X at 32 px, zoomed", "the same pixels: X is set in them"),
    ("X at 1024 px", "the same geometry as vectors: 2 px line, 1 px gap, woven"),
  ]
  for (i, (title, note)) in panels.enumerated() {
    let x = CGFloat(margin + i * (512 + 80))
    label(g, title, at: CGPoint(x: x, y: top - 30), size: 19, color: light.1, bold: true)
    label(g, note, at: CGPoint(x: x, y: top - 50), size: 13, color: light.2)
    let rect = CGRect(x: x, y: top - 52 - 512, width: 512, height: 512)
    switch i {
    case 0:
      if let shot {
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: g, flipped: false)
        shot.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: false, hints: [.interpolation: NSImageInterpolation.none])
        NSGraphicsContext.restoreGraphicsState()
      } else {
        g.interpolationQuality = .none
        g.draw(renderMark(.bloom, palette: tinGlaze, bleeds: true, px: 32), in: rect)
      }
    case 1:
      g.interpolationQuality = .none
      g.draw(x32, in: rect)
    default:
      g.interpolationQuality = .high
      g.draw(renderMark(.pinwheel, palette: tinGlaze, bleeds: true, px: 1024), in: rect)
    }
  }
  top -= CGFloat(heroH)

  /// One icon at 256 with its 32 and 16 px renders under it, 1:1 and zoomed.
  func column(_ m: Mark, _ s: Style, x: CGFloat, top: CGFloat, title: String, ink: NSColor) {
    label(g, title, at: CGPoint(x: x + 2, y: top - 30), size: 16, color: ink, bold: true)
    g.interpolationQuality = .high
    g.draw(renderMark(m, palette: s.palette, bleeds: s.bleeds, px: 512), in: CGRect(x: x, y: top - 40 - CGFloat(cell), width: CGFloat(cell), height: CGFloat(cell)))
    let i32 = renderMark(m, palette: s.palette, bleeds: s.bleeds, px: 32)
    let i16 = renderMark(m, palette: s.palette, bleeds: s.bleeds, px: 16)
    let base = top - 40 - CGFloat(cell) - 18
    g.interpolationQuality = .none
    var sx = x + 8
    g.draw(i32, in: CGRect(x: sx, y: base - 32 - 32, width: 32, height: 32))
    sx += 32 + 8
    g.draw(i16, in: CGRect(x: sx, y: base - 16 - 40, width: 16, height: 16))
    sx += 16 + 16
    g.draw(i32, in: CGRect(x: sx, y: base - 96, width: 96, height: 96))
    sx += 96 + 12
    g.draw(i16, in: CGRect(x: sx, y: base - 64 - 16, width: 64, height: 64))
  }

  // Every X, cobalt on glaze, on light and dark.
  for band in [light, darkBand] {
    g.setFillColor(band.0)
    g.fill(CGRect(x: 0, y: top - CGFloat(bandH), width: CGFloat(width), height: CGFloat(bandH)))
    for (i, m) in Mark.pinwheelSet.enumerated() {
      column(m, .glaze, x: CGFloat(margin + i * (cell + gap)), top: top, title: "\(m.letter) · \(m.name)", ink: band.1)
    }
    top -= CGFloat(bandH)
  }

  // The colour alternates.
  label(g, "White on cobalt and two-tone blue: X and the two picks", at: CGPoint(x: CGFloat(margin), y: top - 40), size: 19, color: light.1, bold: true)
  top -= 52
  var i = 0
  for m in pinwheelPicks {
    for s in [Style.cobalt, .twoTone] {
      column(m, s, x: CGFloat(margin + i * (cell + gap)), top: top, title: "\(m.letter) · \(s.name)", ink: light.1)
      i += 1
    }
  }
  writePNG(g.makeImage()!, dir.appendingPathComponent("pinwheel.png"))
}

/// The shipping icon on its own: the 1024 master shown at 256, then 128, 64,
/// 32 and 16 at their real pixel sizes, then 32 and 16 blown up 4× (nearest
/// neighbour), next to Terminal and Notes for scale. Under that, the three
/// side by side at each size, as they'd sit in a Dock. Light left, dark right.
func writeChosen(_ url: URL) {
  typealias Draw = (CGContext, Int, CGRect) -> Void
  func ns(_ path: String) -> Draw? {
    guard FileManager.default.fileExists(atPath: path) else { return nil }
    let image = NSWorkspace.shared.icon(forFile: path)
    return { g, px, rect in
      // Rasterise at the real size first, so a zoom shows the real pixels.
      let small = CGContext(
        data: nil, width: px, height: px, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
      drawNSImage(small, image, in: CGRect(x: 0, y: 0, width: px, height: px))
      g.draw(small.makeImage()!, in: rect)
    }
  }
  let ours: Draw = { g, px, rect in g.draw(chosen.image(chosen.palette, px: px), in: rect) }
  var icons: [(String, Draw)] = [("Azulejo", ours)]
  for (name, path) in [("macOS: Terminal", "/System/Applications/Utilities/Terminal.app"), ("macOS: Notes", "/System/Applications/Notes.app")] {
    if let d = ns(path) { icons.append((name, d)) }
  }
  // The Dock strip puts Azulejo in the middle.
  let strip = icons.count == 3 ? [icons[1], icons[0], icons[2]] : icons

  let sizes = [256, 128, 64, 32, 16]
  let gap = 28
  let labelH = 40
  let stripSizes = [128, 64, 32, 16]
  let stripW = stripSizes.reduce(0) { $0 + 3 * ($1 + max(4, $1 / 8)) + gap }
  let half = max(sizes.reduce(0, +) + 32 * 4 + 16 * 4 + gap * (sizes.count + 3), stripW + 2 * gap)
  let rowH = 256 + labelH + gap
  let stripH = 128 + labelH + gap
  let width = half * 2
  let height = 80 + icons.count * rowH + stripH + gap
  let g = CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
  let bands = [(rgb(0xECECEF), NSColor(white: 0.15, alpha: 1)), (rgb(0x1E1F23), NSColor(white: 0.85, alpha: 1))]
  for (i, (bg, ink)) in bands.enumerated() {
    g.setFillColor(bg)
    g.fill(CGRect(x: i * half, y: 0, width: half, height: height))
    let ox = CGFloat(i * half + gap)
    label(g, "\(chosen.title) · \(i == 0 ? "light" : "dark") · 1024 at 256, then 128 / 64 / 32 / 16 at 1:1, then 32 and 16 at 4×", at: CGPoint(x: ox, y: CGFloat(height - 50)), size: 18, color: ink, bold: true)
    var top = CGFloat(height - 80)
    for (name, draw) in icons {
      label(g, name, at: CGPoint(x: ox, y: top - 30), size: 18, color: ink, bold: true)
      let base = top - CGFloat(labelH) - 256
      var x = ox
      for d in sizes {
        g.interpolationQuality = .high
        draw(g, d == 256 ? 1024 : d, CGRect(x: x, y: base, width: CGFloat(d), height: CGFloat(d)))
        x += CGFloat(d + gap)
      }
      for d in [32, 16] {
        g.interpolationQuality = .none
        draw(g, d, CGRect(x: x, y: base, width: CGFloat(d * 4), height: CGFloat(d * 4)))
        x += CGFloat(d * 4 + gap)
      }
      top -= CGFloat(rowH)
    }
    label(g, "In a row, as in the Dock: 128 / 64 / 32 / 16 at 1:1", at: CGPoint(x: ox, y: top - 30), size: 18, color: ink, bold: true)
    let base = top - CGFloat(labelH) - 128
    var x = ox
    for d in stripSizes {
      for (_, draw) in strip {
        g.interpolationQuality = .high
        draw(g, d, CGRect(x: x, y: base, width: CGFloat(d), height: CGFloat(d)))
        x += CGFloat(d + max(4, d / 8))
      }
      x += CGFloat(gap)
    }
  }
  writePNG(g.makeImage()!, url)
}

// MARK: - Prompt key sheets

func cgImage(_ url: URL) -> CGImage? {
  guard let src = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
  return CGImageSourceCreateImageAtIndex(src, 0, nil)
}

/// A system app's icon rasterised at its real size, or nil if it's missing.
func systemIcon(_ path: String, px: Int) -> CGImage? {
  guard FileManager.default.fileExists(atPath: path) else { return nil }
  let g = bitmap(px)
  drawNSImage(g, NSWorkspace.shared.icon(forFile: path), in: CGRect(x: 0, y: 0, width: px, height: px))
  return g.makeImage()
}

/// Four faces in a 2 × 2 with a grout line, the way tiles sit in a wall.
func keyWall(_ g: CGContext, _ look: Look, at origin: CGPoint, tile: Int, grout: CGFloat) {
  let face = keyFace(look, px: tile)
  let t = CGFloat(tile)
  g.setFillColor(keyLook(look).tile.grout)
  g.fill(CGRect(x: origin.x - grout, y: origin.y - grout, width: 2 * t + 3 * grout, height: 2 * t + 3 * grout))
  for (i, j) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
    g.draw(face, in: CGRect(x: origin.x + CGFloat(i) * (t + grout), y: origin.y + CGFloat(j) * (t + grout), width: t, height: t))
  }
}

/// prompt.png: Y's three looks at every size on light and dark, the 2 × 2
/// wall, then each look on a light and a dark desktop with Terminal and
/// Notes either side.
func writeKeySheet(_ url: URL) {
  let gap = 28
  let shown = 384  // the 1024 master, scaled down
  let sizes = [256, 64, 32, 16]
  let half = gap + shown + sizes.reduce(0) { $0 + $1 + gap } + gap + 32 * 4 + gap + 16 * 4 + gap
  let width = half * 2
  let labelH = 40
  let rowH = labelH + shown + gap
  let wallTile = 220
  let wallH = 60 + 2 * wallTile + 3 * 6 + gap * 2
  let deskH = 360
  let height = 80 + Look.allCases.count * rowH + wallH + 2 * deskH
  let g = CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
  g.interpolationQuality = .high
  let bands = [(rgb(0xECECEF), NSColor(white: 0.15, alpha: 1)), (rgb(0x1E1F23), NSColor(white: 0.85, alpha: 1))]
  var top = CGFloat(height)
  // Sizes, on light (left) and dark (right).
  for (i, (bg, ink)) in bands.enumerated() {
    g.setFillColor(bg)
    g.fill(CGRect(x: i * half, y: 0, width: half, height: height))
    let ox = CGFloat(i * half + gap)
    label(g, "Y · prompt key · \(i == 0 ? "light" : "dark") background · 1024 at 384, then 256 / 64 / 32 / 16 at 1:1, then 32 and 16 at 4×", at: CGPoint(x: ox, y: top - 50), size: 18, color: ink, bold: true)
    var rowTop = top - 80
    for look in Look.allCases {
      let name = look == .clear ? "clear (review only)" : look == .dark ? "dark (AppIconDark.icns, at run time)" : "light (AppIcon.icns)"
      label(g, name, at: CGPoint(x: ox, y: rowTop - 28), size: 18, color: ink, bold: true)
      let base = rowTop - CGFloat(labelH) - CGFloat(shown)
      var x = ox
      g.interpolationQuality = .high
      g.draw(renderKey(look, px: 1024), in: CGRect(x: x, y: base, width: CGFloat(shown), height: CGFloat(shown)))
      x += CGFloat(shown + gap)
      for d in sizes {
        g.draw(renderKey(look, px: d), in: CGRect(x: x, y: base, width: CGFloat(d), height: CGFloat(d)))
        x += CGFloat(d + gap)
      }
      g.interpolationQuality = .none
      for d in [32, 16] {
        g.draw(renderKey(look, px: d), in: CGRect(x: x, y: base, width: CGFloat(d * 4), height: CGFloat(d * 4)))
        x += CGFloat(d * 4 + gap)
      }
      rowTop -= CGFloat(rowH)
    }
  }
  top -= CGFloat(80 + Look.allCases.count * rowH)

  // The wall: four square faces per look, on a neutral band.
  g.setFillColor(rgb(0xD9D9DE))
  g.fill(CGRect(x: 0, y: top - CGFloat(wallH), width: CGFloat(width), height: CGFloat(wallH)))
  label(g, "Tiled 2 × 2: square faces with a grout line, so the corners make whole rings round whole dots and the edges whole dots", at: CGPoint(x: CGFloat(gap), y: top - 44), size: 18, color: NSColor(white: 0.15, alpha: 1), bold: true)
  let wallW = 2 * wallTile + 3 * 6
  let spacing = (width - 3 * wallW) / 4
  for (i, look) in Look.allCases.enumerated() {
    keyWall(g, look, at: CGPoint(x: CGFloat(spacing + i * (wallW + spacing) + 6), y: top - CGFloat(wallH) + CGFloat(gap) + 6), tile: wallTile, grout: 6)
  }
  top -= CGFloat(wallH)

  // Desktops: a light and a dark wallpaper, a Dock-ish shelf on each.
  let terminal = "/System/Applications/Utilities/Terminal.app"
  let notes = "/System/Applications/Notes.app"
  let desks: [(String, CGColor, CGColor, CGColor, NSColor)] = [
    ("Light desktop", rgb(0xDCE3F2), rgb(0xF1EEF6), rgb(0xFFFFFF, 0.45), NSColor(white: 0.15, alpha: 1)),
    ("Dark desktop", rgb(0x10162A), rgb(0x2A3352), rgb(0xFFFFFF, 0.10), NSColor(white: 0.88, alpha: 1)),
  ]
  let cs = CGColorSpace(name: CGColorSpace.sRGB)!
  for (name, low, high, shelf, ink) in desks {
    let rect = CGRect(x: 0, y: top - CGFloat(deskH), width: CGFloat(width), height: CGFloat(deskH))
    g.saveGState()
    g.clip(to: rect)
    g.drawLinearGradient(
      CGGradient(colorsSpace: cs, colors: [high, low] as CFArray, locations: [0, 1])!,
      start: CGPoint(x: 0, y: rect.maxY), end: CGPoint(x: CGFloat(width), y: rect.minY), options: [])
    g.restoreGState()
    label(g, "\(name): Terminal, then Y light, dark and clear, then Notes, at 128 and 64 px", at: CGPoint(x: CGFloat(gap), y: rect.maxY - 44), size: 18, color: ink, bold: true)
    var x = CGFloat(gap)
    for d in [128, 64] {
      let pad = CGFloat(d) / 8
      let n = 5
      let shelfRect = CGRect(x: x, y: rect.minY + 50, width: CGFloat(n) * (CGFloat(d) + pad) + pad, height: CGFloat(d) + 2 * pad)
      g.setFillColor(shelf)
      g.addPath(CGPath(roundedRect: shelfRect, cornerWidth: pad * 2, cornerHeight: pad * 2, transform: nil))
      g.fillPath()
      var ix = x + pad
      let icons: [CGImage?] = [systemIcon(terminal, px: d), renderKey(.light, px: d), renderKey(.dark, px: d), renderKey(.clear, px: d), systemIcon(notes, px: d)]
      for icon in icons {
        if let icon { g.draw(icon, in: CGRect(x: ix, y: shelfRect.minY + pad, width: CGFloat(d), height: CGFloat(d))) }
        ix += CGFloat(d) + pad
      }
      x = shelfRect.maxX + CGFloat(gap * 2)
    }
    top -= CGFloat(deskH)
  }
  writePNG(g.makeImage()!, url)
}

/// prompt-vs-mockup.png: the mockup beside Y at the same scale, with a 50 %
/// onion skin of the two, for the big icon, the 2 × 2 wall and the three
/// looks in a row.
func writeKeyVsMockup(_ dir: URL) {
  guard let mock = cgImage(dir.appendingPathComponent("prompt-mockup.png")) else {
    FileHandle.standardError.write("no prompt-mockup.png in \(dir.path); skipping prompt-vs-mockup.png\n".data(using: .utf8)!)
    return
  }
  let z: CGFloat = 2  // the mockup is @1x; blow both up 2×
  let gap: CGFloat = 32
  // Crops in mockup pixels (top-left origin): the big icon, the wall, the row.
  // The mockup's tile is 219.5 px across, centred at (149, 217.75).
  let mockTile: CGFloat = 219.5
  let big = CGRect(x: 149 - 120, y: 217.75 - 120, width: 240, height: 240)
  let wall = CGRect(x: 302, y: 112, width: 220, height: 220)
  let row = CGRect(x: 130, y: 528, width: 300, height: 88)
  func crop(_ r: CGRect) -> CGImage { mock.cropping(to: r.integral)! }
  let panels: [(String, CGRect)] = [("big icon", big), ("tiled 2 × 2", wall), ("default, dark, clear", row)]
  let width = Int(gap + 3 * (row.width * z + gap))
  let height = Int(gap + 60 + big.height * z + gap + 60 + wall.height * z + gap + 60 + row.height * z + gap)
  let g = CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
  g.setFillColor(rgb(0xE9ECF4))
  g.fill(CGRect(x: 0, y: 0, width: width, height: height))
  let ink = NSColor(white: 0.15, alpha: 1)
  var top = CGFloat(height) - gap

  // Each panel: mockup | ours | onion skin. `ours` draws into a context set
  // up in mockup pixels (y up), over the mockup's own background.
  func panel(_ title: String, _ r: CGRect, _ ours: (CGContext) -> Void) {
    label(g, "\(title): mockup, Y, both at 50 %", at: CGPoint(x: gap, y: top - 30), size: 18, color: ink, bold: true)
    let h = r.height * z
    let base = top - 50 - h
    let m = crop(r)
    let mine = CGContext(
      data: nil, width: Int(r.width * z), height: Int(r.height * z), bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    mine.interpolationQuality = .high
    mine.scaleBy(x: z, y: z)
    mine.translateBy(x: -r.minX, y: r.maxY)
    mine.scaleBy(x: 1, y: -1)  // now in mockup coordinates, y down
    ours(mine)
    let oursImage = mine.makeImage()!
    g.interpolationQuality = .high
    var x = gap
    g.draw(m, in: CGRect(x: x, y: base, width: r.width * z, height: h))
    x += r.width * z + gap
    g.draw(oursImage, in: CGRect(x: x, y: base, width: r.width * z, height: h))
    x += r.width * z + gap
    g.draw(m, in: CGRect(x: x, y: base, width: r.width * z, height: h))
    g.setAlpha(0.5)
    g.draw(oursImage, in: CGRect(x: x, y: base, width: r.width * z, height: h))
    g.setAlpha(1)
    top = base - gap
  }
  /// Draws `image` (y up) into a y-down context with its top-left at `rect`'s.
  func put(_ c: CGContext, _ image: CGImage, _ rect: CGRect) {
    c.saveGState()
    c.translateBy(x: rect.minX, y: rect.maxY)
    c.scaleBy(x: 1, y: -1)
    c.draw(image, in: CGRect(x: 0, y: 0, width: rect.width, height: rect.height))
    c.restoreGState()
  }
  // The icon canvas is 1024/824 of the tile.
  func iconRect(centre: CGPoint, tile: CGFloat) -> CGRect {
    let s = tile * 1024 / 824
    return CGRect(x: centre.x - s / 2, y: centre.y - s / 2, width: s, height: s)
  }
  panel(panels[0].0, panels[0].1) { c in
    c.setFillColor(rgb(0xE9ECF4))  // the mockup's page, under our shadow
    c.fill(big)
    put(c, renderKey(.light, px: Int(mockTile * 1024 / 824 * z)), iconRect(centre: CGPoint(x: 149, y: 217.75), tile: mockTile))
  }
  panel(panels[1].0, panels[1].1) { c in
    // The mockup's wall: 4 tiles of ~104 px with a 3.5 px grout, from (306, 116).
    let t: CGFloat = 104
    c.setFillColor(keyLook(.light).tile.grout)
    c.fill(wall)
    let face = keyFace(.light, px: Int(t * z))
    for (i, j) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
      put(c, face, CGRect(x: 306 + CGFloat(i) * (t + 3.5), y: 116 + CGFloat(j) * (t + 3.5), width: t, height: t))
    }
  }
  panel(panels[2].0, panels[2].1) { c in
    c.setFillColor(rgb(0x2E3342))
    c.fill(row)
    // The mockup's 72 px tiles, centred at x 180, 280, 380 and y 571.
    for (i, look) in Look.allCases.enumerated() {
      put(c, renderKey(look, px: Int(72 * 1024 / 824 * z)), iconRect(centre: CGPoint(x: 180 + CGFloat(i) * 100, y: 571), tile: 72))
    }
  }
  writePNG(g.makeImage()!, dir.appendingPathComponent("prompt-vs-mockup.png"))
}

// MARK: - Icon Composer layers

/// A path in tile units as SVG path data on a full-bleed 1024 canvas: the
/// tile's edge is the canvas's edge, y down.
func svgPath(_ path: CGPath) -> String {
  func f(_ p: CGPoint) -> String {
    String(format: "%.2f %.2f", 512 + 512 * p.x, 512 - 512 * p.y)
  }
  var d: [String] = []
  path.applyWithBlock { e in
    let pts = e.pointee.points
    switch e.pointee.type {
    case .moveToPoint: d.append("M" + f(pts[0]))
    case .addLineToPoint: d.append("L" + f(pts[0]))
    case .addQuadCurveToPoint: d.append("Q" + f(pts[0]) + " " + f(pts[1]))
    case .addCurveToPoint: d.append("C" + f(pts[0]) + " " + f(pts[1]) + " " + f(pts[2]))
    case .closeSubpath: d.append("Z")
    @unknown default: break
    }
  }
  return d.joined()
}

func hex(_ c: CGColor) -> (String, CGFloat) {
  let comps = c.converted(to: CGColorSpace(name: CGColorSpace.sRGB)!, intent: .defaultIntent, options: nil)!.components!
  let rgb = comps.prefix(3).map { String(format: "%02X", Int(($0 * 255).rounded())) }.joined()
  return ("#" + rgb, comps.count > 3 ? comps[3] : 1)
}

/// Y as Icon Composer layers: one SVG per layer, full bleed on a 1024
/// canvas, in the light look's colours. scripts/icon/composer/README.md
/// says how to assemble them and lists every look's colours.
func writeComposer(_ dir: URL) {
  typealias K = KeyGeometry
  try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
  let k = keyLook(.light)
  func svg(_ body: String) -> String {
    "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1024\" height=\"1024\" viewBox=\"0 0 1024 1024\">\n\(body)</svg>\n"
  }
  func fill(_ c: CGColor) -> String {
    let (h, a) = hex(c)
    return a < 1 ? "fill=\"\(h)\" fill-opacity=\"\(String(format: "%.2f", a))\"" : "fill=\"\(h)\""
  }
  let band = CGMutablePath()
  let dots = CGMutablePath()
  for c in corners {
    band.addPath(disc(c, K.ringOut))
    band.addPath(disc(c, K.ringIn))
    dots.addPath(disc(c, K.cornerDot))
  }
  for m in edgeMids { dots.addPath(disc(m, K.edgeDot)) }
  let key = squircle(CGRect(x: -K.keyHalf, y: -K.keyHalf, width: 2 * K.keyHalf, height: 2 * K.keyHalf), K.keyRadius)
  let layers: [(String, String)] = [
    ("1-rings.svg", "  <path fill-rule=\"evenodd\" \(fill(k.ring)) d=\"\(svgPath(band))\"/>\n"),
    ("2-dots.svg", "  <path \(fill(k.dot)) d=\"\(svgPath(dots))\"/>\n"),
    ("3-key.svg", "  <path \(fill(k.keyTop)) d=\"\(svgPath(key))\"/>\n"),
    ("4-prompt.svg", "  <path \(fill(k.glyph)) d=\"\(svgPath(keyPrompt(px: 1024)))\"/>\n"),
  ]
  for (name, body) in layers {
    try! svg(body).write(to: dir.appendingPathComponent(name), atomically: true, encoding: .utf8)
  }
}

func writeDocs(_ dir: URL) {
  try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
  for c in Candidate.allCases {
    writePNG(render(c, palette: c.palette, px: 1024), dir.appendingPathComponent("candidate-\(c.rawValue).png"))
  }
  writePNG(chosen.image(Look.dark, px: 1024), dir.appendingPathComponent("appearance-dark.png"))
  writePNG(
    chosen.image(Look.clear, px: 1024),
    dir.appendingPathComponent(chosen == .key ? "appearance-clear.png" : "appearance-tinted.png"))
  writeSheet(dir.appendingPathComponent("sheet.png"))
  writeChosen(dir.appendingPathComponent("chosen.png"))
  writeOptions(dir.appendingPathComponent("options.png"))
  writeMinimal(dir)
  writeWoven(dir.appendingPathComponent("woven.png"))
  writeWovenHero(dir.appendingPathComponent("woven-hero.png"))
  writePinwheel(dir)
  writeKeySheet(dir.appendingPathComponent("prompt.png"))
  writeKeyVsMockup(dir)
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
  case "--iconset-dark": writeIconset(url, look: .dark)
  case "--docs": writeDocs(url)
  case "--minimal": writeMinimal(url)
  case "--woven":
    try! FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    writeWoven(url.appendingPathComponent("woven.png"))
    writeWovenHero(url.appendingPathComponent("woven-hero.png"))
  case "--chosen":
    try! FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    writeChosen(url.appendingPathComponent("chosen.png"))
  case "--pinwheel":
    try! FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    writePinwheel(url)
  case "--composer": writeComposer(url)
  case "--key":
    try! FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    writeKeySheet(url.appendingPathComponent("prompt.png"))
    writeKeyVsMockup(url)
  default: fatalError("unknown option \(flag)")
  }
}
