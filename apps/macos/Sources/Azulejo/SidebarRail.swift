import AppKit

/// The collapsed sidebar as a rail (`[sidebar] collapsed = "rail"`, the
/// default), after herdr's collapsed sidebar: a narrow card down the left,
/// under the toolbar row, from the same data as the full sidebar.
///
/// - Per machine group, its first letter ("L" for Local) and connection dot,
///   or a turning ring in the state's colour while it connects or
///   reconnects; then its workspaces, each its number (as ⌘1…⌘8) and status
///   icon, the current one on the selected fill.
/// - `»` at the bottom opens the full sidebar.
///
/// Rows keep the same inset on both sides, and their number and icon sit
/// centred between them (icons in one column, whatever the numbers).
/// Hovering a row shows its tooltip; a click focuses the workspace; a drag
/// reorders it within its group (`WorkspaceDrag`); right-click gives its
/// usual menu. The rail never takes the keyboard.
@MainActor
final class SidebarRailView: NSView, WorkspaceDragHost {
    var onSelect: ((String) -> Void)?
    var onMenu: ((String) -> NSMenu?)?
    var onExpand: (() -> Void)?
    /// A dropped drag: move the workspace to an index in the window.
    var onMove: ((String, Int) -> Void)?

    static var width: CGFloat { Design.Space.railWidth }

    private let scroll = NSScrollView()
    private let document = RailDocument()
    private lazy var expand = RailExpandButton { [weak self] in self?.onExpand?() }
    private var shown: [String] = []
    private var order: [String] = []
    /// A refresh held back while a drag runs.
    private var pending: (window: WindowModel, machines: [MachineModel])?
    private(set) var groups: [RailGroupHeader] = []
    private(set) var workspaceRows: [RailWorkspaceRow] = []

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { false }
    /// A click between rows does nothing (the card would move the window).
    override func mouseDown(with event: NSEvent) {}

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        scroll.documentView = document
        scroll.drawsBackground = false
        // Too narrow for a scroller; a trackpad or wheel still scrolls.
        scroll.hasVerticalScroller = false
        scroll.verticalScrollElasticity = .allowed
        addSubview(scroll)
        addSubview(expand)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    // MARK: Model

    /// One agent's icon, ranked as the sidebar ranks a workspace: blocked,
    /// then finished and not read, then working, else idle (a read finish
    /// is idle even while herdr still says `done`).
    static func status(_ agent: AgentModel) -> Design.Status {
        if agent.status == .blocked { return .blocked }
        if agent.attention { return .done }
        return agent.status == .working ? .working : .idle
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
        for ws in window.workspaces {
            key.append("\(ws.id)|\(ws.name)|\(ws.id == window.activeWorkspace)|\(SidebarView.status(ws).rawValue)"
                       + "|\(SidebarView.branch(ws.git) ?? "")|\(Self.counts(ws))")
        }
        guard key != shown else { return }
        shown = key

        (self.groups as [NSView] + workspaceRows).forEach { $0.removeFromSuperview() }
        self.groups = []
        workspaceRows = []
        for group in groups {
            let header = RailGroupHeader(group: group, letter: Self.letter(group.id, machines: machines))
            document.addSubview(header)
            self.groups.append(header)
            let dimmed = group.machine.map { $0.state != .connected } ?? true
            for (number, ws) in group.workspaces {
                let row = RailWorkspaceRow(
                    ws, number: number, status: SidebarView.status(ws),
                    selected: ws.id == window.activeWorkspace, dimmed: dimmed,
                    tip: Self.tip(ws, number: number)
                ) { [weak self] in self?.onSelect?(ws.id) }
                row.menuProvider = { [weak self] in self?.onMenu?(ws.id) }
                header.rows.append(row)
                document.addSubview(row)
                workspaceRows.append(row)
            }
        }
        // Every number takes the widest one's room, so the icons line up.
        let widest = workspaceRows.map(\.numberWidth).max() ?? 0
        for row in workspaceRows { row.numberColumn = widest }
        needsLayout = true
    }

    /// Name and ⌘ number, branch, agent counts.
    static func tip(_ ws: WorkspaceModel, number: Int) -> String {
        var lines = [number <= 9 ? "\(ws.name) (⌘\(number))" : ws.name]
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
        let pad = Design.Space.railPadding
        let expandHeight = Design.Space.railExpandHeight
        expand.frame = NSRect(x: pad, y: bounds.height - expandHeight - pad / 2,
                              width: max(0, bounds.width - pad * 2), height: expandHeight)
        scroll.frame = NSRect(x: 0, y: pad, width: bounds.width, height: max(0, expand.frame.minY - pad))
        layoutDocument()
    }

