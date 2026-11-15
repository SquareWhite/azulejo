import AppKit

/// One tile in a Monocle tab's stack, as its tab in the header strip shows it.
@MainActor
struct TileTabItem: Equatable {
    let id: String
    let kind: TileKind
    /// What the tile's header strip says: its title ("zsh", "claude") and
    /// subtitle (a path or a command).
    let title: String
    let subtitle: String
    /// The agent's status icon, as the header strip shows it (nil: none).
    let status: Design.Status?

    init(_ tile: TileModel) {
        id = tile.id
        kind = tile.kind
        title = tile.header?.title ?? tile.title ?? tile.kind.rawValue
        subtitle = tile.header?.subtitle ?? ""
        status = AttentionStyle.status(for: tile)
    }

    /// The tab's short form of the subtitle: a path's last folder ("~" for
    /// home), anything else as it is (the label cuts it at its end).
    var shortSubtitle: String {
        guard subtitle.hasPrefix("/") || subtitle.hasPrefix("~/") else { return subtitle }
        return subtitle.split(separator: "/").last.map(String.init) ?? subtitle
    }

    /// The tooltip: the whole title and subtitle.
    var fullTitle: String { subtitle.isEmpty ? title : "\(title) — \(subtitle)" }

    /// The tab's text: the title, then " — " and the short subtitle.
    var shortTitle: String { shortSubtitle.isEmpty ? title : "\(title) — \(shortSubtitle)" }
}

/// A Monocle tab's tiles, in stack order (the tab's order), and the one on
/// screen. Nil for a tab that isn't Monocle, or holds one tile.
@MainActor
struct TileStack: Equatable {
    let items: [TileTabItem]
    let selected: String

    /// The stack `tab` shows as tabs, if it's a Monocle tab with more than
    /// one tile (and nothing zoomed).
    static func of(_ tab: TabModel) -> TileStack? {
        guard tab.kind == .monocle, tab.zoomed == nil, tab.tiles.count > 1 else { return nil }
        let items = tab.order.compactMap { tab.tile($0) }.map(TileTabItem.init)
        guard items.count > 1 else { return nil }
        return TileStack(items: items, selected: tab.focused)
    }
}

/// The tabs in a Monocle tile's header strip (the title content in
/// Monocle): one per tile in the stack, in order, the focused one filled.
/// Each shows the tile's kind icon, its title and the subtitle's last
/// folder, and its agent's status icon at its right end, as the strip
/// does for the one tile. A click focuses that tile (`tile.focus`, as
/// alt+] and alt+[ do); a right-click opens that tile's menu (Close among
/// it); a drag along the strip moves the tile in the stack
/// (`layout.move_order`). Tooltips carry the whole title.
///
/// Sizing: the tabs share the strip's width evenly. Below
/// `headerTabMinWidth` each, the strip keeps as many as fit at that width,
/// the selected one always among them (the window moves only as far as it
/// must to keep it), and a "+N" chip at the right end opens a menu of the
/// rest. No close button: the workspace tabs in the top bar have none
/// either; Close is in the tab's menu.
@MainActor
final class TileTabStripView: NSView {
    /// A tab was clicked: its tile id.
    var onSelect: ((String) -> Void)?
    /// A tab was dragged to another place: its tile id and how many places
    /// it moved (negative: towards the front).
    var onMove: ((String, Int) -> Void)?
    /// A tab's right-click menu, by tile id.
    var onMenu: ((String) -> NSMenu?)?

    private(set) var stack: TileStack?
    private var buttons: [TileTabButton] = []
    private let more = TileTabMoreButton()
    /// The first tab shown when they don't all fit.
    private var windowStart = 0
    /// How many tabs show (all of them unless they overflow).
    private(set) var shownCount = 0
    private var tabWidth: CGFloat = 0

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        more.target = self
        more.action = #selector(moreClicked)
        addSubview(more)
        setAccessibilityRole(.tabGroup)
        setAccessibilityLabel("Tiles in this tab")
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { false }

    /// Shows `stack`'s tiles. Buttons are kept by tile id, so a reorder
    /// moves them and a focus change only refills them.
    func apply(_ stack: TileStack) {
        if drag != nil {
            deferred = stack
            return
        }
        guard stack != self.stack else { return }
        self.stack = stack
        var byID = Dictionary(uniqueKeysWithValues: buttons.map { ($0.item.id, $0) })
        buttons = stack.items.map { item in
            let button = byID.removeValue(forKey: item.id) ?? makeButton(item)
            button.set(item, selected: item.id == stack.selected)
            return button
        }
        for leftover in byID.values { leftover.removeFromSuperview() }
        needsLayout = true
    }

