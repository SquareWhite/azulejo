import AppKit

/// What's inside the sidebar card (D.3, docs/design/mockup-v1.png): an
/// Agents | Files switch, then either the workspace list or the focused
/// workspace's file tree, and a footer counting workspaces by agent state.
/// The card itself, its top band with the traffic lights and the sidebar
/// toggle belong to the window chrome (`SidebarCardView`, D.2); this view
/// sits below that band and draws no background of its own.
///
/// Agents: one group per herdr machine (ADR 0011) — its name, a connection
/// dot or "Reconnecting…", its workspaces (status icon, name, git branch on
/// the right) and a "+" that creates a workspace there. Rows of a machine
/// that isn't connected are dimmed.
///
/// Files: a `files.panel` the core lists and watches like a files tile, on
/// the focused workspace's repository (else its cwd). Return opens a file
/// beside the focused tile; Esc hands the keyboard back to the tiles. A
/// remote workspace shows its terminal's folder on its machine, listed over
/// SSH (ADR 0015).
@MainActor
final class SidebarView: NSView, KeepsKeyboard, ThemeAware, WorkspaceDragHost {
    enum Mode: String { case agents, files }

    static var width: CGFloat { Design.Space.sidebarWidth }
    var onSelect: ((String) -> Void)?
    /// "+" on a machine group: create a workspace on that machine.
    var onAdd: ((String) -> Void)?
    /// A workspace row's right-click menu.
    var onMenu: ((String) -> NSMenu?)?
    /// Esc in the Files view: give the keyboard back to the focused tile.
    var onReturnFocus: (() -> Void)?
    /// A dropped drag: move the workspace to an index in the window.
    var onMove: ((String, Int) -> Void)?

    private(set) var mode: Mode
    private let segments = SidebarSegments()
    private let list = NSScrollView()
    private let stack = NSStackView()
    private let files = FlippedView()
    private let filesTitle = NSTextField(labelWithString: "")
    private let filesNote = NSTextField(wrappingLabelWithString: "")
    private let footer = SidebarFooter()
    private let footerLine = NSView()
    private var shown: [String] = []
    private var window_: WindowModel?
    private var machines: [MachineModel] = []
    /// A refresh held back while a drag runs.
    private var pending: (window: WindowModel, machines: [MachineModel])?

    // The Files view's panel and what it shows.
    private var panelID: String?
    private var panelRoot: String?
    /// The herdr machine the panel's files are on; nil for Local.
    private var panelMachine: String?
    private var panelView: FilesTileView?
    /// A `files.panel` in flight (the core checks the folder on disk, so
    /// the main thread doesn't wait for it): its root and machine, and a
    /// number that tells its reply from an older one's.
    private var panelOpening: (root: String, machine: String?, request: Int)?
    private var panelRequests = 0

    override var isFlipped: Bool { true }
    /// A click on empty space does nothing (the card it sits in would move
    /// the window).
    override func mouseDown(with event: NSEvent) {}

    override init(frame: NSRect) {
        mode = Mode(rawValue: UserDefaults.standard.string(forKey: Self.modeKey) ?? "") ?? .agents
        super.init(frame: frame)
        wantsLayer = true

        segments.selected = mode
        segments.onChange = { [weak self] mode in self?.setMode(mode, focus: true) }
        addSubview(segments)

        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 0
        stack.edgeInsets = NSEdgeInsets(top: 4, left: 0, bottom: 8, right: 0)
        stack.translatesAutoresizingMaskIntoConstraints = false
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(stack)
        list.documentView = document
        list.drawsBackground = false
        list.hasVerticalScroller = true
        list.autohidesScrollers = true
        list.scrollerStyle = .overlay
        NSLayoutConstraint.activate([
            document.topAnchor.constraint(equalTo: list.contentView.topAnchor),
            document.leadingAnchor.constraint(equalTo: list.contentView.leadingAnchor),
            document.widthAnchor.constraint(equalTo: list.contentView.widthAnchor),
            stack.topAnchor.constraint(equalTo: document.topAnchor),
            stack.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            stack.bottomAnchor.constraint(equalTo: document.bottomAnchor),
        ])
        addSubview(list)

        filesTitle.font = Design.Font.groupLabel
        filesTitle.textColor = Design.Color.textMuted
        filesTitle.lineBreakMode = .byTruncatingMiddle
        filesNote.font = Design.Font.groupLabel
        filesNote.textColor = Design.Color.textMuted
        files.addSubview(filesTitle)
        files.addSubview(filesNote)
        addSubview(files)

        footerLine.wantsLayer = true
        addSubview(footerLine)
        addSubview(footer)
        applyMode()
        Theme.track(self)
    }

