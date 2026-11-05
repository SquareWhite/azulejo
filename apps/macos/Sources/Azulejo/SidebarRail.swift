import AppKit

/// The collapsed sidebar as a rail (`[sidebar] collapsed = "rail"`, the
/// default), after herdr's collapsed sidebar: a narrow card down the left,
/// under the toolbar row, from the same data as the full sidebar.
///
/// - Per machine group, its first letter ("L" for Local) and connection dot,
///   or a turning ring in the state's colour while it connects or
///   reconnects; then its workspaces, each its number (as ⌘1…⌘8) and status
///   icon, the current one on the selected fill.
/// - Lower down, one row per terminal with an agent: its machine's letter
///   and the agent's status icon, in window order (workspace, tab, tile, as
///   herdr's default `agent_panel_sort = "spaces"`). Blocked rows sit on a
///   tint of the blocked colour.
/// - `»` at the bottom opens the full sidebar.
///
/// Hovering a row shows its tooltip; a click focuses the workspace or jumps
/// to the tile; right-click on a workspace gives its usual menu. The rail
/// never takes the keyboard.
@MainActor
final class SidebarRailView: NSView, ThemeAware {
    var onSelect: ((String) -> Void)?
    var onMenu: ((String) -> NSMenu?)?
    /// An agent row: focus its tile (and its workspace and tab).
    var onTile: ((String) -> Void)?
    var onExpand: (() -> Void)?

    static var width: CGFloat { Design.Space.railWidth }

    private let scroll = NSScrollView()
    private let document = RailDocument()
    private let separator = NSView()
    private lazy var expand = RailExpandButton { [weak self] in self?.onExpand?() }
    private var shown: [String] = []
    private(set) var groups: [RailGroupHeader] = []
    private(set) var workspaceRows: [RailWorkspaceRow] = []
    private(set) var agentRows: [RailAgentRow] = []

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
        separator.wantsLayer = true
        document.addSubview(separator)
        addSubview(expand)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        separator.layer?.backgroundColor = Design.Color.border.cgColor
    }

    // MARK: Model

    /// A terminal with an agent, for the rail's lower section.
    struct Agent {
        var tile: String
        var machine: String
        var status: Design.Status
        var tip: String
        var blocked: Bool
    }

    /// Every agent in the window, in window order: workspaces as the
    /// sidebar lists them, then tabs, then tiles as they cycle.
    static func agents(_ window: WindowModel, machines: [MachineModel]) -> [Agent] {
        var out: [Agent] = []
        for ws in window.workspaces {
            for tab in ws.tabs {
                for id in tab.order {
                    guard let tile = tab.tile(id), let agent = tile.agent else { continue }
                    let status = status(agent)
                    let name = agent.name.flatMap { $0.isEmpty ? nil : $0 } ?? "agent"
                    let tip = [name, "\(ws.name) / \(tab.label)", status.rawValue].joined(separator: "\n")
                    out.append(Agent(tile: id, machine: letter(ws.machineID, machines: machines),
                                     status: status, tip: tip, blocked: agent.status == .blocked))
                }
            }
        }
        return out
    }

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
        let groups = SidebarView.groups(window, machines: machines)
        let agents = Self.agents(window, machines: machines)
        var key = groups.map {
            "\($0.id)|\($0.machine?.label ?? "")|\($0.machine?.state.rawValue ?? "")|\($0.machine?.message ?? "")"
        }
        for ws in window.workspaces {
            key.append("\(ws.id)|\(ws.name)|\(ws.id == window.activeWorkspace)|\(SidebarView.status(ws).rawValue)"
                       + "|\(SidebarView.branch(ws.git) ?? "")|\(Self.counts(ws))")
        }
        key += agents.map { "\($0.tile)|\($0.machine)|\($0.status.rawValue)|\($0.tip)" }
        guard key != shown else { return }
        shown = key

        (self.groups as [NSView] + workspaceRows + agentRows).forEach { $0.removeFromSuperview() }
        self.groups = []
        workspaceRows = []
        agentRows = []
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
        for agent in agents {
            let row = RailAgentRow(agent) { [weak self] in self?.onTile?(agent.tile) }
            document.addSubview(row)
            agentRows.append(row)
        }
        separator.isHidden = agentRows.isEmpty
        needsLayout = true
    }

    /// Name and ⌘ number, branch, agent counts.
    static func tip(_ ws: WorkspaceModel, number: Int) -> String {
        var lines = [number <= 9 ? "\(ws.name) (⌘\(number))" : ws.name]
        if let branch = SidebarView.branch(ws.git) { lines.append(branch) }
        lines.append(counts(ws))
        return lines.joined(separator: "\n")
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

    /// Groups and workspaces from the top; the agents from the bottom when
    /// everything fits (herdr's layout), else straight after.
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
        let agentsHeight = CGFloat(agentRows.count) * Design.Space.railRowHeight
        let gap = Design.Space.railSectionGap
        let available = scroll.contentSize.height
        var top = y + gap
        if !agentRows.isEmpty, top + agentsHeight <= available { top = available - agentsHeight }
        separator.frame = NSRect(x: pad + 2, y: top - gap / 2, width: max(0, rowWidth - 4), height: 1)
        for (i, row) in agentRows.enumerated() {
            row.frame = NSRect(x: pad, y: top + CGFloat(i) * Design.Space.railRowHeight,
                               width: rowWidth, height: Design.Space.railRowHeight)
        }
        let end = agentRows.isEmpty ? y : top + agentsHeight
        document.frame = NSRect(x: 0, y: 0, width: width, height: max(available, end))
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

    /// Automation: agent rows as "letter:status", "!" on blocked ones.
    var agentSummary: String {
        agentRows.map { "\($0.machine):\($0.status.rawValue)\($0.blocked ? "!" : "")" }.joined(separator: "|")
    }

    /// Automation: whether every agent row lies inside the rail (visible
    /// without scrolling), and nothing overlaps.
    var fits: Bool {
        let rows: [NSView] = groups + workspaceRows + agentRows
        let visible = scroll.documentVisibleRect
        for (i, a) in rows.enumerated() {
            if !visible.contains(a.frame) { return false }
            for b in rows[(i + 1)...] where a.frame.intersects(b.frame) { return false }
        }
        return true
    }

    var expandButton: NSView { expand }
}

