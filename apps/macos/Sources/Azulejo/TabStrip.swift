import AppKit

/// The active workspace's tabs, in the toolbar row (D.4): a rounded pill
/// holding one button per tab (its agents' state icon and its name, the
/// selected one filled) and a "+" that opens a new tab. Tabs share the width
/// they're given, down to `tabFitMinWidth`, with names cut by an ellipsis.
/// The layout kind (2.12) shows in each tab's tooltip; the mockup draws no
/// glyph.
///
/// Overflow (provisional, pending the designer; the mockup has three tabs):
/// past that width the tabs scroll sideways (trackpad, or the wheel), the
/// edges with more tabs fade out, the selected tab scrolls into view, and
/// "+" stays put at the end.
@MainActor
final class TabStripView: NSView {
    static let height = Design.Space.tabStripHeight
    var onSelect: ((String) -> Void)?
    /// "+": a new tab in the active workspace.
    var onAdd: (() -> Void)?
    private var buttons: [TabButton] = []
    private let add = NSButton()
    private var shown: [String] = []
    /// The widest the strip can be; set by the chrome layout before `layout`.
    var maxWidth: CGFloat = .greatestFiniteMagnitude
    /// The tabs scroll in this; "+" sits outside it.
    private let scroll = TabScrollView()
    private let tabs = NSView()
    private let fade = CAGradientLayer()
    /// Scroll the selected tab into view at the next layout.
    private var revealSelected = false

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = Design.Color.tabStrip.cgColor
        layer?.borderColor = Design.Color.tabStripBorder.cgColor
        layer?.borderWidth = Design.Border.hairline
        layer?.cornerRadius = Design.Radius.tabStrip
        // The strip sits under the (transparent) title bar; no inset for it.
        scroll.automaticallyAdjustsContentInsets = false
        scroll.contentInsets = NSEdgeInsetsZero
        scroll.drawsBackground = false
        scroll.hasHorizontalScroller = false
        scroll.hasVerticalScroller = false
        scroll.verticalScrollElasticity = .none
        scroll.horizontalScrollElasticity = .allowed
        scroll.wantsLayer = true
        scroll.documentView = tabs
        scroll.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(
            self, selector: #selector(scrolled), name: NSView.boundsDidChangeNotification, object: scroll.contentView)
        fade.startPoint = CGPoint(x: 0, y: 0.5)
        fade.endPoint = CGPoint(x: 1, y: 0.5)
        addSubview(scroll)
        add.image = NSImage(systemSymbolName: "plus", accessibilityDescription: "New tab")?
            .withSymbolConfiguration(.init(pointSize: 13, weight: .regular))
        add.isBordered = false
        add.contentTintColor = Design.Color.tabAdd
        add.refusesFirstResponder = true
        add.target = self
        add.action = #selector(addClicked)
        add.toolTip = "New tab"
        addSubview(add)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        layer?.backgroundColor = Design.Color.tabStrip.cgColor
        layer?.borderColor = Design.Color.tabStripBorder.cgColor
    }

    /// Automation: each tab's marker, in order (" ● 2" without spaces, or "").
    var markers: [String] { buttons.map(\.markerText) }
    /// Automation: each tab's state icon (`working`, `blocked`, …).
    var statuses: [String] { buttons.map(\.status.rawValue) }
    /// Automation: the first tab, for hit tests.
    var firstTab: NSView? { buttons.first }

    func apply(_ workspace: WorkspaceModel?) {
        let tabs = workspace?.tabs ?? []
        let active = workspace?.tab?.id
        let rollups = tabs.map { AttentionStyle.Rollup($0.tiles) }
        let key = zip(tabs, rollups).map { tab, r in
            "\(tab.id)|\(tab.label)|\(tab.kind)|\(tab.id == active)|\(tab.tiles.count)|\(r.blocked)|\(r.attention)|\(r.working)|\(Self.hasAgent(tab))"
        }
        guard key != shown else { return }
        shown = key
        buttons.forEach { $0.removeFromSuperview() }
        buttons = zip(tabs, rollups).map { tab, rollup in
            let button = TabButton(
                tab: tab, selected: tab.id == active, rollup: rollup,
                status: Self.status(rollup, hasAgent: Self.hasAgent(tab))
            ) { [weak self] in self?.onSelect?(tab.id) }
            self.tabs.addSubview(button)
            return button
        }
        revealSelected = true
        needsLayout = true
        superview?.needsLayout = true
    }