    func applyTheme() {
        footerLine.layer?.backgroundColor = Design.Color.border.cgColor
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    // MARK: Layout

    override func layout() {
        super.layout()
        let pad = Design.Space.sidebarPadding
        let width = bounds.width
        segments.frame = NSRect(x: pad, y: Design.Space.sidebarSegmentTop, width: width - pad * 2,
                                height: Design.Space.sidebarSegmentHeight)
        let footerHeight = Design.Space.sidebarFooterHeight
        footer.frame = NSRect(x: pad, y: bounds.height - footerHeight, width: width - pad * 2, height: footerHeight)
        footerLine.frame = NSRect(x: pad, y: footer.frame.minY - 1, width: width - pad * 2, height: 1)
        let bodyTop = segments.frame.maxY + Design.Space.sidebarListTop
        let body = NSRect(x: 0, y: bodyTop, width: width, height: max(0, footerLine.frame.minY - bodyTop))
        list.frame = body
        files.frame = body
        filesTitle.frame = NSRect(x: pad + 9, y: 2, width: body.width - pad * 2 - 9, height: 16)
        filesNote.frame = NSRect(x: pad + 9, y: 24, width: body.width - pad * 2 - 18, height: 60)
        panelView?.frame = NSRect(x: 2, y: 22, width: body.width - 4, height: max(0, body.height - 22))
    }

    // MARK: Model

    /// A machine group: its machine (nil for workspaces whose machine left
    /// herdr's list) and its workspaces with their numbers (⌘1…⌘8).
    struct Group {
        var id: String
        var machine: MachineModel?
        var workspaces: [(number: Int, workspace: WorkspaceModel)]
    }

    /// Local first, then herdr's machines in `machines.list` order, then any
    /// machine a workspace still names but herdr no longer lists.
    static func groups(_ window: WindowModel, machines: [MachineModel]) -> [Group] {
        var groups: [Group] = []
        var known = Set<String>()
        let local = machines.first(where: \.isLocal)
            ?? MachineModel(id: MachineModel.local, label: "Local", state: .connected)
        for machine in [local] + machines.filter({ !$0.isLocal }) where known.insert(machine.id).inserted {
            groups.append(Group(id: machine.id, machine: machine, workspaces: []))
        }
        for ws in window.workspaces {
            if known.insert(ws.machineID).inserted {
                groups.append(Group(id: ws.machineID, machine: nil, workspaces: []))
            }
            if let g = groups.firstIndex(where: { $0.id == ws.machineID }) {
                groups[g].workspaces.append((0, ws))
            }
        }
        // Numbers run top to bottom across the groups (ADR 0013): the core
        // keeps the window's list grouped the same way, so they're also its
        // `workspace.list` numbers.
        var number = 0
        for g in groups.indices {
            for w in groups[g].workspaces.indices {
                number += 1
                groups[g].workspaces[w].number = number
            }
        }
        return groups
    }

    /// The window's workspaces as the sidebar and the rail show them, top
    /// to bottom: what ⌘1…⌘8, ⌘9 and next/previous workspace step through,
    /// and which way a workspace slide goes (further down is "below").
    static func ordered(_ window: WindowModel, machines: [MachineModel]) -> [WorkspaceModel] {
        groups(window, machines: machines).flatMap { $0.workspaces.map(\.workspace) }
    }

    /// The chord that reaches workspace `number` of `count`: ⌘1…⌘8, and ⌘9
    /// for the last one (as `switch_workspace_last`); nil past ⌘8 otherwise.
    static func chord(_ number: Int, of count: Int) -> String? {
        number <= 8 ? "⌘\(number)" : number == count ? "⌘9" : nil
    }

    func apply(_ window: WindowModel, machines: [MachineModel] = []) {
        if WorkspaceDrag.current?.hosted(by: self) == true {
            pending = (window, machines)
            return
        }
        pending = nil
        window_ = window
        self.machines = machines
        if mode == .files { syncPanel() }
        let statuses = Dictionary(uniqueKeysWithValues: window.workspaces.map { ($0.id, Self.status($0)) })
        let rollups = Dictionary(uniqueKeysWithValues: window.workspaces.map {
            ($0.id, AttentionStyle.Rollup($0.tabs.flatMap(\.tiles)))
        })
        let groups = Self.groups(window, machines: machines)
        var key = window.workspaces.map { ws in
            let r = rollups[ws.id]
            return "\(ws.id)|\(ws.name)|\(ws.machineID)|\(ws.id == window.activeWorkspace)"
                + "|\(statuses[ws.id]?.rawValue ?? "")|\(r?.blocked ?? 0)|\(r?.attention ?? 0)|\(r?.working ?? 0)"
                + "|\(ws.git?.branch ?? "")|\(ws.git?.detached == true)|\(ws.git?.dirty == true)"
        }
        key += groups.map { "\($0.id)|\($0.machine?.label ?? "")|\($0.machine?.state.rawValue ?? "")|\($0.machine?.message ?? "")" }
        guard key != shown else { return }
        shown = key
        footer.apply(window.workspaces.map { statuses[$0.id] ?? .idle })
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for group in groups {
            let header = MachineHeader(group: group)
            add(header, height: Design.Space.sidebarGroupHeight)
            let dimmed = group.machine.map { $0.state != .connected } ?? true
            for (number, ws) in group.workspaces {
                let row = WorkspaceRow(
                    ws, number: number, of: window.workspaces.count, status: statuses[ws.id] ?? .idle,
                    marker: rollups[ws.id]?.marker, selected: ws.id == window.activeWorkspace,
                    dimmed: dimmed
                ) { [weak self] in self?.onSelect?(ws.id) }
                row.menuProvider = { [weak self] in self?.onMenu?(ws.id) }
                add(row, height: Design.Space.sidebarRowHeight)
            }
            // No "+" where nothing would run: off, or gone from herdr's list.
            if let machine = group.machine, machine.state != .off {
                let addRow = AddRow(machine: machine.label) { [weak self] in self?.onAdd?(machine.id) }
                add(addRow, height: Design.Space.sidebarAddHeight)
            }
        }
    }

    private func add(_ view: NSView, height: CGFloat) {
        stack.addArrangedSubview(view)
        view.translatesAutoresizingMaskIntoConstraints = false
        let pad = Design.Space.sidebarPadding
        NSLayoutConstraint.activate([
            view.leadingAnchor.constraint(equalTo: stack.leadingAnchor, constant: pad),
            view.trailingAnchor.constraint(equalTo: stack.trailingAnchor, constant: -pad),
            view.heightAnchor.constraint(equalToConstant: height),
        ])
    }

    /// One state for a workspace's row and the footer: blocked wins, then a
    /// finished agent nobody has read, then working, else idle. A read
    /// finish is idle even while herdr still says `done` (only `attention`
    /// knows it was read).
    static func status(_ ws: WorkspaceModel) -> Design.Status {
        let agents = ws.tabs.flatMap(\.tiles).compactMap(\.agent)
        if agents.contains(where: { $0.status == .blocked }) { return .blocked }
        if agents.contains(where: \.attention) { return .done }
        if agents.contains(where: { $0.status == .working }) { return .working }
        return .idle
    }

    // MARK: Agents | Files

    private static var modeKey: String {
        (ProcessInfo.processInfo.environment["AZULEJO_PROFILE"] == "dev" ? "dev." : "") + "sidebarMode"
    }

    /// Switches the view; with `focus`, the Files tree takes the keyboard.
    func setMode(_ mode: Mode, focus: Bool = false) {
        guard mode != self.mode else {
            if focus, mode == .files { panelView?.focusTree() }
            return
        }
        self.mode = mode
        segments.selected = mode
        UserDefaults.standard.set(mode.rawValue, forKey: Self.modeKey)
        applyMode()
        if mode == .files, focus { panelView?.focusTree() }
    }

    private func applyMode() {
        list.isHidden = mode != .agents
        files.isHidden = mode != .files
        if mode == .files {
            syncPanel()
        } else {
            closePanel()
        }
        needsLayout = true
    }

    /// The folder the Files view shows for the focused workspace (on its
    /// machine; nil for Local), or why it shows none.
    private func filesRoot() -> (root: String?, machine: String?, note: String?) {
        guard let window = window_, let ws = window.workspace else { return (nil, nil, "No workspace.") }
        let focused = ws.tab?.tile(ws.tab?.focused ?? "")
        if ws.machineID != MachineModel.local {
            // A remote workspace: its terminal's folder there (ADR 0015).
            let machine = machines.first { $0.id == ws.machineID }
            let name = machine?.label ?? ws.machineID
            // While it reconnects the panel stays, with the last listing
            // and the core's error; off or gone from herdr, there's no link.
            guard let state = machine?.state, state != .off else {
                return (nil, nil, "\(name) isn't connected. \(ws.name)'s files show here once it is.")
            }
            let terminals = [focused].compactMap { $0 } + (ws.tab?.tiles ?? [])
            let candidates = terminals.filter { $0.kind == .terminal }.flatMap { [$0.pane?.cwd, $0.cwd] } + [ws.cwd]
            guard let root = candidates.compactMap({ $0 }).first(where: { $0.hasPrefix("/") }) else {
                return (nil, nil, "Waiting for \(ws.name)'s folder on \(name).")
            }
            return (root, ws.machineID, nil)
        }
        let candidates = [ws.git?.repo, ws.cwd, focused?.cwd, focused?.root]
        if let root = candidates.compactMap({ $0 }).first(where: { !$0.isEmpty }) { return (root, nil, nil) }
        return (NSHomeDirectory(), nil, nil)
    }

    private func syncPanel() {
        let (root, machine, note) = filesRoot()
        filesNote.stringValue = note ?? ""
        filesNote.isHidden = note == nil
        guard let root else {
            closePanel()
            filesTitle.stringValue = ""
            return
        }
        let label = machine.map { id in machines.first { $0.id == id }?.label ?? id }
        filesTitle.stringValue = label.map { "\($0): \(root)" } ?? Self.tilde(root)
        filesTitle.toolTip = label.map { "\(root) on \($0)" } ?? root
        if let opening = panelOpening {
            // On its way already; another root in flight is overtaken below.
            if opening.root == root, opening.machine == machine { return }
        } else if root == panelRoot, machine == panelMachine, panelView != nil {
            return
        }
        panelRequests += 1
        let request = panelRequests
        panelOpening = (root, machine, request)
        FilesTileView.openPanel(root: root, machine: machine, reuse: panelID) { [weak self] id in
            guard let self, self.panelOpening?.request == request else {
                // Closed or re-rooted meanwhile: don't leave a stray panel
                // in the core.
                if let id, id != self?.panelID { FilesTileView.closePanel(id) }
                return
            }
            self.panelOpening = nil
            self.showPanel(id, root: root, machine: machine)
        }
    }

    private func showPanel(_ id: String?, root: String, machine: String?) {
        guard let id else {
            closePanel()
            filesNote.stringValue = "Can't list \(Self.tilde(root))."
            filesNote.isHidden = false
            return
        }
        panelRoot = root
        panelMachine = machine
        if let view = panelView, id == panelID, view.machine == machine {
            view.update(TileModel(id: id, kind: .files, root: root))
        } else {
            panelView?.tileWillClose()
            panelView?.removeFromSuperview()
            let view = FilesTileView(panel: id, root: root, machine: machine)
            view.onEscape = { [weak self] in self?.onReturnFocus?() }
            files.addSubview(view)
            panelView = view
        }
        panelID = id
        needsLayout = true
    }

    /// Stops the Files view's panel (switching to Agents, closing the window).
    func closePanel() {
        panelOpening = nil
        if let panelID { FilesTileView.closePanel(panelID) }
        panelView?.tileWillClose()
        panelView?.removeFromSuperview()
        panelView = nil
        panelID = nil
        panelRoot = nil
        panelMachine = nil
    }

    /// A status icon for the sidebar. Filled symbols (blocked's
    /// `exclamationmark.circle.fill`) get a dark glyph on the status colour,
    /// as in the mockup, rather than one flat disc.
    static func icon(_ status: Design.Status, pointSize: CGFloat) -> NSImage? {
        guard status.symbol.hasSuffix(".fill") else { return Design.StatusIcon.image(status, pointSize: pointSize) }
        let config = NSImage.SymbolConfiguration(pointSize: pointSize, weight: .semibold)
            .applying(.init(paletteColors: [Theme.resolved(Design.Color.window), Theme.resolved(status.color)]))
        return NSImage(systemSymbolName: status.symbol, accessibilityDescription: status.rawValue)?
            .withSymbolConfiguration(config)
    }

    static func tilde(_ path: String) -> String {
        let home = NSHomeDirectory()
        return path == home ? "~" : path.hasPrefix(home + "/") ? "~" + path.dropFirst(home.count) : path
    }

}

extension SidebarView {
    var rows: [WorkspaceRow] { stack.arrangedSubviews.compactMap { $0 as? WorkspaceRow } }

