import AppKit

/// The collapsed sidebar as a rail (`[sidebar] collapsed = "rail"`, the
/// default), after herdr's collapsed sidebar: a narrow card down the left,
/// under the toolbar row, from the same data as the full sidebar's
/// Workspaces view (workspaces only: no Workspaces | Files switch).
///
/// - Per machine group, its first letter ("L" for Local) and connection dot,
///   or a turning arc in the state's colour while it connects or
///   reconnects, a hairline between groups; then its workspaces, each its
///   status icon alone, the current one on the selected fill. Holding ⌘
///   alone shows each row's chord (⌘1…⌘8, ⌘9 for the last) in the full
///   sidebar's key badge, in the icon's place (`WorkspaceHints`); a row's
///   hover card names its chord too.
/// - At the bottom: how many workspaces wait (its hover card counts the
///   rest), then `»`, which opens the full sidebar.
///
/// Rows keep the same inset on both sides, their icon (or badge) centred.
/// Resting the pointer on a row (or a letter) shows its hover card; a click
/// focuses the workspace; a drag reorders it within its group
/// (`WorkspaceDrag`); right-click gives its usual menu. The list scrolls
/// between the top and the footer: its edges fade where rows go on beyond
/// them, a marker counts the waiting rows out of view ("↑2"), and the
/// active row comes into view when the rail shows and when the workspace
/// changes. The rail never takes the keyboard.
@MainActor
final class SidebarRailView: NSView, WorkspaceDragHost, ThemeAware {
    var onSelect: ((String) -> Void)?
    var onMenu: ((String) -> NSMenu?)?
    var onExpand: (() -> Void)?
    /// A dropped drag: move the workspace to an index in the window.
    var onMove: ((String, Int) -> Void)?

    static var width: CGFloat { Design.Space.railWidth }