    /// Groups and their workspaces from the top, the same inset on both sides.
    private func layoutDocument() {
        let pad = Design.Space.railPadding
        let width = scroll.bounds.width
        let rowWidth = max(0, width - pad * 2)
        var y: CGFloat = 0
        for header in groups {
            header.frame = NSRect(x: pad, y: y, width: rowWidth, height: Design.Space.railGroupHeight)
            y += Design.Space.railGroupHeight
            for row in header.rows {
                row.frame = NSRect(x: pad, y: y, width: rowWidth, height: Design.Space.railRowHeight)
                y += Design.Space.railRowHeight
            }
        }
        document.frame = NSRect(x: 0, y: 0, width: width, height: max(scroll.contentSize.height, y))
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
    /// (its number and icon, as drawn), and each row's inset from the rail
    /// on the left and right ("0|0|0" and "4/4|4/4"), so a lopsided rail fails.
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
}

/// The rail's scrolling content, top down.
private final class RailDocument: NSView {
    override var isFlipped: Bool { true }
}

// MARK: - Content

/// A label and a mark (status icon or dot) side by side, centred together
/// in their row: the label right-aligned in a column `column` points wide,
/// then `Design.Space.railContentGap`, then the mark.
@MainActor
private enum RailContent {
    /// The width `text` takes in `font`.
    static func width(_ text: String, font: NSFont) -> CGFloat {
        ceil((text as NSString).size(withAttributes: [.font: font]).width)
    }

    /// Frames for the label and the mark; `column` is the label's room.
    /// Returns how far the drawn pair sits off the middle (0 when centred).
    @discardableResult
    static func layout(label: NSTextField, text: CGFloat, column: CGFloat, mark: NSView, markSize: NSSize,
                       in bounds: NSRect, labelY: CGFloat? = nil) -> CGFloat {
        let gap = Design.Space.railContentGap
        let total = column + gap + markSize.width
        let left = (bounds.width - total) / 2
        // A label draws its text 2 points in from each side of its frame
        // (its cell size; its intrinsic size leaves that out): line its
        // text, not its frame, up with the column's right edge.
        let field = label.cell?.cellSize ?? label.intrinsicContentSize
        let inset = max(0, (field.width - text) / 2)
        let textRight = left + column
        label.frame = NSRect(x: textRight + inset - field.width, y: labelY ?? (bounds.height - field.height) / 2,
                             width: field.width, height: field.height)
        mark.frame = NSRect(x: textRight + gap, y: (bounds.height - markSize.height) / 2,
                            width: markSize.width, height: markSize.height)
        let drawnLeft = textRight - text
        let drawnRight = textRight + gap + markSize.width
        return (drawnLeft + drawnRight) / 2 - bounds.width / 2
    }
}

/// A machine group in the rail: its letter and a connection dot, or a
/// turning ring while it connects or reconnects. Not clickable.
@MainActor
final class RailGroupHeader: NSView {
    let summary: String
    fileprivate var rows: [RailWorkspaceRow] = []
    private let label: NSTextField
    private let mark: NSView
    private let letterWidth: CGFloat
    private(set) var contentOffset: CGFloat = 0

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
        label = NSTextField(labelWithString: letter)
        label.font = Design.Font.railLetter
        label.textColor = Design.Color.textMuted
        letterWidth = RailContent.width(letter, font: Design.Font.railLetter)
        let busy = machine?.state == .connecting || machine?.state == .reconnecting
        if busy {
            mark = StatusIconView(status, image: {
                Design.customSymbol("azulejo.ring", color: status.color, pointSize: Design.Symbol.machineRing)
            }, spins: true)
        } else {
            mark = RailDot(color: status.color)
        }
        super.init(frame: .zero)
        let state: String? = switch machine?.state {
        case .connected, nil: nil
        case .connecting: "Connecting…"
        case .reconnecting: "Reconnecting…"
        case .error: "Needs attention"
        case .off: "Off"
        }
        toolTip = [machine?.label ?? "machine \(group.id.prefix(8))", state ?? machine.map { _ in "Connected" } ?? "Not in herdr",
                   machine?.message].compactMap { $0 }.joined(separator: "\n")
        addSubview(label)
        addSubview(mark)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { false }