    // MARK: Dragging (WorkspaceDragHost)

    var dragRows: [WorkspaceDragRow] { mode == .agents ? rows : [] }
    var dragOrder: [String] { (pending?.window ?? window_)?.workspaces.map(\.id) ?? [] }
    var dropArea: NSRect {
        convert(bounds, to: nil).insetBy(dx: -Design.Space.dropMargin, dy: -Design.Space.dropMargin)
    }

    func dragMove(_ workspace: String, to index: Int) { onMove?(workspace, index) }

    func dragEnded() {
        if let pending { apply(pending.window, machines: pending.machines) }
    }

    /// Automation: each workspace row's attention marker (" ● 2"), in order.
    var markers: [String] { rows.map(\.markerText) }

    /// Automation: each machine group's header text ("dev-server ● Reconnecting…").
    var machineHeaders: [String] {
        stack.arrangedSubviews.compactMap { ($0 as? MachineHeader)?.summary }
    }

    /// Automation: each workspace row's number and name, in order ("1 alpha").
    var numbers: [String] { rows.map { "\($0.number) \($0.dragName)" } }

    /// Automation: each workspace row's branch, in order ("" outside a repo).
    var branches: [String] { rows.map(\.detailText) }

    /// Automation: each workspace row's status icon state, in order.
    var statuses: [String] { rows.map(\.status.rawValue) }