    private let scroll = NSScrollView()
    private let document = RailDocument()
    private lazy var expand = RailExpandButton { [weak self] in self?.onExpand?() }
    private let wait = RailWaitPill()
    private let footerLine = NSView()
    private let topMarker = EdgePill(style: .marker)
    private let bottomMarker = EdgePill(style: .marker)
    private var shown: [String] = []
    private var shownActive: String?
    private var order: [String] = []
    /// A refresh held back while a drag runs.
    private var pending: (window: WindowModel, machines: [MachineModel])?
    private(set) var groups: [RailGroupHeader] = []
    private(set) var workspaceRows: [RailWorkspaceRow] = []
    private var dividers: [GroupDivider] = []
    /// The active row comes into view on the next layout.
    private var revealActive = false
    /// The rows out of view, as the markers last counted them.
    private(set) var overflow = Overflow()
    /// Each row's ⌘N key badge shows (⌘ held alone, `WorkspaceHints`).
    private(set) var hintsShown = false
    /// Automation: how long the last show or hide of the badges faded.
    private(set) var hintsFade: TimeInterval = 0

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { false }
    /// A click between rows does nothing (the card would move the window).
    override func mouseDown(with event: NSEvent) {}

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        scroll.documentView = document
        scroll.drawsBackground = false
        // Too narrow for a scroller; a trackpad or wheel still scrolls, and
        // the markers stand in for it.
        scroll.hasVerticalScroller = false
        scroll.verticalScrollElasticity = .allowed
        addSubview(scroll)
        scroll.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(self, selector: #selector(scrolled),
                                               name: NSView.boundsDidChangeNotification, object: scroll.contentView)
        footerLine.wantsLayer = true
        addSubview(footerLine)
        wait.isHidden = true
        addSubview(wait)
        addSubview(expand)
        topMarker.onPress = { [weak self] in self?.scrollToWaiting(up: true) }
        bottomMarker.onPress = { [weak self] in self?.scrollToWaiting(up: false) }
        addSubview(topMarker)
        addSubview(bottomMarker)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        footerLine.layer?.backgroundColor = Design.Color.border.cgColor
    }

    /// Folded: the active row comes into view.
    override func viewDidUnhide() {
        super.viewDidUnhide()
        revealActive = true
        needsLayout = true
    }

    // MARK: Model

    /// One agent's icon, as its tile's header shows it: blocked, then
    /// working, then finished and not read (or marked unread), else idle (a
    /// read finish is idle even while herdr still says `done`).
    static func status(_ agent: AgentModel) -> Design.Status {
        if agent.status == .blocked { return .blocked }
        if agent.status == .working { return .working }
        return agent.attention ? .done : .idle
    }

    /// A machine's letter: "L" for Local, else its label's first letter.
    static func letter(_ machineID: String, machines: [MachineModel]) -> String {
        let label = machines.first { $0.id == machineID }?.label
            ?? (machineID == MachineModel.local ? "Local" : machineID)
        return label.first.map { String($0).uppercased() } ?? "?"
    }

    /// "2 working · 1 blocked", or "no agents".
    static func counts(_ ws: WorkspaceModel) -> String {
        let statuses = ws.tabs.flatMap(\.tiles).compactMap(\.agent).map(status)
        let order: [Design.Status] = [.working, .blocked, .done, .idle]
        let parts = order.compactMap { s -> String? in
            let n = statuses.filter { $0 == s }.count
            return n > 0 ? "\(n) \(s.rawValue)" : nil
        }
        return parts.isEmpty ? "no agents" : parts.joined(separator: " · ")
    }

    func apply(_ window: WindowModel, machines: [MachineModel]) {
        if WorkspaceDrag.current?.hosted(by: self) == true {
            pending = (window, machines)
            return
        }
        pending = nil
        order = window.workspaces.map(\.id)
        let groups = SidebarView.groups(window, machines: machines)
        var key = groups.map {
            "\($0.id)|\($0.machine?.label ?? "")|\($0.machine?.state.rawValue ?? "")|\($0.machine?.message ?? "")"
        }
        let infos = Dictionary(uniqueKeysWithValues: window.workspaces.map { ($0.id, WorkspaceInfo($0)) })
        for ws in window.workspaces {
            key.append("\(ws.id == window.activeWorkspace)|\(String(describing: infos[ws.id]))|\(Self.counts(ws))")
        }
        guard key != shown else { return }
        shown = key
        if window.activeWorkspace != shownActive {
            shownActive = window.activeWorkspace
            revealActive = true
        }

        (self.groups as [NSView] + workspaceRows + dividers).forEach { $0.removeFromSuperview() }
        if let card = HoverCard.shown, card.anchor.window == nil { HoverCard.hide() }
        self.groups = []
        workspaceRows = []
        dividers = []
        for (index, group) in groups.enumerated() {
            if index > 0 {
                let divider = GroupDivider()
                document.addSubview(divider)
                dividers.append(divider)
            }
            let header = RailGroupHeader(group: group, letter: Self.letter(group.id, machines: machines))
            document.addSubview(header)
            self.groups.append(header)
            let dimmed = group.machine.map { $0.state != .connected } ?? true
            for (number, ws) in group.workspaces {
                let info = infos[ws.id] ?? WorkspaceInfo(ws)
                let row = RailWorkspaceRow(
                    info, groupID: ws.machineID, number: number, chord: SidebarView.chord(number, of: window.workspaces.count),
                    selected: ws.id == window.activeWorkspace, dimmed: dimmed,
                    tip: Self.tip(ws, number: number, of: window.workspaces.count)
                ) { [weak self] in self?.onSelect?(ws.id) }
                row.menuProvider = { [weak self] in self?.onMenu?(ws.id) }
                row.setHint(hintsShown, duration: 0)
                header.rows.append(row)
                document.addSubview(row)
                workspaceRows.append(row)
            }
        }
        // The footer: how many wait, and the others in its card.
        let statuses = window.workspaces.map { infos[$0.id]?.status ?? .idle }
        let counts = Dictionary(grouping: statuses, by: { $0 }).mapValues(\.count)
        wait.count = counts[.blocked] ?? 0
        wait.isHidden = wait.count == 0
        wait.words = [(Design.Status.blocked, "waiting"), (.working, "working"), (.done, "done"), (.idle, "idle")]
            .compactMap { status, word in counts[status].map { "\($0) \(word)" } }
            .joined(separator: " · ")
        needsLayout = true
    }

    /// Shows or hides each row's ⌘N badge in its icon's place, as the full
    /// sidebar does: a quick fade with layout motion on, at once with it off
    /// or Reduce Motion on.
    func setHints(_ on: Bool) {
        guard on != hintsShown else { return }
        hintsShown = on
        hintsFade = Motion.active ? Design.Motion.sidebarHint : 0
        for row in workspaceRows { row.setHint(on, duration: hintsFade) }
    }

    /// Name and ⌘ number, branch, agent counts (automation's `rail.tips`;
    /// the hover card says more).
    static func tip(_ ws: WorkspaceModel, number: Int, of count: Int) -> String {
        var lines = [SidebarView.chord(number, of: count).map { "\(ws.name) (\($0))" } ?? ws.name]
        if let branch = SidebarView.branch(ws.git) { lines.append(branch) }
        lines.append(counts(ws))
        return lines.joined(separator: "\n")
    }

    // MARK: Dragging (WorkspaceDragHost)

    var dragRows: [WorkspaceDragRow] { workspaceRows }
    var dragOrder: [String] { pending.map { $0.window.workspaces.map(\.id) } ?? order }
    var dropArea: NSRect {
        convert(bounds, to: nil).insetBy(dx: -Design.Space.dropMargin, dy: -Design.Space.dropMargin)
    }

    func dragMove(_ workspace: String, to index: Int) { onMove?(workspace, index) }

    func dragEnded() {
        if let pending { apply(pending.window, machines: pending.machines) }
    }

    // MARK: Layout

    override func layout() {
        super.layout()
        let w = bounds.width
        let expandHeight = Design.Space.railExpandHeight
        expand.frame = NSRect(x: (w - expandHeight) / 2, y: bounds.height - 4 - expandHeight,
                              width: expandHeight, height: expandHeight)
        var footerTop = expand.frame.minY - 8
        if !wait.isHidden {
            let pillWidth = min(w - 8, wait.fittingWidth)
            let h = Design.Space.railWaitHeight
            wait.frame = NSRect(x: (w - pillWidth) / 2, y: expand.frame.minY - 4 - h, width: pillWidth, height: h)
            footerTop = wait.frame.minY - 8
        }
        footerLine.frame = NSRect(x: 6, y: footerTop - 1, width: max(0, w - 12), height: 1)
        let top: CGFloat = 5
        scroll.frame = NSRect(x: 0, y: top, width: w, height: max(0, footerLine.frame.minY - top))
        layoutDocument()
        if revealActive, let row = workspaceRows.first(where: \.selected) {
            revealActive = false
            document.scrollToVisible(row.frame.insetBy(dx: 0, dy: -(Design.Space.railEdgeMarker + 4)))
        }
        updateOverflow()
    }

    /// Groups and their workspaces from the top, the same inset on both
    /// sides; a hairline (inset a little more) between groups.
    private func layoutDocument() {
        let pad = Design.Space.railPadding
        let width = scroll.bounds.width
        let rowWidth = max(0, width - pad * 2)
        var y: CGFloat = 0
        for (index, header) in groups.enumerated() {
            if index > 0, dividers.indices.contains(index - 1) {
                dividers[index - 1].frame = NSRect(x: 6, y: y, width: max(0, width - 12),
                                                   height: Design.Space.sidebarGroupGap)
                y += Design.Space.sidebarGroupGap
            }
            header.frame = NSRect(x: pad, y: y, width: rowWidth, height: Design.Space.railGroupHeight)
            y += Design.Space.railGroupHeight
            for row in header.rows {
                row.frame = NSRect(x: pad, y: y, width: rowWidth, height: Design.Space.railRowHeight)
                y += Design.Space.railRowHeight
            }
        }
        document.frame = NSRect(x: 0, y: 0, width: width, height: max(scroll.contentSize.height, y + 4))
    }

    @objc private func scrolled() {
        HoverCard.hide()
        updateOverflow()
    }

    /// The edge fades and the waiting markers, for the rows out of view now.
    private func updateOverflow() {
        let visible = scroll.documentVisibleRect
        let rows = workspaceRows.map { (frame: $0.frame, waiting: $0.status == .blocked) }
        overflow = Overflow(rows: rows, visible: visible, flipped: true)
        let content = (workspaceRows.map(\.frame.maxY) + groups.map(\.frame.maxY)).max() ?? 0
        EdgeFade.apply(to: scroll, top: visible.minY > 0.5, bottom: visible.maxY < content - 0.5)
        topMarker.isHidden = overflow.aboveWaiting == 0
        bottomMarker.isHidden = overflow.belowWaiting == 0
        if !topMarker.isHidden { topMarker.set(arrow: "↑", plain: nil, waiting: "\(overflow.aboveWaiting)") }
        if !bottomMarker.isHidden { bottomMarker.set(arrow: "↓", plain: nil, waiting: "\(overflow.belowWaiting)") }
        for (marker, top) in [(topMarker, true), (bottomMarker, false)] where !marker.isHidden {
            let w = min(bounds.width - 4, marker.fittingWidth)
            let h = marker.height
            let y = top ? scroll.frame.minY + 3 : scroll.frame.maxY - 5 - h
            marker.frame = NSRect(x: (bounds.width - w) / 2, y: y, width: w, height: h)
        }
    }

    /// A marker's click: the nearest waiting row that way into view.
    private func scrollToWaiting(up: Bool) {
        let visible = scroll.documentVisibleRect
        let beyond = workspaceRows.filter { up ? $0.frame.midY < visible.minY : $0.frame.midY > visible.maxY }
        let ordered = up ? beyond.reversed() : beyond
        guard let target = ordered.first(where: { $0.status == .blocked }) else { return }
        document.scrollToVisible(target.frame.insetBy(dx: 0, dy: -(Design.Space.railEdgeMarker + 6)))
    }
}