    /// The tab's icon: blocked beats done-but-unseen beats working; a tab
    /// whose agents are all quiet (or that has none) shows the idle ring.
    static func status(_ rollup: AttentionStyle.Rollup, hasAgent: Bool) -> Design.Status {
        if rollup.blocked > 0 { return .blocked }
        if rollup.attention > 0 { return .done }
        if rollup.working > 0 { return .working }
        return .idle
    }

    private static func hasAgent(_ tab: TabModel) -> Bool { tab.tiles.contains { $0.agent != nil } }

    /// Width the strip wants: its tabs at their natural widths, "+", padding.
    var naturalWidth: CGFloat {
        let p = Design.Space.tabStripPadding
        let tabs = buttons.reduce(0) { $0 + $1.naturalWidth } + CGFloat(max(0, buttons.count - 1)) * Design.Space.tabSpacing
        return p + tabs + (buttons.isEmpty ? 0 : Design.Space.tabSpacing) + Design.Space.tabHeight + p
    }

    override func layout() {
        super.layout()
        let p = Design.Space.tabStripPadding
        let h = Design.Space.tabHeight
        let y = (bounds.height - h) / 2
        let addWidth = h
        let spacing = Design.Space.tabSpacing
        let room = max(0, bounds.width - 2 * p - addWidth - (buttons.isEmpty ? 0 : spacing))
        let gaps = CGFloat(max(0, buttons.count - 1)) * spacing
        let widths = Self.fit(buttons.map(\.naturalWidth), into: room - gaps, min: Design.Space.tabFitMinWidth)
        let total = widths.reduce(0, +) + gaps
        scroll.frame = NSRect(x: p, y: 0, width: min(total, room), height: bounds.height)
        tabs.frame = NSRect(x: 0, y: 0, width: total, height: bounds.height)
        var x: CGFloat = 0
        for (button, width) in zip(buttons, widths) {
            button.frame = NSRect(x: x, y: y, width: width, height: h)
            x += width + spacing
        }
        add.frame = NSRect(x: p + scroll.frame.width + (buttons.isEmpty ? 0 : spacing), y: y,
                           width: addWidth, height: h)
        if revealSelected {
            revealSelected = false
            if let selected = buttons.first(where: \.selected) {
                tabs.scrollToVisible(selected.frame.insetBy(dx: -Design.Space.tabFadeWidth, dy: 0))
            }
        }
        updateFade()
    }

    @objc private func scrolled() { updateFade() }

    /// Fades the edges that have more tabs past them; no mask when all fit.
    private func updateFade() {
        let clip = scroll.contentView.bounds
        let left = clip.minX > 0.5, right = clip.maxX < tabs.frame.width - 0.5
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        if left || right, scroll.bounds.width > 0 {
            let f = min(Design.Space.tabFadeWidth / scroll.bounds.width, 0.5)
            let solid = NSColor.black.cgColor, clear = NSColor.clear.cgColor
            fade.frame = scroll.bounds
            fade.colors = [left ? clear : solid, solid, solid, right ? clear : solid]
            fade.locations = [0, NSNumber(value: Double(f)), NSNumber(value: Double(1 - f)), 1]
            if scroll.layer?.mask !== fade { scroll.layer?.mask = fade }
        } else {
            scroll.layer?.mask = nil
        }
        CATransaction.commit()
    }

    /// Automation: whether the tabs overflow, and which edges fade
    /// ("none", "right", "left,right", "left").
    var overflow: String {
        let clip = scroll.contentView.bounds
        let edges = [clip.minX > 0.5 ? "left" : nil, clip.maxX < tabs.frame.width - 0.5 ? "right" : nil]
            .compactMap { $0 }
        return edges.isEmpty ? "none" : edges.joined(separator: ",")
    }

    /// Caps the widest tabs (all at one width) until they fit `room`, never
    /// below `min`; narrow tabs keep their natural width.
    static func fit(_ natural: [CGFloat], into room: CGFloat, min: CGFloat) -> [CGFloat] {
        guard natural.reduce(0, +) > room, let widest = natural.max() else { return natural }
        var lo = min, hi = widest
        for _ in 0..<24 {
            let mid = (lo + hi) / 2
            if natural.reduce(0, { $0 + Swift.min($1, mid) }) > room { hi = mid } else { lo = mid }
        }
        return natural.map { Swift.min($0, floor(lo)) }
    }

    @objc private func addClicked() { onAdd?() }
}