    /// Automation: the footer's counts ("working=2,blocked=1,idle=3,done=0").
    var footerCounts: String { footer.summary }

    /// Automation: the Files view's panel, when it's shown.
    var filesPanel: FilesTileView? { mode == .files ? panelView : nil }

    /// Automation: the Files view's title (its folder, with the machine for
    /// a remote one) and its note ("" when none shows).
    var filesTitleText: String { filesTitle.stringValue }
    var filesNoteText: String { filesNote.isHidden ? "" : filesNote.stringValue }

    /// The branch a row shows (F26): its name, `@abc1234` when detached, and
    /// a dot when tracked files changed.
    static func branch(_ git: GitModel?) -> String? {
        guard let git, let branch = git.branch, !branch.isEmpty else { return nil }
        let name = git.detached == true ? "@\(branch)" : branch
        return git.dirty == true ? "\(name) •" : name
    }
}

/// A flipped plain view: the sidebar list's document grows downwards.
private final class FlippedView: NSView {
    override var isFlipped: Bool { true }
}

// MARK: - Rows

/// A machine group's header: its name, then on the right a connection dot,
/// or a turning arc in the state's colour (the mockup's orange for
/// reconnecting) and its state while it connects or reconnects.
@MainActor
final class MachineHeader: NSView {
    /// Automation: what the header says ("dev-server ● Reconnecting…").
    let summary: String