extension SidebarRailView {
    /// Automation: workspace rows as "number:status", "*" on the current one,
    /// "~" on a dimmed one ("1:working*|2:blocked").
    var rowSummary: String {
        workspaceRows.map { "\($0.number):\($0.status.rawValue)\($0.selected ? "*" : "")\($0.dimmed ? "~" : "")" }
            .joined(separator: "|")
    }

    /// Automation: group headers as "letter:state" ("L:connected|B:reconnecting").
    var groupSummary: String { groups.map(\.summary).joined(separator: "|") }

    /// Automation: whether every row lies inside the rail (visible without
    /// scrolling), and nothing overlaps.
    var fits: Bool {
        let rows: [NSView] = groups + workspaceRows
        let visible = scroll.documentVisibleRect
        for (i, a) in rows.enumerated() {
            if !visible.contains(a.frame) { return false }
            for b in rows[(i + 1)...] where a.frame.intersects(b.frame) { return false }
        }
        return true
    }

    /// Automation: how far each row's content sits off the row's middle
    /// (its letter and dot, or its icon, as drawn), and each
    /// row's inset from the rail on the left and right ("0|0|0" and
    /// "4/4|4/4"), so a lopsided rail fails.
    var contentOffsets: String {
        (groups.map(\.contentOffset) + workspaceRows.map(\.contentOffset))
            .map { String(format: "%g", ($0 * 2).rounded() / 2) }.joined(separator: "|")
    }