    private func makeButton(_ item: TileTabItem) -> TileTabButton {
        let button = TileTabButton(item)
        button.strip = self
        addSubview(button, positioned: .below, relativeTo: more)
        return button
    }

    // MARK: Layout

    override func layout() {
        super.layout()
        layoutTabs()
    }

    private func layoutTabs() {
        let n = buttons.count
        guard n > 0 else { more.isHidden = true; shownCount = 0; return }
        let s = Design.Space.headerTabSpacing, minW = Design.Space.headerTabMinWidth
        let even = (bounds.width - s * CGFloat(n - 1)) / CGFloat(n)
        var k = n
        var room = bounds.width
        if even < minW, n > 1 {
            room = max(0, bounds.width - Design.Space.headerTabMoreWidth - s)
            k = min(n - 1, max(1, Int((room + s) / (minW + s))))
        }
        shownCount = k
        let selected = buttons.firstIndex { $0.selected } ?? 0
        // The window moves as little as it can to keep the selected tab in it.
        if selected < windowStart { windowStart = selected }
        if selected >= windowStart + k { windowStart = selected - k + 1 }
        windowStart = min(max(0, windowStart), n - k)
        tabWidth = max(0, (room - s * CGFloat(k - 1)) / CGFloat(k))
        for (i, button) in buttons.enumerated() {
            let slot = i - windowStart
            button.isHidden = slot < 0 || slot >= k
            guard !button.isHidden, drag?.button !== button else { continue }
            button.frame = slotFrame(slot)
        }
        if let drag { placeForDrag(drag, animated: false) }
        more.isHidden = k == n
        if !more.isHidden {
            more.count = n - k
            let h = Design.Space.headerTabHeight
            more.frame = NSRect(x: bounds.width - Design.Space.headerTabMoreWidth, y: (bounds.height - h) / 2,
                                width: Design.Space.headerTabMoreWidth, height: h)
            more.toolTip = overflowed.map(\.item.fullTitle).joined(separator: "\n")
        }
    }

    /// Where the tab in place `slot` (0: the first one shown) sits; whole
    /// points, the last one taking what rounding leaves.
    private func slotFrame(_ slot: Int) -> NSRect {
        let s = Design.Space.headerTabSpacing, h = Design.Space.headerTabHeight
        let x = (CGFloat(slot) * (tabWidth + s)).rounded()
        let next = (CGFloat(slot + 1) * (tabWidth + s)).rounded() - s
        return NSRect(x: x, y: ((bounds.height - h) / 2).rounded(), width: max(0, next - x), height: h)
    }

    /// The tabs the "+N" chip stands for, in stack order.
    private var overflowed: [TileTabButton] { buttons.filter(\.isHidden) }

    // MARK: Overflow