/// The rail's scrolling content, top down.
private final class RailDocument: NSView {
    override var isFlipped: Bool { true }
}

// MARK: - Rows

/// A rail row: hover fill, a click handler, no keyboard focus.
@MainActor
class RailRow: NSView, ThemeAware {
    fileprivate let handler: () -> Void
    fileprivate var fill: NSColor? { nil }
    private var icon: StatusIconView?
    /// Automation: the working ring is turning.
    var isSpinning: Bool { icon?.isSpinning == true }
    private var hovering = false {
        didSet { Theme.reapply(self) }
    }

    init(handler: @escaping () -> Void) {
        self.handler = handler
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.row
        layer?.cornerCurve = .continuous
        setAccessibilityRole(.button)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { false }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect], owner: self))
    }

    override func mouseEntered(with event: NSEvent) { hovering = true }
    override func mouseExited(with event: NSEvent) { hovering = false }
    override func mouseDown(with event: NSEvent) {}
    override func mouseUp(with event: NSEvent) {
        if bounds.contains(convert(event.locationInWindow, from: nil)) { handler() }
    }
    override func accessibilityPerformPress() -> Bool { handler(); return true }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    /// The selected or blocked fill, or the hover one.
    func applyTheme() {
        layer?.backgroundColor = (fill
            ?? (hovering ? Design.Color.selected.withAlphaComponent(Design.Alpha.rowHover) : nil))?.cgColor
    }

    fileprivate func updateBackground() { Theme.reapply(self) }

    /// Automation: what a click does.
    func press() { handler() }

    /// A centred label and status icon side by side.
    fileprivate func addContent(text: String, font: NSFont, color: NSColor, icon: StatusIconView) {
        self.icon = icon
        let label = NSTextField(labelWithString: text)
        label.font = font
        label.textColor = color
        label.alignment = .right
        label.lineBreakMode = .byClipping
        for view in [label, icon] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            label.trailingAnchor.constraint(equalTo: centerXAnchor, constant: -1),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
            label.leadingAnchor.constraint(greaterThanOrEqualTo: leadingAnchor, constant: 1),
            icon.leadingAnchor.constraint(equalTo: centerXAnchor, constant: 2),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: Design.Symbol.railStatus + 3),
        ])
    }
}