    /// Automation: every row's content within a point of its middle, and
    /// every row inset the same on both sides.
    var centred: Bool {
        let offsets = groups.map(\.contentOffset) + workspaceRows.map(\.contentOffset)
        let even = workspaceRows.allSatisfy { row in
            let r = row.convert(row.bounds, to: self)
            return abs(r.minX - (bounds.width - r.maxX)) < 0.5
        }
        return even && offsets.allSatisfy { abs($0) <= 1 }
    }

    var rowInsets: String {
        workspaceRows.map { row in
            let r = row.convert(row.bounds, to: self)
            return "\(Int(r.minX.rounded()))/\(Int((bounds.width - r.maxX).rounded()))"
        }.joined(separator: "|")
    }

    var expandButton: NSView { expand }

    /// Automation: each badge showing lies inside the rail and its row's
    /// middle is its own (within half a point), its icon faded out; each
    /// row without one shows its icon.
    var hintsPlaced: Bool {
        workspaceRows.allSatisfy { row in
            guard row.hintShown, let frame = row.hintFrame else { return row.iconAlpha > 0.99 }
            let badge = row.convert(frame, to: self)
            let mid = row.convert(row.bounds, to: self)
            return bounds.contains(badge) && abs(badge.midX - mid.midX) <= 0.5 && abs(badge.midY - mid.midY) <= 0.5
                && row.iconAlpha < 0.01
        }
    }

