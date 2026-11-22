import AppKit

// Hand-written helpers on top of the generated Design tokens (Design.swift):
// mappings from app types to tokens. Values themselves live in
// shared/design/tokens.json.

extension Design.Icon {
    /// SF Symbol for a tile kind (tile headers, quick open rows).
    static func `for`(_ kind: TileKind) -> String {
        switch kind {
        case .terminal: terminal
        }
    }
}

extension Design.Color {
    /// Placeholder body for a tile kind whose view doesn't exist yet.
    static func placeholder(_ kind: TileKind) -> NSColor {
        switch kind {
        case .terminal: kindTerminal
        }
    }
}

// MARK: - Status icons drawn by the app

extension Design {
    /// Icons the token file names `azulejo.*`, drawn here because SF Symbols
    /// has no match for the mockup, in a box `pointSize` × 8/7 square (16 for
    /// the sidebar's 14), as the mockup's 16-point viewBox:
    /// - `azulejo.ring`, working: an open ring (about 270°, round caps,
    ///   radius 5.5 of 16) over its full circle at `alpha.workingTrack`.
    /// - `azulejo.idleRing`, idle: a small hollow ring (radius 3.25 of 16).
    /// - `azulejo.idleCircle`, idle in the sidebar and the rail: a thin
    ///   hollow ring as wide as the other states' icons there (13.5 of 16
    ///   across, the working ring's 13 and the filled discs' 14 between), so
    ///   the column lines up and idle reads as an empty slot.
    /// - `azulejo.arc`: the working ring's arc alone, with no circle under
    ///   it (a machine connecting or reconnecting).
    static func customSymbol(_ name: String, color: NSColor, pointSize: CGFloat) -> NSImage? {
        guard ["azulejo.ring", "azulejo.arc", "azulejo.idleRing", "azulejo.idleCircle"].contains(name) else {
            return nil
        }
        let side = (pointSize * 8 / 7).rounded()
        // Resolved now: the image is baked into a layer (StatusIconView makes
        // it again on a theme change).
        let color = Theme.resolved(color)
        let working = name != "azulejo.idleRing" && name != "azulejo.idleCircle"
        let wide = name == "azulejo.idleCircle"
        let circled = name == "azulejo.ring"
        let image = NSImage(size: NSSize(width: side, height: side), flipped: false) { rect in
            let unit = min(rect.width, rect.height) / 16
            let centre = NSPoint(x: rect.midX, y: rect.midY)
            if working {
                let line = max(1.4, unit * 2)
                let radius = unit * 5.5
                if circled {
                    let circle = NSBezierPath()
                    circle.appendArc(withCenter: centre, radius: radius, startAngle: 0, endAngle: 360)
                    circle.lineWidth = line
                    color.withAlphaComponent(color.alphaComponent * Design.Alpha.workingTrack).setStroke()
                    circle.stroke()
                }
                let arc = NSBezierPath()
                // From 12 o'clock, clockwise, leaving a quarter open at the top-left.
                arc.appendArc(withCenter: centre, radius: radius, startAngle: 90, endAngle: 180, clockwise: true)
                arc.lineWidth = line
                arc.lineCapStyle = .round
                color.setStroke()
                arc.stroke()
            } else {
                let ring = NSBezierPath()
                // Wide: the stroke's outer edge at 6.75 of 16.
                let line = wide ? max(1, unit * 1.1) : max(1.1, unit * 1.5)
                ring.appendArc(withCenter: centre, radius: wide ? unit * 6.75 - line / 2 : unit * 3.25,
                               startAngle: 0, endAngle: 360)
                ring.lineWidth = line
                color.setStroke()
                ring.stroke()
            }
            return true
        }
        image.accessibilityDescription = working ? "working" : "idle"
        return image
    }
}

/// A state icon. The working ring turns slowly while layout motion is on and
/// holds still when it's off or under Reduce Motion; `spins` makes any icon
/// turn the same way (a machine's reconnecting arc). The image lives in a
/// centred sublayer, so it spins about its middle whatever the view's frame.
/// The layer holds a baked image, so `image` makes it again on each theme
/// change.
@MainActor
final class StatusIconView: NSView, ThemeAware {
    private let icon = CALayer()
    private(set) var status: Design.Status
    private var size: NSSize = .zero
    private let spins: Bool?
    private let makeImage: () -> NSImage?

    init(_ status: Design.Status, image: @escaping () -> NSImage?, spins: Bool? = nil) {
        self.status = status
        self.spins = spins
        makeImage = image
        super.init(frame: .zero)
        wantsLayer = true
        layer?.addSublayer(icon)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Automation: whether the ring is turning.
    var isSpinning: Bool { icon.animation(forKey: Self.spinKey) != nil }

    func applyTheme() {
        let image = makeImage()
        size = image?.size ?? .zero
        icon.contents = image
        icon.contentsScale = window?.backingScaleFactor ?? 2
        invalidateIntrinsicContentSize()
        needsLayout = true
        updateSpin()
    }

    override var intrinsicContentSize: NSSize { size }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        icon.bounds = CGRect(origin: .zero, size: size)
        icon.position = CGPoint(x: bounds.midX, y: bounds.midY)
        CATransaction.commit()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        icon.contentsScale = window?.backingScaleFactor ?? 2
        updateSpin()
    }

    private static let spinKey = "azulejo.spin"

    private func updateSpin() {
        let wanted = (spins ?? (status == .working)) && Motion.active && window != nil
        if !wanted { icon.removeAnimation(forKey: Self.spinKey); return }
        guard icon.animation(forKey: Self.spinKey) == nil else { return }
        let spin = CABasicAnimation(keyPath: "transform.rotation.z")
        spin.fromValue = 0
        spin.toValue = -2 * Double.pi
        spin.duration = 1.6
        spin.repeatCount = .infinity
        spin.isRemovedOnCompletion = false
        icon.add(spin, forKey: Self.spinKey)
    }
}