    init(group: SidebarView.Group) {
        let machine = group.machine
        let (status, state): (Design.Status, String?) = switch machine?.state {
        case .connected: (.connected, nil)
        case .connecting: (.connecting, "Connecting…")
        case .reconnecting: (.reconnecting, "Reconnecting…")
        case .error: (.error, "Needs attention")
        case .off: (.off, "Off")
        case nil: (.off, "Not in herdr")
        }
        let title = machine?.label ?? "machine \(group.id.prefix(8))"
        summary = [title, "●", state].compactMap { $0 }.joined(separator: " ")
        super.init(frame: .zero)
        toolTip = machine?.message

        let name = NSTextField(labelWithString: title)
        name.font = Design.Font.groupLabel
        name.textColor = Design.Color.textMuted
        name.lineBreakMode = .byTruncatingTail
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        let right = NSStackView()
        right.orientation = .horizontal
        right.spacing = 5
        right.alignment = .centerY
        let busy = machine?.state == .connecting || machine?.state == .reconnecting
        if busy {
            // The app's own ring, not the system spinner, which can't take a
            // colour. It holds still with motion off or Reduce Motion on.
            let spinner = StatusIconView(status, image: {
                Design.customSymbol("azulejo.ring", color: status.color, pointSize: Design.Symbol.machineRing)
            }, spins: true)
            spinner.translatesAutoresizingMaskIntoConstraints = false
            spinner.setContentHuggingPriority(.required, for: .horizontal)
            right.addArrangedSubview(spinner)
        }
        if let state {
            let text = NSTextField(labelWithString: state)
            text.font = Design.Font.caption
            text.textColor = machine?.state == .off || machine == nil ? Design.Color.textMuted : status.color
            text.lineBreakMode = .byTruncatingTail
            text.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            right.addArrangedSubview(text)
        }
        if !busy {
            right.addArrangedSubview(Dot(color: status.color))
        }

        for view in [name, right] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            name.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 9),
            name.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -5),
            right.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -9),
            right.centerYAnchor.constraint(equalTo: name.centerYAnchor),
            right.leadingAnchor.constraint(greaterThanOrEqualTo: name.trailingAnchor, constant: 6),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

}

/// A small filled circle: a machine's connection state.
@MainActor
private final class Dot: NSView, ThemeAware {
    private let color: NSColor

    init(color: NSColor) {
        self.color = color
        let side = Design.Radius.machineDot * 2
        super.init(frame: NSRect(x: 0, y: 0, width: side, height: side))
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.machineDot
        Theme.track(self)
        translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            widthAnchor.constraint(equalToConstant: side),
            heightAnchor.constraint(equalToConstant: side),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func applyTheme() {
        layer?.backgroundColor = color.cgColor
    }
}

/// A workspace: status icon, name, and its branch on the right. A click
/// focuses it; a drag reorders it within its group (`WorkspaceDrag`).
@MainActor
final class WorkspaceRow: NSView, ContextMenuProviding, ThemeAware, WorkspaceDragRow {
    /// Its number in the sidebar (⌘1…⌘8), top to bottom across groups.
    let number: Int
    let status: Design.Status
    let workspaceID: String
    let groupID: String
    let dragName: String
    /// Right-click: Rename, New Tab, Close and the folder's items.
    var menuProvider: (() -> NSMenu?)?
    /// Automation: the attention marker (" ● 2"), or "".
    let markerText: String
    /// Automation: the branch, or "".
    let detailText: String
    private let handler: () -> Void
    private let selected: Bool
    private var pressing = RowPress()
    private var hovering = false {
        didSet { Theme.reapply(self) }
    }