    /// Automation: the waiting markers showing ("↑2|-", "-" for none).
    var markers: String {
        [topMarker, bottomMarker].map { $0.isHidden ? "-" : $0.text }.joined(separator: "|")
    }

    /// Automation: the footer's waiting count, or "-".
    var waitText: String { wait.isHidden ? "-" : "\(wait.count)" }

    /// Automation: the active row lies in the visible part of the list.
    var activeVisible: Bool {
        guard let row = workspaceRows.first(where: \.selected) else { return false }
        return scroll.documentVisibleRect.contains(row.frame.insetBy(dx: 1, dy: 1))
    }

    /// `sidebar rail select N` (the Nth workspace row), `hover N` (its hover
    /// fill and card at once; 0 for none), `group N` (the Nth letter's
    /// card), `wait` (the footer count's card), `scroll top|bottom`,
    /// `marker top|bottom` (a click on it), `expand` (»).
    func automate(_ bits: [String]) -> String? {
        let n = bits.count > 1 ? Int(bits[1]) : nil
        switch bits.first ?? "" {
        case "select":
            guard let n, workspaceRows.indices.contains(n - 1) else { return "no rail row \(bits.dropFirst().joined())" }
            workspaceRows[n - 1].press()
        case "expand":
            (expand as NSButton).performClick(nil)
        case "hover":
            HoverCard.cancel()
            HoverCard.hide()
            for (i, row) in workspaceRows.enumerated() { row.hover(i == (n ?? 0) - 1) }
            if let n, workspaceRows.indices.contains(n - 1) { workspaceRows[n - 1].showCard(now: true) }
        case "group":
            HoverCard.hide()
            guard let n, groups.indices.contains(n - 1) else { return "no rail group \(bits.dropFirst().joined())" }
            groups[n - 1].showCard(now: true)
        case "wait":
            HoverCard.hide()
            guard !wait.isHidden else { return "nothing waits" }
            wait.showCard(now: true)
        case "scroll":
            let bottom = bits.count > 1 && bits[1] == "bottom"
            document.scroll(NSPoint(x: 0, y: bottom ? max(0, document.frame.height - scroll.contentSize.height) : 0))
            updateOverflow()
        case "marker":
            let top = bits.count > 1 && bits[1] == "top"
            let marker = top ? topMarker : bottomMarker
            guard !marker.isHidden else { return "no \(top ? "top" : "bottom") marker" }
            marker.onPress?()
        default:
            return "sidebar rail takes select N, hover N, group N, wait, scroll top|bottom, marker top|bottom or expand"
        }
        return nil
    }
}

/// The rail's scrolling content, top down.
private final class RailDocument: NSView {
    override var isFlipped: Bool { true }
}

// MARK: - Groups

/// A machine group in the rail: its letter and a connection dot, or a
/// turning arc while it connects or reconnects. Not clickable; resting the
/// pointer on it shows the machine's state and herdr's message.
@MainActor
final class RailGroupHeader: NSView, ThemeAware {
    let summary: String
    /// Automation: what its card says, line by line ("standin\nConnected").
    let tipText: String
    fileprivate var rows: [RailWorkspaceRow] = []
    private let label: NSTextField
    private let mark: NSView
    private let markSize: NSSize
    private let card: [HoverLine]
    private(set) var contentOffset: CGFloat = 0
    private var hovering = false {
        didSet { Theme.reapply(self) }
    }

    override var isFlipped: Bool { true }

