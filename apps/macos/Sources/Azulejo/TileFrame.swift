import AppKit

/// A tile view that puts controls in its header strip (F28): the preview
/// tile's Preview | Source toggle, the files tile's filter button.
@MainActor
protocol TileHeaderAccessory {
    var headerAccessory: NSView? { get }
}

/// Every tile in the container: a header strip (kind icon, bold title, muted
/// subtitle — what the core's `tile.header` says) above the tile's own view.
///
/// The container only ever sets this view's frame; it sizes the content below
/// the header in one go, so a terminal's PTY still resizes once per layout
/// change (ADR 0004). Everything a tile view does is forwarded.
@MainActor
class TileFrame: NSView, TileView {
    static let headerHeight: CGFloat = 28

    let content: any TileView
    let header: TileHeaderView
    var tileID: String { content.tileID }
    var kind: TileKind { content.kind }
    var keyContext: String { content.keyContext }

    /// Wraps `content`; tiles that take part in motion get a frame that does too.
    static func wrap(_ content: any TileView) -> TileFrame {
        content is MotionAware ? MotionTileFrame(content) : TileFrame(content)
    }

    init(_ content: any TileView) {
        self.content = content
        header = TileHeaderView(kind: content.kind)
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = 6
        layer?.masksToBounds = true
        if let accessory = (content as? TileHeaderAccessory)?.headerAccessory {
            header.setAccessory(accessory)
        }
        // The header stays above the content, so a terminal's 1:1 motion
        // snapshot (crop style) can't cover it mid-motion.
        addSubview(content)
        addSubview(header)
        updateColors()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { false }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        let h = min(Self.headerHeight, max(newSize.height, 0))
        header.frame = NSRect(x: 0, y: newSize.height - h, width: newSize.width, height: h)
        content.frame = NSRect(x: 0, y: 0, width: newSize.width, height: max(newSize.height - h, 0))
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        updateColors()
    }

    private func updateColors() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        }
    }

    // MARK: TileView

    /// What the content view last got, without the header: header text
    /// changes with every command a terminal runs, which the content doesn't
    /// need to hear about.
    private var forwarded: TileModel?

    /// The model the content view was made with.
    func prime(_ tile: TileModel) {
        header.show(tile)
        var bare = tile
        bare.header = nil
        forwarded = bare
    }

    func update(_ tile: TileModel) {
        header.show(tile)
        var bare = tile
        bare.header = nil
        guard bare != forwarded else { return }
        forwarded = bare
        content.update(tile)
    }

    func setFocused(_ focused: Bool) {
        header.focused = focused
        content.setFocused(focused)
    }

    func tileWillClose() { content.tileWillClose() }
}

/// A frame around a tile that freezes behind a snapshot while it moves. The
/// container speaks in this frame's coordinates; the snapshot lives in the
/// content's, below the header.
@MainActor
final class MotionTileFrame: TileFrame, MotionAware {
    private var aware: MotionAware { content as! MotionAware }

    @discardableResult
    func motionWillStart(freeze: Bool, fill: Bool) -> Bool {
        aware.motionWillStart(freeze: freeze, fill: fill)
    }

    var snapshotFrame: CGRect? { aware.snapshotFrame.map { content.convert($0, to: self) } }
    var snapshotContents: Any? { aware.snapshotContents }

    func placeSnapshot(_ frame: CGRect, cover: CGRect) {
        aware.placeSnapshot(convert(frame, to: content), cover: convert(cover, to: content))
    }

    func revealWhenFresh(clock: MotionClock) { aware.revealWhenFresh(clock: clock) }
    func motionDidEnd(clock: MotionClock) { aware.motionDidEnd(clock: clock) }
    func motionCancelled() { aware.motionCancelled() }
    var surfaceResizes: Int { aware.surfaceResizes }
    var headerInset: CGFloat { header.frame.height }
}

extension TileFrame: AutomatableTile {
    func automate(_ verb: String, _ arg: String) -> String? {
        guard let inner = content as? AutomatableTile else {
            return "the focused tile takes no automation verbs"
        }
        return inner.automate(verb, arg)
    }

    func automationState() -> [String: String] {
        var state = (content as? AutomatableTile)?.automationState() ?? [:]
        state["tile.header.title"] = header.titleText
        state["tile.header.subtitle"] = header.subtitleText
        return state
    }
}

/// The strip across the top of a tile: icon, bold title, " — " and a muted
/// subtitle (truncated in the middle), then the tile's own controls.
@MainActor
final class TileHeaderView: NSView {
    private let icon = NSImageView()
    private let label = NSTextField(labelWithString: "")
    private var accessory: NSView?
    private(set) var titleText = ""
    private(set) var subtitleText = ""

    var focused = false { didSet { if focused != oldValue { render() } } }

    init(kind: TileKind) {
        super.init(frame: .zero)
        wantsLayer = true
        icon.image = NSImage(systemSymbolName: Self.symbol(kind), accessibilityDescription: kind.rawValue)
        icon.symbolConfiguration = .init(pointSize: 11, weight: .regular)
        icon.contentTintColor = .secondaryLabelColor
        label.lineBreakMode = .byTruncatingMiddle
        label.maximumNumberOfLines = 1
        label.cell?.truncatesLastVisibleLine = true
        addSubview(icon)
        addSubview(label)
        updateColors()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func setAccessory(_ view: NSView) {
        accessory?.removeFromSuperview()
        accessory = view
        view.translatesAutoresizingMaskIntoConstraints = true
        addSubview(view)
        needsLayout = true
    }

    /// Shows the core's header for `tile`, or a fallback before it has one.
    func show(_ tile: TileModel) {
        let header = tile.header
        let title = header?.title ?? tile.title ?? tile.kind.rawValue
        let subtitle = header?.subtitle ?? ""
        guard title != titleText || subtitle != subtitleText else { return }
        titleText = title
        subtitleText = subtitle
        toolTip = subtitle.isEmpty ? title : "\(title) — \(subtitle)"
        render()
    }

    private func render() {
        let text = NSMutableAttributedString(string: titleText, attributes: [
            .font: NSFont.systemFont(ofSize: 12, weight: .semibold),
            .foregroundColor: focused ? NSColor.labelColor : NSColor.secondaryLabelColor,
        ])
        if !subtitleText.isEmpty {
            text.append(NSAttributedString(string: " — \(subtitleText)", attributes: [
                .font: NSFont.systemFont(ofSize: 12),
                .foregroundColor: NSColor.tertiaryLabelColor,
            ]))
        }
        label.attributedStringValue = text
    }

    override func layout() {
        super.layout()
        let h = bounds.height
        icon.frame = NSRect(x: 9, y: (h - 14) / 2, width: 14, height: 14)
        var right = bounds.width - 6
        if let accessory {
            let size = accessory.fittingSize
            let w = min(size.width, max(bounds.width * 0.6, 0))
            accessory.frame = NSRect(x: right - w, y: (h - size.height) / 2, width: w, height: size.height)
            right -= w + 6
        }
        let x: CGFloat = 29
        let labelHeight = label.intrinsicContentSize.height
        label.frame = NSRect(x: x, y: (h - labelHeight) / 2, width: max(right - x, 0), height: labelHeight)
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        updateColors()
        render()
    }

    private func updateColors() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        }
    }

    private static func symbol(_ kind: TileKind) -> String {
        switch kind {
        case .terminal: "terminal"
        case .browser: "globe"
        case .preview: "doc.text"
        case .files: "folder"
        }
    }
}