    init(_ ws: WorkspaceModel, number: Int, of count: Int, status: Design.Status, marker: NSAttributedString?,
         selected: Bool, dimmed: Bool, handler: @escaping () -> Void) {
        self.number = number
        self.status = status
        self.selected = selected
        self.handler = handler
        workspaceID = ws.id
        groupID = ws.machineID
        dragName = ws.name
        markerText = marker?.string ?? ""
        let branch = SidebarView.branch(ws.git)
        detailText = branch ?? ""
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.row + 1
        layer?.cornerCurve = .continuous
        alphaValue = dimmed ? Design.Alpha.dimmed : 1
        var tip = SidebarView.chord(number, of: count).map { "\(ws.name) (\($0))" } ?? ws.name
        if let branch { tip += "\n\(branch)" }
        toolTip = tip
        setAccessibilityRole(.button)
        setAccessibilityLabel("\(ws.name), \(status.rawValue)")

        let icon = StatusIconView(status) { SidebarView.icon(status, pointSize: Design.Symbol.sidebarRow) }
        let name = NSTextField(labelWithString: ws.name)
        name.font = Design.Font.sidebarRow
        name.textColor = Design.Color.textPrimary
        name.lineBreakMode = .byTruncatingTail
        name.setContentCompressionResistancePriority(.defaultLow + 1, for: .horizontal)
        name.setContentHuggingPriority(.defaultHigh, for: .horizontal)
        let detail = NSTextField(labelWithString: branch ?? "")
        detail.font = Design.Font.sidebarBranch
        detail.textColor = Design.Color.textMuted
        detail.alignment = .right
        detail.lineBreakMode = .byTruncatingTail
        detail.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        detail.setContentHuggingPriority(.defaultLow, for: .horizontal)

        for view in [icon, name, detail] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 16),
            name.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 8),
            name.centerYAnchor.constraint(equalTo: centerYAnchor),
            detail.leadingAnchor.constraint(greaterThanOrEqualTo: name.trailingAnchor, constant: 10),
            detail.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            detail.centerYAnchor.constraint(equalTo: centerYAnchor),
            // The branch never takes more than about half the row.
            detail.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor, multiplier: 0.48),
        ])
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect],
            owner: self))
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

    func applyTheme() {
        layer?.backgroundColor = selected
            ? Design.Color.selected.cgColor
            : hovering ? Theme.faded(Design.Color.selected, Design.Alpha.rowHover).cgColor : nil
    }

    /// Automation: what a click does.
    func press() { handler() }
}

/// "+" under a machine group: a new workspace there.
@MainActor
private final class AddRow: NSView {
    private let handler: () -> Void