    init(group: SidebarView.Group, letter: String) {
        let machine = group.machine
        let status: Design.Status = switch machine?.state {
        case .connected: .connected
        case .connecting: .connecting
        case .reconnecting: .reconnecting
        case .error: .error
        case .off, nil: .off
        }
        summary = "\(letter):\(machine?.state.rawValue ?? "gone")"
        label = SidebarText.label(letter, font: Design.Font.railLetter, color: Design.Color.textSecondary)
        label.lineBreakMode = .byClipping
        let busy = machine?.state == .connecting || machine?.state == .reconnecting
        if busy {
            mark = StatusIconView(status, image: {
                Design.customSymbol("azulejo.arc", color: status.color, pointSize: Design.Symbol.machineRing)
            }, spins: true)
            let side = (Design.Symbol.machineRing * 8 / 7).rounded()
            markSize = NSSize(width: side, height: side)
        } else {
            let side = Design.Radius.machineDot * 2 - 1
            mark = DotView(color: status.color, side: side)
            markSize = NSSize(width: side, height: side)
        }
        let state: String? = switch machine?.state {
        case .connected, nil: nil
        case .connecting: "Connecting…"
        case .reconnecting: "Reconnecting…"
        case .error: "Needs attention"
        case .off: "Off"
        }
        let name = machine?.label ?? "machine \(group.id.prefix(8))"
        let stateText = state ?? machine.map { _ in "Connected" } ?? "Not in herdr"
        tipText = [name, stateText, machine?.message].compactMap { $0 }.joined(separator: "\n")
        var card: [HoverLine] = [.title(name, key: nil)]
        let color = machine?.state == .connected ? Design.Color.statusConnected
            : machine?.state == .off || machine == nil ? Design.Color.textSecondary : status.color
        card.append(.state(status, stateText, color: color, trailing: nil))
        if let message = machine?.message, !message.isEmpty { card.append(.text(message)) }
        let names = group.workspaces.map(\.workspace.name)
        card.append(.text(names.isEmpty ? "No workspaces"
            : "\(names.count) workspace\(names.count == 1 ? "" : "s") · \(names.joined(separator: ", "))"))
        self.card = card
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.row + 1
        layer?.cornerCurve = .continuous
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        setAccessibilityLabel(tipText.replacingOccurrences(of: "\n", with: ", "))
        addSubview(label)
        addSubview(mark)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { false }

    override func layout() {
        super.layout()
        // The letter and its mark side by side, centred together.
        let width = SidebarText.width(label.stringValue, font: Design.Font.railLetter)
        let gap: CGFloat = 4
        let total = width + gap + markSize.width
        let left = (bounds.width - total) / 2
        let field = label.cell?.cellSize ?? label.intrinsicContentSize
        let inset = max(0, (field.width - width) / 2)
        label.frame = NSRect(x: left - inset, y: (bounds.height - field.height) / 2, width: field.width,
                             height: field.height)
        mark.frame = NSRect(x: left + width + gap, y: label.frame.midY - markSize.height / 2,
                            width: markSize.width, height: markSize.height)
        contentOffset = (left + total / 2) - bounds.width / 2
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect], owner: self))
    }

    override func mouseEntered(with event: NSEvent) {
        hovering = true
        showCard(now: false)
    }
    override func mouseExited(with event: NSEvent) {
        hovering = false
        HoverCard.leave(self)
    }

    /// Its hover card: the machine's state, herdr's message, its workspaces.
    func showCard(now: Bool) {
        let card = self.card
        HoverCard.schedule(self, placement: .right, now: now) { card }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        layer?.backgroundColor = hovering ? Theme.faded(Design.Color.selected, Design.Alpha.rowHover).cgColor : nil
    }
}

// MARK: - Rows

