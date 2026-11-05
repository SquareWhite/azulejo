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
///
/// Drag to reorder (Nov 5 feedback): press a tab and move past
/// `dragThreshold` and it lifts (filled, with a shadow) and follows the
/// pointer along the strip while the others slide aside; letting go puts it
/// there through the core's `tab.move`, so the API, the CLI and the saved
/// session see the same order. Escape, or letting go away from the strip,
/// puts everything back. A press that stays under the threshold is a plain
/// click (it selects on mouse up, as before). The slides follow
/// `[animation]` and Reduce Motion; near a faded edge of an overflowing
/// strip the tabs scroll along.
@MainActor
final class TabStripView: NSView, ThemeAware {
    static let height = Design.Space.tabStripHeight
    var onSelect: ((String) -> Void)?
    /// A dropped tab: its id and its new index in the workspace (`tab.move`).
    var onMove: ((String, Int) -> Void)?
    /// "+": a new tab in the active workspace.
    var onAdd: (() -> Void)?
    /// A tab's right-click menu, by tab id (`ContextMenus.tabMenu`).
    var onMenu: ((String) -> NSMenu?)?
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
            .withSymbolConfiguration(.init(pointSize: Design.Symbol.tabAdd, weight: .regular))
        add.isBordered = false
        add.contentTintColor = Design.Color.tabAdd
        add.refusesFirstResponder = true
        add.target = self
        add.action = #selector(addClicked)
        add.toolTip = "New tab"
        addSubview(add)
        Theme.track(self)
    }

    func applyTheme() {
        layer?.backgroundColor = Design.Color.tabStrip.cgColor
        layer?.borderColor = Design.Color.tabStripBorder.cgColor
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    /// Automation: each tab's marker, in order (" ● 2" without spaces, or "").
    var markers: [String] { buttons.map(\.markerText) }
    /// Automation: each tab's state icon (`working`, `blocked`, …).
    var statuses: [String] { buttons.map(\.status.rawValue) }
    /// Automation: the first tab, for hit tests.
    var firstTab: NSView? { buttons.first }
    /// Automation: the tabs, in order (`menu tab N`).
    var tabButtons: [TabButton] { buttons }
    /// The selected tab's hint (its tooltip: name, tile count, layout kind).
    var selectedHint: String { buttons.first { $0.selected }?.toolTip ?? "" }

    func apply(_ workspace: WorkspaceModel?) {
        // Mid-drag the tabs stay as they are; the newest model goes in when
        // the drag ends (`endDrag`).
        if drag != nil {
            deferred = .some(workspace)
            return
        }
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
            button.menuProvider = { [weak self] in self?.onMenu?(tab.id) }
            button.strip = self
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

    /// The narrowest the strip gets: the selected tab (its icon at the
    /// least) and "+". The search field gives way before the strip goes below it.
    var minimumWidth: CGFloat {
        let p = Design.Space.tabStripPadding, h = Design.Space.tabHeight
        return p + (buttons.isEmpty ? 0 : h + Design.Space.tabSpacing) + h + p
    }

    /// Too narrow to scroll through tabs: only the selected one shows.
    private var selectedOnly = false

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
        // No room for even one tab at `tabFitMinWidth` (the minimum window
        // size): the selected tab alone, shrunk, down to its icon. The others
        // are a ⌘K or ctrl+number away.
        if let selected = buttons.first(where: \.selected),
           room < min(Design.Space.tabFitMinWidth, selected.naturalWidth) {
            // Nothing to reorder against any more.
            if drag != nil { cancelDrag(animated: false) }
            selectedOnly = true
            for button in buttons { button.isHidden = button !== selected }
            let width = max(min(selected.naturalWidth, room), h)
            scroll.frame = NSRect(x: p, y: 0, width: width, height: bounds.height)
            tabs.frame = NSRect(x: 0, y: 0, width: width, height: bounds.height)
            selected.frame = NSRect(x: 0, y: y, width: width, height: h)
            scroll.contentView.scroll(to: .zero)
            scroll.reflectScrolledClipView(scroll.contentView)
            add.frame = NSRect(x: p + width + spacing, y: y, width: addWidth, height: h)
            revealSelected = false
            updateFade()
            return
        }
        selectedOnly = false
        for button in buttons { button.isHidden = false }
        let gaps = CGFloat(max(0, buttons.count - 1)) * spacing
        let widths = Self.fit(buttons.map(\.naturalWidth), into: room - gaps, min: Design.Space.tabFitMinWidth)
        let total = widths.reduce(0, +) + gaps
        scroll.frame = NSRect(x: p, y: 0, width: min(total, room), height: bounds.height)
        tabs.frame = NSRect(x: 0, y: 0, width: total, height: bounds.height)
        slotWidths = widths
        for (button, frame) in slotFrames() { button.frame = frame }
        add.frame = NSRect(x: p + scroll.frame.width + (buttons.isEmpty ? 0 : spacing), y: y,
                           width: addWidth, height: h)
        if revealSelected, drag == nil {
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
            // theme: a mask's alpha, not a colour anyone sees.
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
    /// ("none", "right", "left,right", "left"), or "selected" when only the
    /// selected tab shows.
    var overflow: String {
        if selectedOnly { return "selected" }
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

    // MARK: Drag to reorder

    /// How far the pointer moves (any direction) before a press on a tab
    /// becomes a drag: a click that wobbles a little still just selects.
    static let dragThreshold: CGFloat = 4
    /// How far past the strip's edges a drop still lands in it; farther out,
    /// the tabs go back to where they were and letting go cancels.
    static let dropMargin = NSSize(width: 24, height: 18)
    /// The lifted tab, faded while the pointer is away from the strip.
    static let outsideAlpha: CGFloat = 0.5

    /// A press on a tab, until mouse up.
    private struct Press {
        let button: TabButton
        /// Where it went down, window coordinates.
        let start: NSPoint
        /// Escape cancelled its drag: mouse up neither drops nor selects.
        var cancelled = false
    }

    /// A tab lifted and following the pointer.
    private struct Drag {
        let button: TabButton
        /// Its index when it lifted, and where it would land now.
        let from: Int
        var to: Int
        /// Pointer x minus the tab's left edge when it lifted, so it doesn't jump.
        let grab: CGFloat
        /// Its left edge in the tabs' coordinates (clamped to the strip).
        var x: CGFloat = 0
        /// The pointer is away from the strip: a drop there cancels.
        var outside = false
        /// The pointer, window coordinates (autoscroll moves the tab under it).
        var pointer: NSPoint
    }

    private var press: Press?
    private var drag: Drag?
    /// Each tab's width at the last layout, in `buttons` order.
    private var slotWidths: [CGFloat] = []
    /// A model that arrived mid-drag (`apply`); `.some(nil)` is "no workspace".
    private var deferred: WorkspaceModel??
    private var autoscroll: Timer?

    /// Where every tab sits: in `buttons` order, or with the dragged tab
    /// moved to its landing index and drawn where the pointer has it.
    private func slotFrames() -> [(TabButton, NSRect)] {
        guard slotWidths.count == buttons.count else { return [] }
        let h = Design.Space.tabHeight, y = (bounds.height - h) / 2
        var order = Array(zip(buttons, slotWidths))
        if let drag, let i = order.firstIndex(where: { $0.0 === drag.button }) {
            let lifted = order.remove(at: i)
            order.insert(lifted, at: min(drag.to, order.count))
        }
        var x: CGFloat = 0
        return order.map { button, width in
            defer { x += width + Design.Space.tabSpacing }
            if let drag, button === drag.button {
                return (button, NSRect(x: drag.x, y: y, width: width, height: h))
            }
            return (button, NSRect(x: x, y: y, width: width, height: h))
        }
    }

    /// Moves every tab to its slot; the others slide unless motion is off.
    /// The dragged tab follows the pointer at once (it's under the hand).
    private func placeTabs(animated: Bool, alsoDragged: Bool = false) {
        let frames = slotFrames()
        let slide = animated && Motion.active && window != nil
        let moving = frames.filter { button, frame in
            (alsoDragged || button !== drag?.button) && button.frame != frame
        }
        if let drag, !alsoDragged, let frame = frames.first(where: { $0.0 === drag.button })?.1 {
            drag.button.frame = frame
        }
        guard slide, !moving.isEmpty else {
            for (button, frame) in moving { button.frame = frame }
            return
        }
        NSAnimationContext.runAnimationGroup { context in
            context.duration = Self.slideDuration
            context.timingFunction = Motion.timing
            context.allowsImplicitAnimation = true
            for (button, frame) in moving { button.animator().frame = frame }
        }
    }

    /// The slide: quick and fixed, as Safari's (slowed with layout motion
    /// for automation).
    static var slideDuration: CFTimeInterval {
        Motion.durationOverride.map { $0 * 0.9 } ?? Design.Motion.layout
    }

    /// Reordering needs another tab to reorder against, all of them on show.
    private var canDrag: Bool { buttons.count > 1 && !selectedOnly }

    fileprivate func tabPressed(_ button: TabButton, _ event: NSEvent) {
        if drag != nil { return }
        // ctrl-click is a right-click (its menu), never a drag.
        guard !event.modifierFlags.contains(.control) else { press = nil; return }
        press = Press(button: button, start: event.locationInWindow)
    }

    fileprivate func tabDragged(_ button: TabButton, _ event: NSEvent) {
        guard let press, press.button === button, !press.cancelled else { return }
        let point = event.locationInWindow
        if drag == nil {
            let dx = point.x - press.start.x, dy = point.y - press.start.y
            guard canDrag, dx * dx + dy * dy >= Self.dragThreshold * Self.dragThreshold,
                  let from = buttons.firstIndex(where: { $0 === button })
            else { return }
            let start = tabs.convert(press.start, from: nil)
            drag = Drag(button: button, from: from, to: from, grab: start.x - button.frame.minX,
                        x: button.frame.minX, pointer: point)
            button.lifted = true
        }
        moveDrag(to: point)
    }

    /// Mouse up on a pressed tab: true when it was a drag (dropped or
    /// cancelled), so the tab doesn't also select.
    fileprivate func tabReleased(_ button: TabButton, _ event: NSEvent) -> Bool {
        defer { press = nil }
        if let drag, drag.button === button {
            moveDrag(to: event.locationInWindow)
            endDrag(commit: !(self.drag?.outside ?? true))
            return true
        }
        return press?.button === button && press?.cancelled == true
    }

    /// The pointer moved to `point` (window coordinates) mid-drag.
    private func moveDrag(to point: NSPoint) {
        guard var drag else { return }
        drag.pointer = point
        let width = drag.button.frame.width
        let p = tabs.convert(point, from: nil)
        drag.x = min(max(0, p.x - drag.grab), max(0, tabs.bounds.width - width))
        let zone = bounds.insetBy(dx: -Self.dropMargin.width, dy: -Self.dropMargin.height)
        drag.outside = !zone.contains(convert(point, from: nil))
        // Where the pointer has it, unclamped: pushing past either end of
        // the strip takes it to that end, whatever the widths.
        let to = drag.outside ? drag.from : landing(drag, middle: p.x - drag.grab + width / 2)
        let reflow = to != drag.to || drag.outside != self.drag?.outside
        drag.to = to
        self.drag = drag
        drag.button.alphaValue = drag.outside ? Self.outsideAlpha : 1
        if reflow { placeTabs(animated: true) } else { placeTabs(animated: false) }
        updateAutoscroll()
    }

    /// Where the dragged tab lands for where it is now: past a neighbour once
    /// its middle crosses that neighbour's middle (where the neighbour sits
    /// now, so a tab that just slid aside doesn't slide straight back).
    private func landing(_ drag: Drag, middle: CGFloat) -> Int {
        let spacing = Design.Space.tabSpacing
        guard let i = buttons.firstIndex(where: { $0 === drag.button }), slotWidths.count == buttons.count
        else { return drag.to }
        let width = slotWidths[i]
        var others = Array(slotWidths)
        others.remove(at: i)
        var x: CGFloat = 0, to = 0
        for (j, w) in others.enumerated() {
            // Its middle with the dragged tab before it (pushed right) or after it.
            let shift = j >= drag.to ? width + spacing : 0
            if middle > x + w / 2 + shift { to = j + 1 }
            x += w + spacing
        }
        return to
    }

    /// Lets go of the dragged tab: into its landing spot (`onMove`) or, when
    /// cancelled, back where it was. Either way it slides into place.
    private func endDrag(commit: Bool, animated: Bool = true) {
        guard let drag else { return }
        autoscroll?.invalidate()
        autoscroll = nil
        var landed = drag
        if !commit { landed.to = drag.from }
        self.drag = nil
        drag.button.alphaValue = 1
        let moved = commit && landed.to != drag.from
        if moved {
            // The order the core will send back: the next refresh finds
            // nothing new and the tabs stay as they are (no rebuild mid-slide).
            let button = buttons.remove(at: drag.from)
            buttons.insert(button, at: landed.to)
            let width = slotWidths.remove(at: drag.from)
            slotWidths.insert(width, at: landed.to)
            if shown.indices.contains(drag.from) {
                let entry = shown.remove(at: drag.from)
                shown.insert(entry, at: min(landed.to, shown.count))
            }
        }
        // Still on top while it settles.
        placeTabs(animated: animated, alsoDragged: true)
        let settle = animated && Motion.active ? Self.slideDuration : 0
        DispatchQueue.main.asyncAfter(deadline: .now() + settle) { [weak self, weak button = drag.button] in
            guard let button, self?.drag?.button !== button else { return }
            button.lifted = false
        }
        updateFade()
        if moved {
            onMove?(drag.button.tabID, landed.to)
            deferred = nil
        } else if let model = deferred {
            deferred = nil
            apply(model)
        }
    }

    /// Escape mid-drag (KeyRouter): everything goes back; true when there
    /// was a drag to cancel.
    @discardableResult
    func cancelDrag(animated: Bool = true) -> Bool {
        guard drag != nil else { return false }
        press?.cancelled = true
        endDrag(commit: false, animated: animated)
        return true
    }

    /// Near a faded edge of an overflowing strip, the tabs scroll under the
    /// dragged one, faster the closer it gets.
    private func updateAutoscroll() {
        guard let drag, !drag.outside, tabs.frame.width > scroll.bounds.width + 0.5 else {
            autoscroll?.invalidate()
            autoscroll = nil
            return
        }
        let x = scroll.convert(drag.pointer, from: nil).x
        let edge = Design.Space.tabFadeWidth
        let speed: CGFloat = x < edge ? -(edge - x) : x > scroll.bounds.width - edge ? x - (scroll.bounds.width - edge) : 0
        guard speed != 0 else {
            autoscroll?.invalidate()
            autoscroll = nil
            return
        }
        guard autoscroll == nil else { return }
        let timer = Timer(timeInterval: 1.0 / 60, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.autoscrollTick() }
        }
        RunLoop.main.add(timer, forMode: .common)
        autoscroll = timer
    }

    private func autoscrollTick() {
        guard let drag else { return }
        let x = scroll.convert(drag.pointer, from: nil).x
        let edge = Design.Space.tabFadeWidth
        let depth = x < edge ? -(edge - x) : x > scroll.bounds.width - edge ? x - (scroll.bounds.width - edge) : 0
        let clip = scroll.contentView
        let maxX = max(0, tabs.frame.width - clip.bounds.width)
        let step = max(-12, min(12, depth / 3))
        let target = min(max(0, clip.bounds.minX + step), maxX)
        guard step != 0, target != clip.bounds.minX else { return updateAutoscroll() }
        clip.scroll(to: NSPoint(x: target, y: 0))
        scroll.reflectScrolledClipView(clip)
        moveDrag(to: drag.pointer)
    }

    override func viewWillMove(toWindow newWindow: NSWindow?) {
        super.viewWillMove(toWindow: newWindow)
        if newWindow == nil { cancelDrag(animated: false) }
    }

    /// Automation: the drag's phase (`idle`, `pressed`, `dragging`,
    /// `outside`, `cancelled`), where the lifted tab would land (1-based, or
    /// `-`), and the tabs' names in the order they sit on screen: the others
    /// by their frames (where they slid to), the lifted one where it would land.
    var dragPhase: String {
        if let drag { return drag.outside ? "outside" : "dragging" }
        if let press { return press.cancelled ? "cancelled" : "pressed" }
        return "idle"
    }
    var dragLanding: String { drag.map { "\($0.to + 1)" } ?? "-" }
    var visualOrder: [String] {
        var order = buttons.filter { $0 !== drag?.button }.sorted { $0.frame.minX < $1.frame.minX }
        if let drag { order.insert(drag.button, at: min(drag.to, order.count)) }
        return order.map(\.title)
    }
    var liftedTitle: String { buttons.first { $0.lifted }?.title ?? "-" }
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

/// One tab in the strip: state icon and name, filled when selected. A
/// right-click shows its menu without switching to it; a drag reorders
/// (`TabStripView`, which gets its mouse events).
@MainActor
final class TabButton: NSView, ThemeAware, ContextMenuProviding {
    let status: Design.Status
    let tabID: String
    /// The strip that reorders it.
    weak var strip: TabStripView?
    /// Lifted by a drag: filled like the selected tab, with a shadow, over
    /// the others.
    var lifted = false {
        didSet {
            guard lifted != oldValue else { return }
            layer?.zPosition = lifted ? 1 : 0
            applyTheme()
        }
    }
    /// Automation: the rollup's marker text, as the sidebar shows it.
    let markerText: String
    private let handler: () -> Void
    var menuProvider: (() -> NSMenu?)?
    /// Automation: the tab's name as shown.
    let title: String
    private let icon: StatusIconView
    private let label = NSTextField(labelWithString: "")
    let selected: Bool

    init(tab: TabModel, selected: Bool, rollup: AttentionStyle.Rollup, status: Design.Status,
         handler: @escaping () -> Void) {
        self.status = status
        self.selected = selected
        tabID = tab.id
        self.handler = handler
        icon = StatusIconView(status) { Self.icon(status) }
        markerText = rollup.marker?.string ?? ""
        title = tab.label
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.tab
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
        Theme.track(self)
    }

    func applyTheme() {
        layer?.backgroundColor = selected || lifted ? Design.Color.tabSelected.cgColor : nil
        layer?.shadowColor = Design.Color.shadow.cgColor
        layer?.shadowOpacity = lifted ? 0.28 : 0
        layer?.shadowRadius = 4
        layer?.shadowOffset = CGSize(width: 0, height: -1)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// The state icon. Filled symbols (blocked) draw their mark in the strip's
    /// colour on the state colour, as in the mockup, not one flat disc.
    static func icon(_ status: Design.Status) -> NSImage? {
        guard status.symbol.hasSuffix(".fill") else {
            return Design.StatusIcon.image(status, pointSize: Design.Symbol.tabStatus)
        }
        // The mark in the strip's colour; a see-through strip (the macOS
        // themes) would leave no mark, so the window's colour then.
        let strip = Theme.resolved(Design.Color.tabStrip)
        let mark = strip.alphaComponent < 1 ? Theme.resolved(Design.Color.window) : strip
        let config = NSImage.SymbolConfiguration(pointSize: Design.Symbol.tabStatus, weight: .bold)
            .applying(.init(paletteColors: [mark, Theme.resolved(status.color)]))
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
        // Too narrow for a few letters of the name: the icon alone, centred.
        let iconOnly = bounds.width < 2 * px + 14 + Design.Space.tabIconGap + 16
        label.isHidden = iconOnly
        icon.frame = NSRect(x: iconOnly ? (bounds.width - 14) / 2 : px, y: (bounds.height - 14) / 2,
                            width: 14, height: 14)
        // The cell pads its text by ~2 pt: start that much early, end that much late.
        let textX = px + 14 + Design.Space.tabIconGap - 2
        let textHeight = ceil(label.intrinsicContentSize.height)
        label.frame = NSRect(x: textX, y: (bounds.height - textHeight) / 2,
                             width: max(0, bounds.width - textX - px + Self.cellSlack - 2), height: textHeight)
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    /// The whole tab is one click target (not its label or icon).
    override func hitTest(_ point: NSPoint) -> NSView? {
        frame.contains(point) ? self : nil
    }
    /// A press selects on mouse up unless it became a drag (`TabStripView`).
    override func mouseDown(with event: NSEvent) { strip?.tabPressed(self, event) }
    override func mouseDragged(with event: NSEvent) { strip?.tabDragged(self, event) }
    override func mouseUp(with event: NSEvent) {
        if strip?.tabReleased(self, event) == true { return }
        if bounds.contains(convert(event.locationInWindow, from: nil)) { handler() }
    }
    override var mouseDownCanMoveWindow: Bool { false }
    override func accessibilityPerformPress() -> Bool { handler(); return true }
    override func menu(for event: NSEvent) -> NSMenu? { contextMenu() }
    func contextMenu() -> NSMenu? { menuProvider?() }
}

/// `tabdrag …` in automation scripts: mouse events made with `NSEvent` and
/// handed straight to a tab's own mouseDown / mouseDragged / mouseUp, as
/// AppKit would deliver a real drag. Nothing goes through the window server
/// and no other app sees anything.
///
///     tabdrag press N          mouse down in the middle of tab N (1-based)
///     tabdrag move DX DY [S]   drag to the press point + (DX, DY), in S steps
///     tabdrag over N [S]       drag until the lifted tab's middle passes tab
///                              N's middle (where tab N sits now), in S steps
///     tabdrag away [S]         drag 60 pt below the strip (outside it)
///     tabdrag release          mouse up where the pointer is
///     tabdrag click N          press and release without moving
///     tabdrag mark TEXT        a line in the app log (to find a drop in it)
///
/// State: `tabdrag.phase` (idle, pressed, dragging, outside, cancelled),
/// `tabdrag.to` (where the lifted tab would land, 1-based), `tabdrag.lifted`,
/// `tab.visual` (names in the order the tabs sit on screen), `tab.selected`
/// and `tab.session` (the session's order, from the core).
@MainActor
enum TabDragAutomation {
    private static weak var button: TabButton?
    private static var start = NSPoint.zero
    private static var pointer = NSPoint.zero

    static func run(_ arg: String, app: AppController?) -> String? {
        guard let app, let controller = app.focusedController else { return "no window" }
        let words = arg.split(separator: " ").map(String.init)
        let strip = controller.tabStrip
        let number = { (i: Int) in words.count > i ? Int(words[i]) : nil }
        func tab(_ n: Int?) -> TabButton? {
            guard let n, strip.tabButtons.indices.contains(n - 1) else { return nil }
            return strip.tabButtons[n - 1]
        }
        func middle(_ tab: TabButton) -> NSPoint {
            tab.convert(NSPoint(x: tab.bounds.midX, y: tab.bounds.midY), to: nil)
        }
        switch words.first ?? "" {
        case "press", "click":
            guard let target = tab(number(1)) else {
                return "\(words[0]) needs a tab number (1–\(strip.tabButtons.count))"
            }
            button = target
            start = middle(target)
            pointer = start
            send(.leftMouseDown, to: target, in: controller.window)
            if words[0] == "click" { send(.leftMouseUp, to: target, in: controller.window) }
        case "move":
            guard words.count > 2, let dx = Double(words[1]), let dy = Double(words[2]) else {
                return "move needs DX DY [steps]"
            }
            drag(to: NSPoint(x: start.x + dx, y: start.y + dy), steps: number(3) ?? 1, in: controller.window)
        case "over":
            guard let target = tab(number(1)), let lifted = button else { return "over needs a press and a tab number" }
            // The pointer where the lifted tab's middle passes the target's:
            // it keeps the offset it was grabbed at (the middle, by `press`).
            let x = middle(target).x - middle(lifted).x + pointer.x
            drag(to: NSPoint(x: x + (x > pointer.x ? 3 : -3), y: start.y), steps: number(2) ?? 6,
                 in: controller.window)
        case "away":
            drag(to: NSPoint(x: pointer.x, y: start.y - 60), steps: number(1) ?? 4, in: controller.window)
        case "release":
            guard let lifted = button else { return "release needs a press" }
            send(.leftMouseUp, to: lifted, in: controller.window)
        case "mark":
            app.log("tabdrag mark \(words.dropFirst().joined(separator: " "))")
        default:
            return "tabdrag needs press N|move DX DY [S]|over N [S]|away [S]|release|click N|mark TEXT"
        }
        return nil
    }

    private static func drag(to end: NSPoint, steps: Int, in window: NSWindow) {
        guard let lifted = button else { return }
        let from = pointer
        let steps = max(1, steps)
        for i in 1...steps {
            let t = CGFloat(i) / CGFloat(steps)
            pointer = NSPoint(x: from.x + (end.x - from.x) * t, y: from.y + (end.y - from.y) * t)
            send(.leftMouseDragged, to: lifted, in: window)
        }
    }

    private static func send(_ type: NSEvent.EventType, to view: TabButton, in window: NSWindow) {
        guard let event = NSEvent.mouseEvent(
            with: type, location: pointer, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1,
            pressure: type == .leftMouseUp ? 0 : 1)
        else { return }
        switch type {
        case .leftMouseDown: view.mouseDown(with: event)
        case .leftMouseDragged: view.mouseDragged(with: event)
        default: view.mouseUp(with: event)
        }
    }

    static func state(app: AppController, controller: WindowController) -> [String: String] {
        let strip = controller.tabStrip
        return [
            "tabdrag.phase": strip.dragPhase,
            "tabdrag.to": strip.dragLanding,
            "tabdrag.lifted": strip.liftedTitle,
            "tab.visual": strip.visualOrder.joined(separator: "|"),
            "tab.selected": strip.tabButtons.first { $0.selected }?.title ?? "-",
            // The tab at the strip's left edge, the lifted one aside (the
            // others slid under it).
            "tab.leftmost": strip.tabButtons.filter { !$0.lifted && $0.frame.minX < 0.5 }.first?.title ?? "-",
            "tab.session": app.session?.window(controller.windowID)?.workspace?.tabs
                .map(\.label).joined(separator: "|") ?? "-",
        ]
    }
}