/// A machine group in the rail: its letter and a connection dot, or a
/// turning ring while it connects or reconnects. Not clickable.
@MainActor
final class RailGroupHeader: NSView {
    let summary: String
    fileprivate var rows: [RailWorkspaceRow] = []

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

        let label = NSTextField(labelWithString: letter)
        label.font = Design.Font.railLetter
        label.textColor = Design.Color.textMuted
        let busy = machine?.state == .connecting || machine?.state == .reconnecting
        let mark: NSView
        if busy {
            mark = StatusIconView(status, image: {
                Design.customSymbol("azulejo.ring", color: status.color, pointSize: Design.Symbol.machineRing)
            }, spins: true)
        } else {
            mark = RailDot(color: status.color)
        }
        for view in [label, mark] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            label.trailingAnchor.constraint(equalTo: centerXAnchor, constant: -1),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -3),
            mark.leadingAnchor.constraint(equalTo: centerXAnchor, constant: 3),
            mark.centerYAnchor.constraint(equalTo: label.centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { false }
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
        NSLayoutConstraint.activate([
            widthAnchor.constraint(equalToConstant: side),
            heightAnchor.constraint(equalToConstant: side),
        ])
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

/// A workspace in the rail: its number and status icon.
@MainActor
final class RailWorkspaceRow: RailRow, ContextMenuProviding {
    let number: Int
    let status: Design.Status
    let selected: Bool
    let dimmed: Bool
    var menuProvider: (() -> NSMenu?)?

    init(_ ws: WorkspaceModel, number: Int, status: Design.Status, selected: Bool, dimmed: Bool,
         tip: String, handler: @escaping () -> Void) {
        self.number = number
        self.status = status
        self.selected = selected
        self.dimmed = dimmed
        super.init(handler: handler)
        toolTip = tip
        alphaValue = dimmed ? Design.Alpha.dimmed : 1
        setAccessibilityLabel("\(ws.name), \(status.rawValue)")
        let icon = StatusIconView(status) { SidebarView.icon(status, pointSize: Design.Symbol.railStatus) }
        addContent(text: "\(number)", font: Design.Font.railNumber,
                   color: selected ? Design.Color.textPrimary : Design.Color.textSecondary, icon: icon)
        updateBackground()
    }

    override fileprivate var fill: NSColor? { selected ? Design.Color.selected : nil }

    override func menu(for event: NSEvent) -> NSMenu? { contextMenu() }
    func contextMenu() -> NSMenu? { menuProvider?() }
}

/// An agent in the rail: its machine's letter and status icon. Blocked
/// ones sit on a tint of the blocked colour.
@MainActor
final class RailAgentRow: RailRow {
    let machine: String
    let status: Design.Status
    let blocked: Bool

    init(_ agent: SidebarRailView.Agent, handler: @escaping () -> Void) {
        machine = agent.machine
        status = agent.status
        blocked = agent.blocked
        super.init(handler: handler)
        toolTip = agent.tip
        setAccessibilityLabel(agent.tip.replacingOccurrences(of: "\n", with: ", "))
        let state = agent.status
        let icon = StatusIconView(state) { SidebarView.icon(state, pointSize: Design.Symbol.railStatus) }
        addContent(text: machine, font: Design.Font.railLetter,
                   color: blocked ? Design.Status.blocked.color : Design.Color.textMuted, icon: icon)
        updateBackground()
    }

    override fileprivate var fill: NSColor? {
        blocked ? Design.Status.blocked.color.withAlphaComponent(Design.Alpha.railAttention) : nil
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
