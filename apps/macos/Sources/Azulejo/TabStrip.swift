import AppKit

/// The active workspace's tabs, in the toolbar row (D.4): a rounded pill
/// holding one button per tab (its agents' state icon and its name, the
/// selected one filled) and a "+" that opens a new tab. Tabs share the width
/// they're given, down to `tabMinWidth`, with names cut by an ellipsis. The
/// layout kind (2.12) shows in each tab's tooltip; the mockup draws no glyph.
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

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = Design.Color.tabStrip.cgColor
        layer?.borderColor = Design.Color.tabStripBorder.cgColor
        layer?.borderWidth = Design.Border.hairline
        layer?.cornerRadius = Design.Radius.tabStrip
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
            addSubview(button)
            return button
        }
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
        let room = bounds.width - 2 * p - addWidth - (buttons.isEmpty ? 0 : spacing)
            - CGFloat(max(0, buttons.count - 1)) * spacing
        let widths = Self.fit(buttons.map(\.naturalWidth), into: room, min: Design.Space.tabMinWidth)
        var x = p
        for (button, width) in zip(buttons, widths) {
            button.frame = NSRect(x: x, y: y, width: width, height: h)
            x += width + spacing
        }
        add.frame = NSRect(x: x, y: y, width: addWidth, height: h)
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

/// One tab in the strip: state icon and name, filled when selected.
@MainActor
final class TabButton: NSView {
    let status: Design.Status
    /// Automation: the rollup's marker text, as the sidebar shows it.
    let markerText: String
    private let handler: () -> Void
    private let icon = NSImageView()
    private let label = NSTextField(labelWithString: "")
    private let selected: Bool

    init(tab: TabModel, selected: Bool, rollup: AttentionStyle.Rollup, status: Design.Status,
         handler: @escaping () -> Void) {
        self.status = status
        self.selected = selected
        self.handler = handler
        markerText = rollup.marker?.string ?? ""
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.tab
        layer?.backgroundColor = selected ? Design.Color.tabSelected.cgColor : nil
        icon.image = Self.icon(status)
        icon.imageScaling = .scaleProportionallyDown
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
