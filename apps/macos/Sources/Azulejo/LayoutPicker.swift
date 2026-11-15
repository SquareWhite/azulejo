import AppKit

/// The layout picker in the toolbar row, left of the search field: one
/// segment per layout kind (Manual, Master (Tile), Three Column, Monocle),
/// the active tab's kind filled like a selected tab. It follows the tab live,
/// whatever changed the layout (alt+[ / alt+], ⌘K, herdr). Picking a kind
/// runs its `toggle_layout_*` action, as its key does, so motion and the
/// herdr mirror behave the same; picking the active kind does nothing.
/// Each tooltip names the kind and its chord, from the keymap.
@MainActor
final class LayoutPickerView: NSView, ThemeAware {
    static let width = 2 * Design.Space.layoutPickerPadding
        + CGFloat(LayoutKind.picker.count) * Design.Space.layoutPickerSegmentWidth
    var onPick: ((LayoutKind) -> Void)?
    private let segments: [LayoutSegment]
    /// The active tab's kind; nil with no tab (the picker hides).
    private(set) var kind: LayoutKind?
    private var chords: [LayoutKind: String] = [:]

    override init(frame: NSRect) {
        segments = LayoutKind.picker.map(LayoutSegment.init)
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.searchField
        layer?.borderWidth = Design.Border.hairline
        for segment in segments {
            segment.handler = { [weak self] kind in self?.onPick?(kind) }
            addSubview(segment)
        }
        setAccessibilityRole(.radioGroup)
        setAccessibilityLabel("Layout")
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func applyTheme() {
        layer?.backgroundColor = Design.Color.tabStrip.cgColor
        layer?.borderColor = Design.Color.tabStripBorder.cgColor
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    /// The active tab's kind (nil: no tab).
    func apply(_ kind: LayoutKind?) {
        guard kind != self.kind else { return }
        let had = self.kind != nil
        self.kind = kind
        for segment in segments { segment.selected = segment.kind == kind }
        // Showing or hiding changes the toolbar row's layout.
        if had != (kind != nil) { superview?.needsLayout = true }
    }

    /// Each kind's `toggle_layout_*` chord from the core's keymap, for the
    /// tooltips (config overrides show up; an unbound kind shows none).
    func apply(keymap: [String: Any]) {
        let bindings = keymap["bindings"] as? [[String: Any]] ?? []
        chords = [:]
        for kind in LayoutKind.picker {
            let chord = bindings.first { $0["action"] as? String == kind.toggleAction }?["chord"] as? String
            chords[kind] = chord.map(PaletteFuzzy.prettyChord)
        }
        for segment in segments { segment.chord = chords[segment.kind] }
    }

    override func layout() {
        super.layout()
        let p = Design.Space.layoutPickerPadding, w = Design.Space.layoutPickerSegmentWidth
        for (i, segment) in segments.enumerated() {
            segment.frame = NSRect(x: p + CGFloat(i) * w, y: p, width: w, height: max(0, bounds.height - 2 * p))
        }
    }

    override var mouseDownCanMoveWindow: Bool { false }

    // MARK: Automation

    /// The segment for `kind`, for hit tests and `picker KIND`.
    func segment(_ kind: LayoutKind) -> NSView? { segments.first { $0.kind == kind } }
    /// Each segment's tooltip, "|"-separated.
    var tooltips: String { segments.map { $0.toolTip ?? "" }.joined(separator: "|") }
    /// Picks `kind` as a click on its segment does.
    func press(_ kind: LayoutKind) { segments.first { $0.kind == kind }?.press() }
}

/// One kind in the picker: its icon, filled when it's the tab's kind.
@MainActor
private final class LayoutSegment: NSView, ThemeAware {
    let kind: LayoutKind
    var handler: ((LayoutKind) -> Void)?
    private let icon = NSImageView()
    var selected = false {
        didSet {
            guard selected != oldValue else { return }
            setAccessibilityValue(selected)
            Theme.reapply(self)
        }
    }
    var chord: String? {
        didSet { toolTip = chord.map { "\(kind.title) layout (\($0))" } ?? "\(kind.title) layout" }
    }

    init(_ kind: LayoutKind) {
        self.kind = kind
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.searchField - Design.Space.layoutPickerPadding
        icon.imageScaling = .scaleProportionallyDown
        addSubview(icon)
        toolTip = "\(kind.title) layout"
        setAccessibilityRole(.radioButton)
        setAccessibilityLabel("\(kind.title) layout")
        setAccessibilityValue(false)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func applyTheme() {
        layer?.backgroundColor = selected ? Design.Color.tabSelected.cgColor : nil
        // Hierarchical: the panes in the text colour, the frame round them lighter.
        let colour = Theme.resolved(selected ? Design.Color.tabTextSelected : Design.Color.tabText)
        let config = NSImage.SymbolConfiguration(pointSize: Design.Symbol.layoutPicker, weight: .regular)
            .applying(.init(hierarchicalColor: colour))
        icon.image = NSImage(systemSymbolName: kind.symbol, accessibilityDescription: kind.title)?
            .withSymbolConfiguration(config)
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    override func layout() {
        super.layout()
        icon.frame = bounds.insetBy(dx: 4, dy: 3)
    }

    func press() { handler?(kind) }

    /// The whole segment is one click target.
    override func hitTest(_ point: NSPoint) -> NSView? { frame.contains(point) ? self : nil }
    override func mouseDown(with event: NSEvent) {}
    override func mouseUp(with event: NSEvent) {
        if bounds.contains(convert(event.locationInWindow, from: nil)) { press() }
    }
    override var mouseDownCanMoveWindow: Bool { false }
    override func accessibilityPerformPress() -> Bool { press(); return true }
}

/// `picker KIND` (manual, tile, three_column, monocle): picks a kind as a
/// click on its segment does. State: `picker.kind` (the selected kind, or
/// `-` with no tab or while the picker is hidden), `picker.tips`.
@MainActor
enum LayoutPickerAutomation {
    static func run(_ arg: String, controller: WindowController?) -> String? {
        guard let controller else { return "no window" }
        guard let kind = LayoutKind(rawValue: arg) else { return "picker manual|tile|three_column|monocle" }
        guard !controller.layoutPicker.isHidden else { return "the picker is hidden" }
        controller.layoutPicker.press(kind)
        return nil
    }

    static func state(_ controller: WindowController) -> [String: String] {
        let picker = controller.layoutPicker
        return [
            "picker.kind": picker.isHidden ? "-" : picker.kind?.rawValue ?? "-",
            "picker.tips": picker.tooltips,
        ]
    }
}