    /// The "+N" chip's menu: the tabs it stands for, each focusing its tile.
    func moreMenu() -> NSMenu {
        let menu = NSMenu()
        for button in overflowed {
            let item = NSMenuItem(title: button.item.shortTitle, action: #selector(pickHidden(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = button.item.id
            item.toolTip = button.item.fullTitle
            item.image = NSImage(systemSymbolName: Design.Icon.for(button.item.kind),
                                 accessibilityDescription: button.item.kind.rawValue)
            menu.addItem(item)
        }
        return menu
    }

    @objc private func moreClicked() {
        moreMenu().popUp(positioning: nil, at: NSPoint(x: 0, y: -2), in: more)
    }

    @objc private func pickHidden(_ sender: NSMenuItem) {
        if let id = sender.representedObject as? String { onSelect?(id) }
    }

    // MARK: Drag to reorder

    /// How far a press moves before it's a drag, as the top tabs.
    static let dragThreshold = TabStripView.dragThreshold
    /// How far past the strip a drop still lands; farther, it cancels.
    static let dropMargin = TabStripView.dropMargin

    private struct Press {
        let button: TileTabButton
        let start: NSPoint
    }

    private struct Drag {
        let button: TileTabButton
        let from: Int
        var to: Int
        let grab: CGFloat
        var x: CGFloat
        var outside = false
    }

    private var press: Press?
    private var drag: Drag?
    /// A stack that arrived mid-drag; it goes in when the drag ends.
    private var deferred: TileStack?

    /// Every tab on show, and none hidden: reordering needs them all.
    private var canDrag: Bool { buttons.count > 1 && shownCount == buttons.count }

    fileprivate func tabPressed(_ button: TileTabButton, _ event: NSEvent) {
        guard drag == nil, !event.modifierFlags.contains(.control) else { press = nil; return }
        press = Press(button: button, start: event.locationInWindow)
    }

    fileprivate func tabDragged(_ button: TileTabButton, _ event: NSEvent) {
        guard let press, press.button === button else { return }
        let point = event.locationInWindow
        if drag == nil {
            let dx = point.x - press.start.x, dy = point.y - press.start.y
            guard canDrag, dx * dx + dy * dy >= Self.dragThreshold * Self.dragThreshold,
                  let from = buttons.firstIndex(where: { $0 === button })
            else { return }
            let start = convert(press.start, from: nil)
            drag = Drag(button: button, from: from, to: from, grab: start.x - button.frame.minX, x: button.frame.minX)
            button.lifted = true
        }
        moveDrag(to: point)
    }

    /// True when the release ended a drag (so the tab doesn't also select).
    fileprivate func tabReleased(_ button: TileTabButton, _ event: NSEvent) -> Bool {
        defer { press = nil }
        guard let drag, drag.button === button else { return false }
        moveDrag(to: event.locationInWindow)
        endDrag(commit: !(self.drag?.outside ?? true))
        return true
    }

    private func moveDrag(to point: NSPoint) {
        guard var drag else { return }
        let p = convert(point, from: nil)
        let width = drag.button.frame.width
        drag.x = min(max(0, p.x - drag.grab), max(0, bounds.width - width))
        let zone = bounds.insetBy(dx: -Self.dropMargin.width, dy: -Self.dropMargin.height)
        drag.outside = !zone.contains(p)
        let step = tabWidth + Design.Space.headerTabSpacing
        let middle = p.x - drag.grab + width / 2
        let to = drag.outside || step <= 0
            ? drag.from : min(max(0, Int(floor(middle / step))), buttons.count - 1)
        let reflow = to != drag.to || drag.outside != self.drag?.outside
        drag.to = to
        self.drag = drag
        drag.button.alphaValue = drag.outside ? TabStripView.outsideAlpha : 1
        placeForDrag(drag, animated: reflow)
    }

    /// The others in their slots with the dragged tab taken out and put in
    /// at its landing place; the dragged one where the pointer has it.
    private func placeForDrag(_ drag: Drag, animated: Bool) {
        var order = buttons
        order.removeAll { $0 === drag.button }
        order.insert(drag.button, at: min(drag.to, order.count))
        var moves: [(NSView, NSRect)] = []
        for (slot, button) in order.enumerated() where button !== drag.button {
            let frame = slotFrame(slot)
            if button.frame != frame { moves.append((button, frame)) }
        }
        var lifted = drag.button.frame
        lifted.origin.x = drag.x
        drag.button.frame = lifted
        slide(moves, animated: animated)
    }

    private func slide(_ moves: [(NSView, NSRect)], animated: Bool) {
        guard animated, Motion.active, window != nil, !moves.isEmpty else {
            for (view, frame) in moves { view.frame = frame }
            return
        }
        NSAnimationContext.runAnimationGroup { context in
            context.duration = TabStripView.slideDuration
            context.timingFunction = Motion.timing
            context.allowsImplicitAnimation = true
            for (view, frame) in moves { view.animator().frame = frame }
        }
    }

    private func endDrag(commit: Bool) {
        guard let drag else { return }
        self.drag = nil
        drag.button.alphaValue = 1
        let to = commit ? drag.to : drag.from
        if to != drag.from {
            // The order the core will send back: nothing to rebuild then.
            let button = buttons.remove(at: drag.from)
            buttons.insert(button, at: to)
            if let stack {
                var items = stack.items
                let item = items.remove(at: drag.from)
                items.insert(item, at: to)
                self.stack = TileStack(items: items, selected: stack.selected)
            }
        }
        let moves = buttons.enumerated().map { (slot, button) in (button as NSView, slotFrame(slot)) }
            .filter { $0.0.frame != $0.1 }
        slide(moves, animated: true)
        let settle = Motion.active ? TabStripView.slideDuration : 0
        DispatchQueue.main.asyncAfter(deadline: .now() + settle) { [weak button = drag.button] in
            button?.lifted = false
        }
        if to != drag.from { onMove?(drag.button.item.id, to - drag.from) }
        if let stack = deferred {
            deferred = nil
            apply(stack)
        }
    }

    /// Ends a drag without moving anything (the strip leaves the screen).
    func cancelDrag() {
        press = nil
        if drag != nil { endDrag(commit: false) }
    }

    override func viewWillMove(toWindow newWindow: NSWindow?) {
        super.viewWillMove(toWindow: newWindow)
        if newWindow == nil { cancelDrag() }
    }

    // MARK: Automation

    /// The tabs, in stack order.
    var tabButtons: [TileTabButton] { buttons }
    /// The "+N" chip (hidden when every tab shows).
    var moreButton: NSView { more }
    /// The drag's phase: `idle`, `pressed`, `dragging` or `outside`.
    var dragPhase: String {
        if let drag { return drag.outside ? "outside" : "dragging" }
        return press == nil ? "idle" : "pressed"
    }
}

/// One tile's tab: kind icon, title and the subtitle's last folder (centred
/// in the tab), and the agent's status icon at its right end. Filled when
/// it's the tile on screen; a softer fill under the pointer.
@MainActor
final class TileTabButton: NSView, ThemeAware {
    private(set) var item: TileTabItem
    private(set) var selected = false
    weak var strip: TileTabStripView?
    private let icon = NSImageView()
    private let label = NSTextField(labelWithString: "")
    private var status: StatusIconView?
    private var hovering = false
    private var tracking: NSTrackingArea?
    private var labelNatural = CGSize.zero