    init(machine: String, handler: @escaping () -> Void) {
        self.handler = handler
        super.init(frame: .zero)
        let button = NSButton()
        button.image = NSImage(systemSymbolName: "plus", accessibilityDescription: "New workspace on \(machine)")?
            .withSymbolConfiguration(.init(pointSize: Design.Symbol.sidebarAdd, weight: .regular))
        button.isBordered = false
        button.contentTintColor = Design.Color.textMuted
        button.refusesFirstResponder = true
        button.toolTip = "New workspace on \(machine)"
        button.target = self
        button.action = #selector(clicked)
        button.translatesAutoresizingMaskIntoConstraints = false
        addSubview(button)
        NSLayoutConstraint.activate([
            button.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            button.centerYAnchor.constraint(equalTo: centerYAnchor),
            button.widthAnchor.constraint(equalToConstant: 16),
            button.heightAnchor.constraint(equalToConstant: 16),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc private func clicked() { handler() }
}

// MARK: - Agents | Files switch

/// The segmented switch under the traffic lights: Agents (list icon) and
/// Files (folder icon) on a dark track, the selected one lighter.
@MainActor
private final class SidebarSegments: NSView, ThemeAware {
    var onChange: ((SidebarView.Mode) -> Void)?
    var selected: SidebarView.Mode = .agents {
        didSet { update() }
    }
    private let agents = SegmentButton(title: "Agents", symbol: "list.bullet")
    private let files = SegmentButton(title: "Files", symbol: "folder")

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.sidebarSegment
        layer?.cornerCurve = .continuous
        Theme.track(self)
        agents.target = self
        agents.action = #selector(agentsClicked)
        files.target = self
        files.action = #selector(filesClicked)
        addSubview(agents)
        addSubview(files)
        update()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func layout() {
        super.layout()
        let inset: CGFloat = 2
        let half = (bounds.width - inset * 2) / 2
        agents.frame = NSRect(x: inset, y: inset, width: half, height: bounds.height - inset * 2)
        files.frame = NSRect(x: inset + half, y: inset, width: half, height: bounds.height - inset * 2)
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        layer?.backgroundColor = Design.Color.control.cgColor
    }

    private func update() {
        agents.isOn = selected == .agents
        files.isOn = selected == .files
    }

    @objc private func agentsClicked() { onChange?(.agents) }
    @objc private func filesClicked() { onChange?(.files) }
}

@MainActor
private final class SegmentButton: NSButton, ThemeAware {
    var isOn = false {
        didSet { restyle() }
    }
    private let label: String
    private let symbol: String

    init(title: String, symbol: String) {
        label = title
        self.symbol = symbol
        super.init(frame: .zero)
        isBordered = false
        imagePosition = .noImage
        alignment = .center
        refusesFirstResponder = true
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.sidebarSegment - 2
        layer?.cornerCurve = .continuous
        setAccessibilityLabel(title)
        Theme.track(self)
    }

    func applyTheme() { restyle() }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private func restyle() {
        let color = isOn ? Design.Color.textPrimary : Design.Color.textSecondary
        // The icon rides in the title, so icon and label centre together.
        let text = NSMutableAttributedString()
        if let image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: Design.Symbol.sidebarSegment, weight: .regular)
                .applying(.init(paletteColors: [Theme.resolved(color)]))) {
            let attachment = NSTextAttachment()
            attachment.image = image
            attachment.bounds = NSRect(x: 0, y: -1.5, width: image.size.width, height: image.size.height)
            text.append(NSAttributedString(attachment: attachment))
        }
        let style = NSMutableParagraphStyle()
        style.alignment = .center
        text.append(NSAttributedString(string: "  " + label, attributes: [
            .font: Design.Font.sidebarSegment, .foregroundColor: color,
        ]))
        text.addAttribute(.paragraphStyle, value: style, range: NSRange(location: 0, length: text.length))
        attributedTitle = text
        // theme: restyle() is this view's applyTheme().
        effectiveAppearance.performAsCurrentDrawingAppearance {
            layer?.backgroundColor = isOn ? Design.Color.selected.cgColor : nil
        }
    }
}

// MARK: - Footer

/// Counts of workspaces by state: working, blocked, idle, done, each
/// number in its state's colour; zero counts are faded.
@MainActor
private final class SidebarFooter: NSView {
    private static let order: [Design.Status] = [.working, .blocked, .idle, .done]
    private let stack = NSStackView()
    private(set) var summary = ""

    override init(frame: NSRect) {
        super.init(frame: frame)
        stack.orientation = .horizontal
        stack.spacing = 14
        stack.alignment = .centerY
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func apply(_ statuses: [Design.Status]) {
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        var parts: [String] = []
        for status in Self.order {
            let count = statuses.filter { $0 == status }.count
            parts.append("\(status.rawValue)=\(count)")
            let item = NSStackView()
            item.orientation = .horizontal
            item.spacing = 4
            item.alignment = .centerY
            let icon = StatusIconView(status) { SidebarView.icon(status, pointSize: Design.Symbol.footerStatus) }
            let text = NSTextField(labelWithString: "\(count)")
            text.font = Design.Font.footerCount
            // Tinted by state, as in the mockup (idle stays grey).
            text.textColor = status.color
            item.addArrangedSubview(icon)
            item.addArrangedSubview(text)
            item.alphaValue = count == 0 ? Design.Alpha.footerZero : 1
            item.toolTip = "\(count) \(status.rawValue)"
            stack.addArrangedSubview(item)
        }
        summary = parts.joined(separator: ",")
    }
}

// MARK: - Automation

/// `sidebar mode agents|files`, `sidebar select <n>` (the nth workspace
/// row, 1-based), `sidebar focus` (the Files tree takes the keyboard) and
/// `sidebar files <verb> [arg]` (the files tile's verbs on the Files view).
/// State: `sidebar.mode`, `sidebar.statuses`, `sidebar.footer`,
/// `sidebar.numbers` (each row's number and name: `1 alpha|2 bravo`), and
/// the Files view's `tile.*` keys as `sidebar.files.*`.
/// The rail: `sidebar rail select N|hover N|expand`; state `rail.shown`,
/// `rail.rows`, `rail.groups`, `rail.tips`, `rail.group.tips`, `rail.fits`,
/// `rail.spinning`, `rail.keyboard`, `rail.offsets` (how far each row's
/// content sits off its middle) and `rail.insets` (each row's left/right
/// inset in the rail), `rail.centred` (both within a point). Also
/// `workspace.active`: the active workspace's name.
/// Dragging rows to reorder, in whichever of the two shows:
/// `sidebar drag N|to N|out|drop` and the `drag.*` keys (`DragAutomation`).
@MainActor
enum SidebarAutomation {
    static func run(_ arg: String, controller: WindowController?) -> String? {
        guard let sidebar = controller?.sidebarView else { return "no window" }
        let parts = arg.split(separator: " ", maxSplits: 1).map(String.init)
        let rest = parts.count > 1 ? parts[1] : ""
        switch parts.first ?? "" {
        case "mode":
            guard let mode = SidebarView.Mode(rawValue: rest) else { return "mode is agents or files" }
            sidebar.setMode(mode)
        case "select":
            guard let n = Int(rest), sidebar.rows.indices.contains(n - 1) else { return "no row \(rest)" }
            sidebar.rows[n - 1].press()
        case "focus":
            guard let panel = sidebar.filesPanel else { return "the Files view isn't shown" }
            panel.focusTree()
        case "files":
            guard let panel = sidebar.filesPanel else { return "the Files view isn't shown" }
            let bits = rest.split(separator: " ", maxSplits: 1).map(String.init)
            return panel.automate(bits.first ?? "", bits.count > 1 ? bits[1] : "")
        case "rail":
            // `sidebar rail select N` (the Nth workspace row), `sidebar rail
            // expand` (»).
            guard let rail = controller?.railView, !rail.isHiddenOrHasHiddenAncestor else { return "the rail isn't shown" }
            let bits = rest.split(separator: " ").map(String.init)
            let n = bits.count > 1 ? Int(bits[1]) : nil
            switch bits.first ?? "" {
            case "select":
                guard let n, rail.workspaceRows.indices.contains(n - 1) else { return "no rail row \(rest)" }
                rail.workspaceRows[n - 1].press()
            case "expand":
                (rail.expandButton as? NSButton)?.performClick(nil)
            case "hover":
                // `sidebar rail hover N` (or 0 for none): the hover fill, for captures.
                for (i, row) in rail.workspaceRows.enumerated() { row.hover(i == (n ?? 0) - 1) }
            default:
                return "sidebar rail takes select N, hover N or expand"
            }
        case "drag":
            return DragAutomation.run(rest.split(separator: " ").map(String.init), host: dragHost(controller))
        default:
            return "unknown sidebar verb \(parts.first ?? "")"
        }
        return nil
    }

