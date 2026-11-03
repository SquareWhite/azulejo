import AppKit

// Hand-written helpers on top of the generated Design tokens (Design.swift):
// mappings from app types to tokens. Values themselves live in
// shared/design/tokens.json.

extension Design.Icon {
    /// SF Symbol for a tile kind (tile headers, quick open rows).
    static func `for`(_ kind: TileKind) -> String {
        switch kind {
        case .terminal: terminal
        case .browser: browser
        case .preview: preview
        case .files: files
        }
    }
}

extension Design.Color {
    /// Placeholder body for a tile kind whose view doesn't exist yet.
    static func placeholder(_ kind: TileKind) -> NSColor {
        switch kind {
        case .terminal: kindTerminal
        case .browser: kindBrowser
        case .preview: kindPreview
        case .files: kindFiles
        }
    }
}

// MARK: - Status icons drawn by the app

extension Design {
    /// Icons the token file names `azulejo.*`, drawn here because SF Symbols
    /// has no match for the mockup. `azulejo.ring` is the working state: an
    /// open ring (about 270°) with round caps.
    static func customSymbol(_ name: String, color: NSColor, pointSize: CGFloat) -> NSImage? {
        guard name == "azulejo.ring" else { return nil }
        let side = ceil(pointSize * 1.15)
        let image = NSImage(size: NSSize(width: side, height: side), flipped: false) { rect in
            let line = max(1.4, pointSize * 0.15)
            let radius = (min(rect.width, rect.height) - line) / 2 - 0.5
            let path = NSBezierPath()
            // From 12 o'clock, clockwise, leaving a quarter open at the top-left.
            path.appendArc(withCenter: NSPoint(x: rect.midX, y: rect.midY), radius: radius,
                           startAngle: 90, endAngle: 180, clockwise: true)
            path.lineWidth = line
            path.lineCapStyle = .round
            color.setStroke()
            path.stroke()
            return true
        }
        image.accessibilityDescription = "working"
        return image
    }
}

/// A state icon. The working ring turns slowly while layout motion is on and
/// holds still when it's off or under Reduce Motion. The image lives in a
/// centred sublayer, so it spins about its middle whatever the view's frame.
@MainActor
final class StatusIconView: NSView {
    private let icon = CALayer()
    private(set) var status: Design.Status
    private var size: NSSize = .zero

    init(_ status: Design.Status, image: NSImage?) {
        self.status = status
        super.init(frame: .zero)
        wantsLayer = true
        layer?.addSublayer(icon)
        set(status, image: image)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Automation: whether the ring is turning.
    var isSpinning: Bool { icon.animation(forKey: Self.spinKey) != nil }

    func set(_ status: Design.Status, image: NSImage?) {
        self.status = status
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
        let wanted = status == .working && Motion.active && window != nil
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