    /// Lifted by a drag: filled, with a shadow, over the others.
    var lifted = false {
        didSet {
            guard lifted != oldValue else { return }
            layer?.zPosition = lifted ? 1 : 0
            applyTheme()
        }
    }

    init(_ item: TileTabItem) {
        self.item = item
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.headerTab
        icon.symbolConfiguration = .init(pointSize: Design.Symbol.header, weight: .regular)
        label.lineBreakMode = .byTruncatingTail
        label.maximumNumberOfLines = 1
        label.cell?.truncatesLastVisibleLine = true
        addSubview(icon)
        addSubview(label)
        setAccessibilityRole(.radioButton)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Takes `item` and the selection; redraws only what changed.
    func set(_ item: TileTabItem, selected: Bool) {
        let first = icon.image == nil
        guard first || item != self.item || selected != self.selected else { return }
        let kindChanged = first || item.kind != self.item.kind
        self.item = item
        self.selected = selected
        if kindChanged {
            icon.image = NSImage(systemSymbolName: Design.Icon.for(item.kind), accessibilityDescription: item.kind.rawValue)
        }
        if status?.status != item.status {
            status?.removeFromSuperview()
            status = item.status.map { s in
                let view = StatusIconView(s) { AttentionOverlayView.headerIcon(s) }
                view.toolTip = s.rawValue
                addSubview(view)
                return view
            }
        }
        toolTip = item.fullTitle
        setAccessibilityLabel(item.fullTitle)
        setAccessibilityValue(selected)
        restyle()
    }

    func applyTheme() { restyle() }

    private func restyle() {
        let text = NSMutableAttributedString(string: item.title, attributes: [
            .font: selected ? Design.Font.headerTitle : Design.Font.headerSubtitle,
            .foregroundColor: selected ? Design.Color.textPrimary : Design.Color.textSecondary,
        ])
        if !item.shortSubtitle.isEmpty {
            // Muted is too faint on the selected fill: secondary there
            // (contrast.json, header.tab.path.selected).
            text.append(NSAttributedString(string: " — \(item.shortSubtitle)", attributes: [
                .font: Design.Font.headerSubtitle,
                .foregroundColor: selected ? Design.Color.textSecondary : Design.Color.textMuted,
            ]))
        }
        label.attributedStringValue = text
        labelNatural = text.size()
        icon.contentTintColor = Design.Color.textSecondary
        // theme: restyle() is applyTheme().
        effectiveAppearance.performAsCurrentDrawingAppearance {
            let fill: NSColor? = selected || lifted ? Design.Color.headerTabSelected
                : hovering ? Design.Color.headerTabSelected.withAlphaComponent(Design.Alpha.rowHover) : nil
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            layer?.backgroundColor = fill?.cgColor
            layer?.shadowColor = Design.Color.shadow.cgColor
            layer?.shadowOpacity = lifted ? 0.28 : 0
            layer?.shadowRadius = 4
            layer?.shadowOffset = CGSize(width: 0, height: -1)
            CATransaction.commit()
        }
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let px = Design.Space.headerTabPaddingX, gap = Design.Space.headerTabIconGap
        let side: CGFloat = 14, h = bounds.height
        var right = bounds.width - px
        if let status {
            status.frame = NSRect(x: right - side, y: (h - side) / 2, width: side, height: side)
            right -= side + gap
        }
        // Too narrow for a few letters: the icon alone, centred.
        let room = right - px - side - gap
        label.isHidden = room < 16
        let labelWidth = label.isHidden ? 0 : min(ceil(labelNatural.width) + 4, room)
        let group = side + (label.isHidden ? 0 : gap + labelWidth)
        let left = max(px, min((bounds.width - group) / 2, right - group))
        icon.frame = NSRect(x: left, y: (h - side) / 2, width: side, height: side)
        let labelHeight = ceil(labelNatural.height)
        label.frame = NSRect(x: left + side + gap, y: (h - labelHeight) / 2, width: labelWidth, height: labelHeight)
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect],
                                  owner: self, userInfo: nil)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) { hover(true) }
    override func mouseExited(with event: NSEvent) { hover(false) }