    override func layout() {
        super.layout()
        let dot = Design.Radius.machineDot * 2
        let ring = mark.intrinsicContentSize
        let size = mark is RailDot ? NSSize(width: dot, height: dot)
            : ring.width > 0 ? ring : NSSize(width: Design.Symbol.machineRing, height: Design.Symbol.machineRing)
        // The letter sits low in its row, as the group's label does in the
        // full sidebar; the mark on the letter's middle.
        let field = label.cell?.cellSize ?? label.intrinsicContentSize
        let labelY: CGFloat = isFlipped ? bounds.height - 3 - field.height : 3
        contentOffset = RailContent.layout(label: label, text: letterWidth, column: letterWidth, mark: mark,
                                           markSize: size, in: bounds, labelY: labelY)
        mark.frame.origin.y = label.frame.midY - size.height / 2
    }
}

/// A machine's connection dot in the rail.
@MainActor
private final class RailDot: NSView, ThemeAware {
    private let color: NSColor

    init(color: NSColor) {
        self.color = color
        let side = Design.Radius.machineDot * 2
        super.init(frame: NSRect(x: 0, y: 0, width: side, height: side))
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.machineDot
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        layer?.backgroundColor = color.cgColor
    }
}

// MARK: - Rows

/// A workspace in the rail: its number and status icon, centred; the
/// selected fill or the hover one; a click, a drag or a right-click.
@MainActor
final class RailWorkspaceRow: NSView, ThemeAware, ContextMenuProviding, WorkspaceDragRow {
    let number: Int
    let status: Design.Status
    let selected: Bool
    let dimmed: Bool
    let workspaceID: String
    let groupID: String
    var dragName: String { "\(number)" }
    var menuProvider: (() -> NSMenu?)?
    /// The width of this row's number as drawn.
    let numberWidth: CGFloat
    /// The room every number in the rail takes (the widest's), set by the rail.
    var numberColumn: CGFloat = 0 {
        didSet { needsLayout = true }
    }
    private(set) var contentOffset: CGFloat = 0
    private let handler: () -> Void
    private let label: NSTextField
    private let icon: StatusIconView
    private var pressing = RowPress()
    /// Automation: the working ring is turning.
    var isSpinning: Bool { icon.isSpinning }
    private var hovering = false {
        didSet { Theme.reapply(self) }
    }

    init(_ ws: WorkspaceModel, number: Int, status: Design.Status, selected: Bool, dimmed: Bool,
         tip: String, handler: @escaping () -> Void) {
        self.number = number
        self.status = status
        self.selected = selected
        self.dimmed = dimmed
        self.handler = handler
        workspaceID = ws.id
        groupID = ws.machineID
        label = NSTextField(labelWithString: "\(number)")
        label.font = Design.Font.railNumber
        label.textColor = selected ? Design.Color.textPrimary : Design.Color.textSecondary
        label.alignment = .right
        label.lineBreakMode = .byClipping
        numberWidth = RailContent.width("\(number)", font: Design.Font.railNumber)
        icon = StatusIconView(status) { SidebarView.icon(status, pointSize: Design.Symbol.railStatus) }
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.row
        layer?.cornerCurve = .continuous
        setAccessibilityRole(.button)
        toolTip = tip
        alphaValue = dimmed ? Design.Alpha.dimmed : 1
        setAccessibilityLabel("\(ws.name), \(status.rawValue)")
        addSubview(label)
        addSubview(icon)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { false }

    override func layout() {
        super.layout()
        let side = Design.Symbol.railStatus + 3
        contentOffset = RailContent.layout(
            label: label, text: numberWidth, column: max(numberColumn, numberWidth), mark: icon,
            markSize: NSSize(width: side, height: side), in: bounds)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect], owner: self))
    }

    override func mouseEntered(with event: NSEvent) { hovering = true }
    override func mouseExited(with event: NSEvent) { hovering = false }
    override func mouseDown(with event: NSEvent) { pressing.down(event) }
    override func mouseDragged(with event: NSEvent) { pressing.dragged(event, row: self) }
    override func mouseUp(with event: NSEvent) {
        if pressing.up(event, in: self) { handler() }
    }
    override func accessibilityPerformPress() -> Bool { handler(); return true }
    override func menu(for event: NSEvent) -> NSMenu? { contextMenu() }
    func contextMenu() -> NSMenu? { menuProvider?() }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    /// The selected fill, or the hover one.
    func applyTheme() {
        layer?.backgroundColor = (selected ? Design.Color.selected
            : hovering ? Design.Color.selected.withAlphaComponent(Design.Alpha.rowHover) : nil)?.cgColor
    }

    /// Automation: what a click does.
    func press() { handler() }

    /// Automation: the pointer over it, or not (its hover fill).
    func hover(_ on: Bool) { hovering = on }
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