/// The tabs' sideways scroller. A plain mouse wheel (vertical deltas only)
/// scrolls it sideways too; it never scrolls up and down.
@MainActor
private final class TabScrollView: NSScrollView {
    override func scrollWheel(with event: NSEvent) {
        guard abs(event.scrollingDeltaY) > abs(event.scrollingDeltaX), let document = documentView else {
            return super.scrollWheel(with: event)
        }
        let clip = contentView
        let step = event.hasPreciseScrollingDeltas ? event.scrollingDeltaY : event.scrollingDeltaY * 8
        let maxX = max(0, document.frame.width - clip.bounds.width)
        clip.scroll(to: NSPoint(x: min(max(0, clip.bounds.minX - step), maxX), y: 0))
        reflectScrolledClipView(clip)
    }
}

/// One tab in the strip: state icon and name, filled when selected.
@MainActor
final class TabButton: NSView {
    let status: Design.Status
    /// Automation: the rollup's marker text, as the sidebar shows it.
    let markerText: String
    private let handler: () -> Void
    private let icon: StatusIconView
    private let label = NSTextField(labelWithString: "")
    let selected: Bool

    init(tab: TabModel, selected: Bool, rollup: AttentionStyle.Rollup, status: Design.Status,
         handler: @escaping () -> Void) {
        self.status = status
        self.selected = selected
        self.handler = handler
        icon = StatusIconView(status, image: Self.icon(status))
        markerText = rollup.marker?.string ?? ""
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.tab
        layer?.backgroundColor = selected ? Design.Color.tabSelected.cgColor : nil
        addSubview(icon)
        label.stringValue = tab.label
        label.font = selected ? Design.Font.tabSelected : Design.Font.tab
        label.textColor = selected ? Design.Color.tabTextSelected : Design.Color.tabText
        label.lineBreakMode = .byTruncatingTail
        label.cell?.truncatesLastVisibleLine = true
        addSubview(label)
        let count = tab.tiles.count == 1 ? "1 tile" : "\(tab.tiles.count) tiles"
        let layout = tab.kind == .manual ? "manual layout" : "\(Self.layoutName(tab.kind)) layout"
        toolTip = "\(tab.label) · \(count) · \(layout)\(tab.kind.glyph.map { " \($0)" } ?? "")"
        setAccessibilityRole(.button)
        setAccessibilityLabel(tab.label)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// The state icon. Filled symbols (blocked) draw their mark in the strip's
    /// colour on the state colour, as in the mockup, not one flat disc.
    static func icon(_ status: Design.Status) -> NSImage? {
        guard status.symbol.hasSuffix(".fill") else { return Design.StatusIcon.image(status, pointSize: 12) }
        let config = NSImage.SymbolConfiguration(pointSize: 12, weight: .bold)
            .applying(.init(paletteColors: [Design.Color.tabStrip, status.color]))
        return NSImage(systemSymbolName: status.symbol, accessibilityDescription: status.rawValue)?
            .withSymbolConfiguration(config)
    }

    static func layoutName(_ kind: LayoutKind) -> String {
        switch kind {
        case .manual: "Manual"
        case .tile: "Tile"
        case .monocle: "Monocle"
        case .threeColumn: "Three column"
        }
    }

    /// The name's width as drawn, plus the text cell's own padding.
    private var textWidth: CGFloat {
        let font = label.font ?? Design.Font.tab
        return ceil((label.stringValue as NSString).size(withAttributes: [.font: font]).width) + Self.cellSlack
    }
    private static let cellSlack: CGFloat = 5

    var naturalWidth: CGFloat {
        let width = 2 * Design.Space.tabPaddingX + 14 + Design.Space.tabIconGap + textWidth - Self.cellSlack
        return min(max(width, Design.Space.tabMinWidth), Design.Space.tabMaxWidth)
    }

    override func layout() {
        super.layout()
        let px = Design.Space.tabPaddingX
        icon.frame = NSRect(x: px, y: (bounds.height - 14) / 2, width: 14, height: 14)
        // The cell pads its text by ~2 pt: start that much early, end that much late.
        let textX = px + 14 + Design.Space.tabIconGap - 2
        let textHeight = ceil(label.intrinsicContentSize.height)
        label.frame = NSRect(x: textX, y: (bounds.height - textHeight) / 2,
                             width: max(0, bounds.width - textX - px + Self.cellSlack - 2), height: textHeight)
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        layer?.backgroundColor = selected ? Design.Color.tabSelected.cgColor : nil
    }

    /// The whole tab is one click target (not its label or icon).
    override func hitTest(_ point: NSPoint) -> NSView? {
        frame.contains(point) ? self : nil
    }
    override func mouseDown(with event: NSEvent) {}
    override func mouseUp(with event: NSEvent) {
        if bounds.contains(convert(event.locationInWindow, from: nil)) { handler() }
    }
    override var mouseDownCanMoveWindow: Bool { false }
    override func accessibilityPerformPress() -> Bool { handler(); return true }
}