/// A workspace in the rail: its status icon, centred, or its chord's key
/// badge in the icon's place while ⌘ is held alone; the selected fill or
/// the hover one; a click, a drag or a right-click; its hover card after a
/// rest.
@MainActor
final class RailWorkspaceRow: NSView, ThemeAware, ContextMenuProviding, WorkspaceDragRow {
    let number: Int
    let info: WorkspaceInfo
    var status: Design.Status { info.status }
    let selected: Bool
    let dimmed: Bool
    let workspaceID: String
    let groupID: String
    /// Automation: name and chord, branch, agent counts (`rail.tips`).
    let tipText: String
    var dragName: String { "\(number)" }
    var menuProvider: (() -> NSMenu?)?
    private(set) var contentOffset: CGFloat = 0
    /// The chord that reaches it (⌘1…⌘8, ⌘9 for the last), on its key badge
    /// while ⌘ is held alone and in its hover card; nil past ⌘8 but the last.
    let chord: String?
    /// The key badge is showing (or fading in).
    private(set) var hintShown = false
    private let handler: () -> Void
    private let icon: StatusIconView
    private let hint = HintBadge()
    private var pressing = RowPress()
    /// Automation: the working ring is turning.
    var isSpinning: Bool { icon.isSpinning }
    private var hovering = false {
        didSet { Theme.reapply(self) }
    }

    override var isFlipped: Bool { true }

    init(_ info: WorkspaceInfo, groupID: String, number: Int, chord: String?, selected: Bool, dimmed: Bool,
         tip: String, handler: @escaping () -> Void) {
        self.info = info
        self.number = number
        self.chord = chord
        self.selected = selected
        self.dimmed = dimmed
        self.handler = handler
        self.groupID = groupID
        workspaceID = info.id
        tipText = tip
        let shown = info.shown
        icon = StatusIconView(shown) { SidebarGlyph.status(shown, pointSize: Design.Symbol.railStatus) }
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.row + 1
        layer?.cornerCurve = .continuous
        setAccessibilityRole(.button)
        alphaValue = dimmed ? Design.Alpha.dimmed : shown == .starting ? 0.75 : 1
        // No number shows at rest: VoiceOver reads the chord with the name.
        let named = chord.map { "\(info.name) (\($0))" } ?? info.name
        setAccessibilityLabel("\(named), \(shown == .blocked ? "waiting" : shown.rawValue)")
        addSubview(icon)
        if let chord {
            // Over the icon, which fades out meanwhile.
            hint.text = chord
            hint.alphaValue = 0
            hint.isHidden = true
            addSubview(hint)
        }
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { false }

    override func layout() {
        super.layout()
        let side: CGFloat = 16
        icon.frame = NSRect(x: ((bounds.width - side) / 2).rounded(), y: ((bounds.height - side) / 2).rounded(),
                            width: side, height: side)
        if chord != nil {
            // Centred on the row, whole points (the rail's rows are a few
            // points wider than the widest badge).
            let size = hint.intrinsicContentSize
            hint.frame = NSRect(x: ((bounds.width - size.width) / 2).rounded(),
                                y: ((bounds.height - size.height) / 2).rounded(),
                                width: size.width, height: size.height)
        }
        // The drawn icon, off the row's middle.
        contentOffset = icon.frame.midX - bounds.width / 2
    }

    /// Shows or hides the key badge, cross-fading it with the icon over
    /// `duration` (0: at once), as a sidebar row cross-fades it with its
    /// time. A row with no chord keeps its icon.
    func setHint(_ on: Bool, duration: TimeInterval) {
        guard chord != nil, on != hintShown else { return }
        hintShown = on
        if on { hint.isHidden = false }
        guard duration > 0 else {
            hint.alphaValue = on ? 1 : 0
            icon.alphaValue = on ? 0 : 1
            return settleHint()
        }
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = duration
            ctx.timingFunction = CAMediaTimingFunction(name: .easeOut)
            hint.animator().alphaValue = on ? 1 : 0
            icon.animator().alphaValue = on ? 0 : 1
        }, completionHandler: { [weak self] in MainActor.assumeIsolated { self?.settleHint() } })
    }

    /// A faded-out badge leaves the view tree's drawing.
    private func settleHint() {
        if !hintShown { hint.isHidden = true }
    }

    /// Automation: the badge's frame in the row, and the icon's.
    var hintFrame: NSRect? { chord == nil ? nil : hint.frame }
    var iconAlpha: CGFloat { icon.alphaValue }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect], owner: self))
    }

    override func mouseEntered(with event: NSEvent) {
        hovering = true
        showCard(now: false)
    }
    override func mouseExited(with event: NSEvent) {
        hovering = false
        HoverCard.leave(self)
    }
    override func mouseDown(with event: NSEvent) {
        HoverCard.cancel()
        HoverCard.hide()
        pressing.down(event)
    }
    override func mouseDragged(with event: NSEvent) { pressing.dragged(event, row: self) }
    override func mouseUp(with event: NSEvent) {
        if pressing.up(event, in: self) { handler() }
    }
    override func accessibilityPerformPress() -> Bool { handler(); return true }
    override func menu(for event: NSEvent) -> NSMenu? {
        HoverCard.cancel()
        HoverCard.hide()
        return contextMenu()
    }
    func contextMenu() -> NSMenu? { menuProvider?() }

    /// Its hover card, right of the rail: the full name and chord, branch,
    /// state and path.
    func showCard(now: Bool) {
        let info = self.info, chord = self.chord
        HoverCard.schedule(self, placement: .right, now: now) { info.cardLines(chord: chord) }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    /// The selected fill, or the hover one.
    func applyTheme() {
        layer?.backgroundColor = (selected ? Design.Color.selected
            : hovering ? Theme.faded(Design.Color.selected, Design.Alpha.rowHover) : nil)?.cgColor
    }

    /// Automation: what a click does.
    func press() { handler() }

    /// Automation: the pointer over it, or not (its hover fill).
    func hover(_ on: Bool) { hovering = on }
}

