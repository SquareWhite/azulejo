import AppKit

/// The toolbar's ⌘ button, at the row's right end: a click opens quick open
/// (⌘K), centred over the window. Its tooltip names whatever chord
/// `quick_open` is bound to, so config overrides show up. It stays pressed
/// (the accent hairline) while the menu is open.
@MainActor
final class QuickOpenButton: NSView, ThemeAware {
    static let label = "Quick open"
    var onActivate: (() -> Void)?
    private let glyph = NSImageView()
    /// Automation: the tooltip's chord ("⌘K", or "" when unbound).
    private(set) var chordText = ""
    /// Pressed: quick open is showing.
    var active = false {
        didSet { if active != oldValue { Theme.reapply(self) } }
    }
    private var hovered = false {
        didSet { if hovered != oldValue { Theme.reapply(self) } }
    }
    private var tracking: NSTrackingArea?

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.searchField
        layer?.cornerCurve = .continuous
        glyph.imageScaling = .scaleNone
        addSubview(glyph)
        toolTip = Self.label
        Theme.track(self)
        setAccessibilityElement(true)
        setAccessibilityRole(.button)
        setAccessibilityLabel(Self.label)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func applyTheme() {
        let lit = active || hovered
        let fill = active ? Design.Color.tabSelected
            : hovered ? Theme.faded(Design.Color.tabSelected, Design.Alpha.rowHover)
            : Design.Color.tabStrip
        layer?.backgroundColor = fill.cgColor
        // Pressed: the accent hairline inside the edge; else the picker's.
        layer?.borderWidth = active ? 1 : Design.Border.hairline
        layer?.borderColor = (active ? Design.Color.searchBorderActive : Design.Color.tabStripBorder).cgColor
        let tint = Theme.resolved(lit ? Design.Color.tabTextSelected : Design.Color.tabText)
        let config = NSImage.SymbolConfiguration(pointSize: Design.Symbol.quickOpenButton, weight: .medium)
            .applying(.init(paletteColors: [tint]))
        glyph.image = NSImage(systemSymbolName: Design.Icon.command, accessibilityDescription: nil)?
            .withSymbolConfiguration(config)
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    /// The tooltip's chord from the core's keymap (`azj_core_keymap`).
    func apply(keymap: [String: Any]) {
        chordText = Self.chord(for: "quick_open", in: keymap) ?? ""
        toolTip = chordText.isEmpty ? Self.label : "\(Self.label) (\(chordText))"
    }

    /// An action's first index-less chord, pretty ("⌘K"); nil when unbound.
    static func chord(for action: String, in keymap: [String: Any]) -> String? {
        let chord = (keymap["bindings"] as? [[String: Any]] ?? []).first {
            $0["action"] as? String == action && ($0["index"] == nil || $0["index"] is NSNull)
        }?["chord"] as? String
        return chord.map(PaletteFuzzy.prettyChord)
    }

    override func layout() {
        super.layout()
        glyph.frame = bounds
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect],
                                  owner: self, userInfo: nil)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) { hovered = true }
    override func mouseExited(with event: NSEvent) { hovered = false }

    /// The whole button is one click target (not its glyph).
    override func hitTest(_ point: NSPoint) -> NSView? {
        frame.contains(point) ? self : nil
    }
    override func mouseDown(with event: NSEvent) { onActivate?() }
    override var mouseDownCanMoveWindow: Bool { false }
    /// Out of the title bar's drag region: a press opens quick open, never
    /// moves the window as well (see `TabButton`'s).
    @objc(_opaqueRectForWindowMoveWhenInTitlebar)
    private func opaqueRectForWindowMove() -> NSRect { isHidden ? .zero : bounds }
    override func accessibilityPerformPress() -> Bool { onActivate?(); return true }

    // Keyboard focus only with Full Keyboard Access on (Tab moves through
    // controls): the system focus ring shows, and Space or Return presses it.
    override var acceptsFirstResponder: Bool { NSApp.isFullKeyboardAccessEnabled }
    override var canBecomeKeyView: Bool { NSApp.isFullKeyboardAccessEnabled }
    override var focusRingMaskBounds: NSRect { bounds }
    override func drawFocusRingMask() {
        let r = Design.Radius.searchField
        NSBezierPath(roundedRect: bounds, xRadius: r, yRadius: r).fill()
    }
    override func keyDown(with event: NSEvent) {
        // Space or Return.
        if event.keyCode == 49 || event.keyCode == 36 { onActivate?() } else { super.keyDown(with: event) }
    }
}

/// The sidebar toggle by the traffic lights: in the sidebar card's top
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