    private func hover(_ inside: Bool) {
        guard inside != hovering else { return }
        hovering = inside
        restyle()
    }

    /// The whole tab is one click target.
    override func hitTest(_ point: NSPoint) -> NSView? {
        !isHidden && frame.contains(point) ? self : nil
    }

    override func mouseDown(with event: NSEvent) { strip?.tabPressed(self, event) }
    override func mouseDragged(with event: NSEvent) { strip?.tabDragged(self, event) }
    override func mouseUp(with event: NSEvent) {
        if strip?.tabReleased(self, event) == true { return }
        if bounds.contains(convert(event.locationInWindow, from: nil)) { strip?.onSelect?(item.id) }
    }

    override func accessibilityPerformPress() -> Bool { strip?.onSelect?(item.id); return true }
    override func menu(for event: NSEvent) -> NSMenu? { strip?.onMenu?(item.id) }
}

/// "+N" at the strip's right end when not every tab fits: a menu of the rest.
@MainActor
final class TileTabMoreButton: NSButton, ThemeAware {
    var count = 0 {
        didSet { if count != oldValue { restyle() } }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        isBordered = false
        refusesFirstResponder = true
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.headerTab
        setAccessibilityLabel("More tiles")
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func applyTheme() { restyle() }

    private func restyle() {
        attributedTitle = NSAttributedString(string: "+\(count)", attributes: [
            .font: Design.Font.headerSubtitle, .foregroundColor: Design.Color.textSecondary,
        ])
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }
}

/// `tile tabs …` in automation scripts, on the focused tile's header strip
/// in a Monocle tab: mouse events made with `NSEvent` and handed straight to
/// a tab's own mouseDown / mouseDragged / mouseUp, as AppKit would deliver
/// them. Nothing goes through the window server.
///
///     tile tabs click N        click tab N (1-based, stack order)
///     tile tabs drag N M       press tab N, drag it over tab M's middle, let go
///     tile tabs more N         pick the Nth entry of the "+N" chip's menu
///                              (built as a click builds it, not shown)
///
/// State (`TileFrame.automationState`): `tile.header.content` (title, tabs or
/// override), `tile.tabs` (how many; 0 without tabs), `tile.tabs.selected`
/// (1-based), `tile.tabs.titles` ("a|b|c", as the tabs show them),
/// `tile.tabs.tips` (their tooltips), `tile.tabs.shown` (how many show),
/// `tile.tabs.window` (the first and last shown, "2-4"),
/// `tile.tabs.overflow` (the "+N" chip's N, 0 without it),
/// `tile.tabs.statuses` (each tab's status icon, `-` for none),
/// `tile.tabs.drag` (idle, pressed, dragging, outside), `tile.tabs.widths`
/// and `tile.tabs.even` (the shown tabs' widths, all the same), and
/// `tile.header.opacity` (the title and the tabs as they show right now,
/// mid-fade included: "title=0.00 tabs=1.00") and `tile.header.fading`
/// (yes partway through the cross-fade between them).
@MainActor
enum TileTabAutomation {
    static func run(_ arg: String, strip: TileTabStripView?) -> String? {
        guard let strip, !strip.isHidden, strip.window != nil else { return "no tile tabs on the focused tile" }
        let words = arg.split(separator: " ").map(String.init)
        let tabs = strip.tabButtons
        func tab(_ i: Int) -> TileTabButton? {
            guard words.count > i, let n = Int(words[i]), tabs.indices.contains(n - 1) else { return nil }
            return tabs[n - 1]
        }
        func middle(_ view: NSView) -> NSPoint { view.convert(NSPoint(x: view.bounds.midX, y: view.bounds.midY), to: nil) }
        switch words.first ?? "" {
        case "click":
            guard let target = tab(1) else { return "click needs a tab number (1–\(tabs.count))" }
            guard !target.isHidden else { return "tab \(words[1]) is behind the +N chip" }
            let at = middle(target)
            send(.leftMouseDown, to: target, at: at)
            send(.leftMouseUp, to: target, at: at)
        case "drag":
            guard let lifted = tab(1), let over = tab(2) else { return "drag needs two tab numbers" }
            let start = middle(lifted), end = middle(over)
            let x = end.x + (end.x > start.x ? 3 : -3)
            send(.leftMouseDown, to: lifted, at: start)
            for i in 1...6 {
                let t = CGFloat(i) / 6
                send(.leftMouseDragged, to: lifted, at: NSPoint(x: start.x + (x - start.x) * t, y: start.y))
            }
            send(.leftMouseUp, to: lifted, at: NSPoint(x: x, y: start.y))
        case "more":
            let menu = strip.moreMenu()
            guard words.count > 1, let n = Int(words[1]), (1...menu.numberOfItems).contains(n) else {
                return "more needs an entry number (1–\(menu.numberOfItems))"
            }
            menu.performActionForItem(at: n - 1)
        default:
            return "tile tabs needs click N|drag N M|more N"
        }
        return nil
    }