// MARK: - Footer

/// The rail's footer count: how many workspaces wait ("! 3"), on the
/// waiting colour's tint; its hover card counts the rest.
@MainActor
private final class RailWaitPill: NSView, ThemeAware {
    var count = 0 {
        didSet { label.stringValue = "\(count)"; needsLayout = true }
    }
    /// Every state's count, for the card ("3 waiting · 1 working").
    var words = ""
    private let icon = NSImageView()
    private let label = SidebarText.label(font: Design.Font.railWait, color: Design.Color.statusBlocked)

    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        label.lineBreakMode = .byClipping
        addSubview(icon)
        addSubview(label)
        setAccessibilityElement(true)
        setAccessibilityRole(.staticText)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    var fittingWidth: CGFloat { 4 + 9 + 2 + SidebarText.width(label.stringValue, font: Design.Font.railWait) + 6 }

    override func layout() {
        super.layout()
        layer?.cornerRadius = bounds.height / 2
        let iconSide = Design.Symbol.railWait + 1
        icon.frame = NSRect(x: 4, y: (bounds.height - iconSide) / 2, width: iconSide - 1, height: iconSide)
        let field = label.cell?.cellSize ?? label.intrinsicContentSize
        label.frame = NSRect(x: icon.frame.maxX + 2 - 2, y: (bounds.height - field.height) / 2,
                             width: max(0, bounds.width - icon.frame.maxX), height: field.height)
        setAccessibilityLabel("\(count) waiting")
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect], owner: self))
    }

    override func mouseEntered(with event: NSEvent) { showCard(now: false) }
    override func mouseExited(with event: NSEvent) { HoverCard.leave(self) }

    func showCard(now: Bool) {
        let words = self.words
        HoverCard.schedule(self, placement: .right, now: now) { [.text(words)] }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        layer?.backgroundColor = Theme.faded(Design.Color.statusBlocked, 0.14).cgColor
        icon.image = NSImage(systemSymbolName: "exclamationmark", accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: Design.Symbol.railWait, weight: .heavy)
                .applying(.init(paletteColors: [Theme.resolved(Design.Color.statusBlocked)])))
    }
}

/// `»` at the bottom of the rail: opens the full sidebar.
@MainActor
private final class RailExpandButton: NSButton {
    private let onPress: () -> Void

    init(_ onPress: @escaping () -> Void) {
        self.onPress = onPress
        super.init(frame: .zero)
        image = NSImage(systemSymbolName: "chevron.right.2", accessibilityDescription: "Expand the sidebar")?
            .withSymbolConfiguration(.init(pointSize: Design.Symbol.railExpand, weight: .regular))
        isBordered = false
        imagePosition = .imageOnly
        contentTintColor = Design.Color.textMuted
        refusesFirstResponder = true
        toolTip = "Expand the sidebar"
        target = self
        action = #selector(pressed)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc private func pressed() { onPress() }
}