    /// The rows a drag works on: the rail when it shows, else the full
    /// sidebar's Agents list.
    private static func dragHost(_ controller: WindowController?) -> (NSView & WorkspaceDragHost)? {
        guard let controller else { return nil }
        if !controller.railView.isHiddenOrHasHiddenAncestor { return controller.railView }
        let sidebar = controller.sidebarView
        return sidebar.isHiddenOrHasHiddenAncestor || sidebar.mode != .agents ? nil : sidebar
    }

    static func state(_ controller: WindowController) -> [String: String] {
        let sidebar = controller.sidebarView
        var s = [
            "sidebar.mode": sidebar.mode.rawValue,
            "sidebar.statuses": sidebar.statuses.joined(separator: "|"),
            "sidebar.footer": sidebar.footerCounts,
            "sidebar.numbers": sidebar.numbers.joined(separator: "|"),
        ]
        // The rail: rows, groups, tooltips (lines joined by " ¦ ", rows by
        // " ‖ "), whether it all fits, spinning working rings, whether the
        // keyboard is in it (never), and how centred its rows are.
        let rail = controller.railView
        let tips = { (views: [NSView]) in
            views.map { ($0.toolTip ?? "").replacingOccurrences(of: "\n", with: " ¦ ") }.joined(separator: " ‖ ")
        }
        s["rail.shown"] = rail.isHiddenOrHasHiddenAncestor ? "no" : "yes"
        s["rail.rows"] = rail.rowSummary
        s["rail.groups"] = rail.groupSummary
        s["rail.tips"] = tips(rail.workspaceRows)
        s["rail.group.tips"] = tips(rail.groups)
        s["rail.fits"] = rail.fits ? "yes" : "no"
        s["rail.spinning"] = "\(rail.workspaceRows.filter(\.isSpinning).count)"
        s["rail.offsets"] = rail.contentOffsets
        s["rail.insets"] = rail.rowInsets
        s["rail.centred"] = rail.centred ? "yes" : "no"
        s["workspace.active"] = controller.model?.workspace?.name ?? "-"
        s.merge(DragAutomation.state(host: dragHost(controller))) { $1 }
        let responder = controller.window.firstResponder as? NSView
        s["rail.keyboard"] = responder.map { $0.isDescendant(of: rail) } == true ? "yes" : "no"
        if sidebar.mode == .files {
            s["sidebar.files.title"] = sidebar.filesTitleText
            s["sidebar.files.note"] = sidebar.filesNoteText.isEmpty ? "-" : sidebar.filesNoteText
        }
        if let panel = sidebar.filesPanel {
            for (key, value) in panel.automationState() {
                s["sidebar.files." + key.replacingOccurrences(of: "tile.", with: "")] = value
            }
            let responder = controller.window.firstResponder as? NSView
            s["sidebar.files.keyboard"] = responder.map { $0.isDescendant(of: panel) } == true ? "yes" : "no"
        }
        return s
    }
}
