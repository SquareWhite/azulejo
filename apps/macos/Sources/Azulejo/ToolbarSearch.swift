import AppKit

/// The toolbar's "Jump to workspace or tile" field (D.8). It isn't a text
/// field: clicking it opens quick open (⌘K) as a dropdown over it, and the
/// typing happens in quick open's own field. The key badge shows whatever
/// chord `quick_open` is bound to, so config overrides show up.
@MainActor
final class ToolbarSearchView: NSView, ThemeAware {
    static let placeholder = "Jump to workspace or tile"
    var onActivate: (() -> Void)?
    private let magnifier = NSImageView()
    private let text = NSTextField(labelWithString: placeholder)
    private let badge = NSTextField(labelWithString: "")
    private let badgeBack = NSView()
    /// Automation: the badge's text ("⌘K", or "" when unbound).
    private(set) var chordText = ""
    /// Drawn with the accent hairline while quick open is showing.
    var active = false {
        didSet { if active != oldValue { Theme.reapply(self) } }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.searchField
        layer?.borderWidth = Design.Border.hairline
        magnifier.image = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: Design.Symbol.searchIcon, weight: .medium))
        magnifier.contentTintColor = Design.Color.searchText
        addSubview(magnifier)
        text.font = Design.Font.search
        text.textColor = Design.Color.searchText
        text.lineBreakMode = .byTruncatingTail
        addSubview(text)
        badgeBack.wantsLayer = true
        badgeBack.layer?.cornerRadius = Design.Radius.searchBadge
        addSubview(badgeBack)
        badge.font = Design.Font.searchBadge
        badge.textColor = Design.Color.searchBadgeText
        badge.alignment = .center
        badgeBack.addSubview(badge)
        Theme.track(self)
        setAccessibilityRole(.button)
        setAccessibilityLabel(Self.placeholder)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func applyTheme() {
        layer?.backgroundColor = Design.Color.searchField.cgColor
        layer?.borderColor = (active ? Design.Color.searchBorderActive : Design.Color.searchBorder).cgColor
        badgeBack.layer?.backgroundColor = Design.Color.searchBadge.cgColor
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    /// Picks the badge's chord from the core's keymap (`azj_core_keymap`).
    func apply(keymap: [String: Any]) {
        let chord = (keymap["bindings"] as? [[String: Any]] ?? []).first {
            $0["action"] as? String == "quick_open" && ($0["index"] == nil || $0["index"] is NSNull)
        }?["chord"] as? String
        chordText = chord.map(PaletteFuzzy.prettyChord) ?? ""
        badge.stringValue = chordText
        badgeBack.isHidden = chordText.isEmpty
        toolTip = chordText.isEmpty ? "Quick open" : "Quick open (\(chordText))"
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let h = bounds.height
        magnifier.frame = NSRect(x: 10, y: (h - 14) / 2, width: 14, height: 14)
        let badgeHeight = Design.Space.searchBadgeHeight
        let badgeWidth = chordText.isEmpty ? 0 : ceil(badge.intrinsicContentSize.width) + 12
        badgeBack.frame = NSRect(x: bounds.width - 7 - badgeWidth, y: (h - badgeHeight) / 2,
                                 width: badgeWidth, height: badgeHeight)
        let labelHeight = ceil(badge.intrinsicContentSize.height)
        badge.frame = NSRect(x: 0, y: (badgeHeight - labelHeight) / 2, width: badgeWidth, height: labelHeight)
        let textX: CGFloat = 32
        let textHeight = ceil(text.intrinsicContentSize.height)
        let textRight = chordText.isEmpty ? bounds.width - 8 : badgeBack.frame.minX - 6
        text.frame = NSRect(x: textX, y: (h - textHeight) / 2, width: max(0, textRight - textX), height: textHeight)
    }

    /// The whole field is one click target (not its label or badge).
    override func hitTest(_ point: NSPoint) -> NSView? {
        frame.contains(point) ? self : nil
    }
    override func mouseDown(with event: NSEvent) { onActivate?() }
    override var mouseDownCanMoveWindow: Bool { false }
    override func accessibilityPerformPress() -> Bool { onActivate?(); return true }
}

/// The sidebar toggle by the traffic lights (D.2): in the sidebar card's top
/// right while the sidebar shows, after the traffic lights while it's hidden.
@MainActor
final class SidebarToggleButton: NSButton {
    init(action handler: @escaping () -> Void) {
        self.handler = handler
        super.init(frame: .zero)
        image = NSImage(systemSymbolName: "sidebar.left", accessibilityDescription: "Toggle sidebar")?
            .withSymbolConfiguration(.init(pointSize: Design.Symbol.sidebarToggle, weight: .regular))
        isBordered = false
        contentTintColor = Design.Color.titlebarIcon
        refusesFirstResponder = true
        target = self
        self.action = #selector(clicked)
        toolTip = "Show or hide the sidebar"
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private let handler: () -> Void
    @objc private func clicked() { handler() }
    override var mouseDownCanMoveWindow: Bool { false }
}