    private static func send(_ type: NSEvent.EventType, to view: TileTabButton, at point: NSPoint) {
        guard let window = view.window, let event = NSEvent.mouseEvent(
            with: type, location: point, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1,
            pressure: type == .leftMouseUp ? 0 : 1)
        else { return }
        switch type {
        case .leftMouseDown: view.mouseDown(with: event)
        case .leftMouseDragged: view.mouseDragged(with: event)
        default: view.mouseUp(with: event)
        }
    }

    static func state(_ header: TileHeaderView) -> [String: String] {
        var s = [
            "tile.header.content": header.contentKind,
            "tile.header.opacity": header.contentOpacity,
            "tile.header.fading": header.contentFading ? "yes" : "no",
        ]
        guard let strip = header.tileTabs, header.showsTabs else {
            s["tile.tabs"] = "0"
            return s
        }
        let tabs = strip.tabButtons
        let shown = tabs.enumerated().filter { !$0.element.isHidden }.map(\.offset)
        s["tile.tabs"] = "\(tabs.count)"
        s["tile.tabs.selected"] = "\((tabs.firstIndex { $0.selected } ?? -1) + 1)"
        s["tile.tabs.titles"] = tabs.map(\.item.shortTitle).joined(separator: "|")
        s["tile.tabs.tips"] = tabs.map { $0.toolTip ?? "" }.joined(separator: "|")
        s["tile.tabs.shown"] = "\(shown.count)"
        s["tile.tabs.window"] = shown.isEmpty ? "-" : "\(shown.first! + 1)-\(shown.last! + 1)"
        s["tile.tabs.overflow"] = "\(tabs.count - shown.count)"
        s["tile.tabs.statuses"] = tabs.map { $0.item.status?.rawValue ?? "-" }.joined(separator: ",")
        s["tile.tabs.drag"] = strip.dragPhase
        // Each shown tab's width, and whether they're all the same (± 1 pt).
        let widths = tabs.filter { !$0.isHidden }.map { Int($0.frame.width.rounded()) }
        s["tile.tabs.widths"] = widths.map(String.init).joined(separator: ",")
        s["tile.tabs.even"] = (widths.max() ?? 0) - (widths.min() ?? 0) <= 1 ? "yes" : "no"
        return s
    }
}
