import AppKit

/// What's inside the sidebar card: a Workspaces | Files switch, then either
/// the workspace list or the focused workspace's file tree, and a footer
/// counting workspaces by agent state, in words. The card itself, its top
/// band with the traffic lights and the sidebar toggle belong to the window
/// chrome (`SidebarCardView`); this view sits below that band and draws no
/// background of its own.
///
/// Workspaces: one group per herdr machine. Its header: a machine glyph,
/// the name in bold, a connection dot (or the state in words, folded to an
/// icon below `space.sidebarCompactBelow`), the workspace count and a "+"
/// that creates a workspace there (connected machines only). A hairline
/// parts the groups. Then the workspaces (`WorkspaceRow`): status icon, name
/// and time in state, the branch on its own line. Rows of a machine that
/// isn't connected are dimmed. The list scrolls on its own: its edges fade
/// where rows go on beyond them, and pills count the rows out of view and
/// the waiting ones among them (a click scrolls to the nearest waiting one).
/// A row shows no hover card (it says it all already; the rail's rows
/// have them); a row whose name or branch is cut short gets a plain tooltip
/// with them in full.
///
/// Files: a `files.panel` the core lists and watches (`FilesPanelView`), on
/// the focused workspace's repository (else its cwd), under a header with
/// the folder, its branch and how many files changed. Return opens a file
/// in the editor, in a new terminal beside the focused tile; Esc hands the
/// keyboard back to the tiles. A remote workspace shows its terminal's
/// folder on its machine, listed over SSH.
@MainActor
final class SidebarView: NSView, KeepsKeyboard, ThemeAware, WorkspaceDragHost {
    enum Mode: String {
        case workspaces, files

        /// A mode by name, "agents" (the Workspaces view's old name, in
        /// settings and scripts from before) too.
        init?(name: String) {
            if name == "agents" { self = .workspaces } else { self.init(rawValue: name) }
        }
    }

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
    private let document = FlippedView()
    private let stack = NSStackView()
    private let emptyHint = EmptyListHint()
    private let topPill = EdgePill(style: .pill)
    private let bottomPill = EdgePill(style: .pill)
    private let files = FlippedView()
    private let filesHeader = FilesHeader()
    private let filesNote = NSTextField(wrappingLabelWithString: "")
    private let footer = SidebarFooter()
    private let footerLine = NSView()
    private var shown: [String] = []
    private var shownActive: String?
    private var window_: WindowModel?
    private var machines: [MachineModel] = []
    /// A refresh held back while a drag runs.
    private var pending: (window: WindowModel, machines: [MachineModel])?
    /// The active row comes into view on the next layout.
    private var revealActive = false
    /// The rows out of view, as the pills last counted them.
    private(set) var overflow = Overflow()
    /// Refreshes the rows' time in state.
    private var clock: Timer?
    /// `new_workspace`'s chord (⌘T), for the empty list's hint.
    private var newWorkspaceChord: String?

    // The Files view's panel and what it shows.
    private var panelID: String?
    private var panelRoot: String?
    /// The herdr machine the panel's files are on; nil for Local.
    private var panelMachine: String?
    private var panelView: FilesPanelView?
    /// A `files.panel` in flight (the core checks the folder on disk, so
    /// the main thread doesn't wait for it): its root and machine, and a
    /// number that tells its reply from an older one's.
    private var panelOpening: (root: String, machine: String?, request: Int)?
    private var panelRequests = 0
    /// The tree takes the keyboard when the panel on its way arrives.
    private var focusPending = false
    /// Automation: the Files view's title (its folder, with the machine for
    /// a remote one).
    private(set) var filesTitleText = ""

    /// Each row's ⌘N key badge shows (⌘ held alone, `WorkspaceHints`).
    private(set) var hintsShown = false
    /// Automation: how long the last show or hide of the badges faded.
    private(set) var hintsFade: TimeInterval = 0

    override var isFlipped: Bool { true }
    /// A click on empty space does nothing (the card it sits in would move
    /// the window).
    override func mouseDown(with event: NSEvent) {}

    override init(frame: NSRect) {
        mode = Mode(name: SettingsStore.shared.string(forKey: Self.modeKey) ?? "") ?? .workspaces
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
        list.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(self, selector: #selector(listScrolled),
                                               name: NSView.boundsDidChangeNotification, object: list.contentView)
        emptyHint.isHidden = true
        addSubview(emptyHint)
        topPill.onPress = { [weak self] in self?.scrollToWaiting(up: true) }
        bottomPill.onPress = { [weak self] in self?.scrollToWaiting(up: false) }
        addSubview(topPill)
        addSubview(bottomPill)

        filesNote.font = Design.Font.groupLabel
        filesNote.textColor = Design.Color.textMuted
        files.addSubview(filesHeader)
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

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        clock?.invalidate()
        clock = nil
        guard window != nil else { return }
        // Time in state counts minutes: twice a minute is plenty.
        let timer = Timer(timeInterval: 30, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
        timer.tolerance = 5
        RunLoop.main.add(timer, forMode: .common)
        clock = timer
    }

    private func tick() {
        guard !isHiddenOrHasHiddenAncestor, mode == .workspaces else { return }
        for row in rows { row.refreshTime() }
    }

    /// The keymap's chords this view shows (`new_workspace`'s, in the empty
    /// list's hint).
    func apply(keymap: [String: Any]) {
        newWorkspaceChord = QuickOpenButton.chord(for: "new_workspace", in: keymap)
        emptyHint.chord = newWorkspaceChord
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
        emptyHint.frame = body
        files.frame = body
        let headerHeight = filesHeader.height
        filesHeader.frame = NSRect(x: 0, y: 4, width: body.width, height: headerHeight)
        let below = filesHeader.frame.maxY + 4
        filesNote.frame = NSRect(x: pad + 9, y: below + 4, width: body.width - pad * 2 - 18, height: 60)
        panelView?.frame = NSRect(x: 2, y: below, width: body.width - 4, height: max(0, body.height - below))
        if mode == .workspaces {
            list.layoutSubtreeIfNeeded()
            if revealActive {
                revealActive = false
                if let row = rows.first(where: \.selected) {
                    document.scrollToVisible(document.convert(row.bounds, from: row).insetBy(dx: 0, dy: -8))
                }
            }
        }
        updateOverflow()
    }

    @objc private func listScrolled() {
        updateOverflow()
    }

    /// The edge fades and the pills, for the rows out of view now.
    private func updateOverflow() {
        guard mode == .workspaces, !list.isHidden else {
            topPill.isHidden = true
            bottomPill.isHidden = true
            return
        }
        let visible = list.documentVisibleRect
        let rows = self.rows.map { (frame: document.convert($0.bounds, from: $0), waiting: $0.waiting) }
        overflow = Overflow(rows: rows, visible: visible, flipped: true)
        EdgeFade.apply(to: list, top: visible.minY > 0.5, bottom: visible.maxY < document.frame.height - 0.5)
        if overflow.aboveWaiting > 0 {
            topPill.set(arrow: "↑", plain: nil, waiting: "\(overflow.aboveWaiting) waiting above")
        }
        if overflow.below > 0 {
            bottomPill.set(arrow: "↓", plain: "\(overflow.below) more",
                           waiting: overflow.belowWaiting > 0 ? "\(overflow.belowWaiting) waiting" : nil)
        }
        topPill.isHidden = overflow.aboveWaiting == 0
        bottomPill.isHidden = overflow.below == 0
        for (pill, top) in [(topPill, true), (bottomPill, false)] where !pill.isHidden {
            let w = min(pill.fittingWidth, list.frame.width - 16)
            let y = top ? list.frame.minY + 6 : list.frame.maxY - 8 - pill.height
            pill.frame = NSRect(x: (list.frame.width - w) / 2, y: y, width: w, height: pill.height)
        }
    }

    /// The pills' click: the nearest waiting row that way into view, else
    /// (none waits) the next row out of view.
    private func scrollToWaiting(up: Bool) {
        let visible = list.documentVisibleRect
        let frames = rows.map { (frame: document.convert($0.bounds, from: $0), waiting: $0.waiting) }
        let beyond = frames.filter { up ? $0.frame.midY < visible.minY : $0.frame.midY > visible.maxY }
        let ordered = up ? beyond.reversed() : beyond
        guard let target = ordered.first(where: \.waiting) ?? ordered.first else { return }
        // Clear of the pill at that edge.
        let margin = Design.Space.sidebarPillHeight + 12
        document.scrollToVisible(target.frame.insetBy(dx: 0, dy: -margin))
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
        // Numbers run top to bottom across the groups: the core
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
        let infos = Dictionary(uniqueKeysWithValues: window.workspaces.map { ($0.id, WorkspaceInfo($0)) })
        let rollups = Dictionary(uniqueKeysWithValues: window.workspaces.map {
            ($0.id, AttentionStyle.Rollup($0.tabs.flatMap(\.tiles)))
        })
        let groups = Self.groups(window, machines: machines)
        var key = window.workspaces.map { ws in
            let r = rollups[ws.id]
            return "\(ws.machineID)|\(ws.id == window.activeWorkspace)|\(String(describing: infos[ws.id]))"
                + "|\(r?.blocked ?? 0)|\(r?.attention ?? 0)|\(r?.working ?? 0)"
        }
        key += groups.map { "\($0.id)|\($0.machine?.label ?? "")|\($0.machine?.state.rawValue ?? "")|\($0.machine?.message ?? "")" }
        guard key != shown else { return }
        shown = key
        if window.activeWorkspace != shownActive {
            shownActive = window.activeWorkspace
            revealActive = true
        }
        footer.apply(window.workspaces.map { infos[$0.id]?.status ?? .idle })
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        if let card = HoverCard.shown, card.anchor.window == nil { HoverCard.hide() }
        // No workspaces at all: a hint in place of the list.
        emptyHint.isHidden = !window.workspaces.isEmpty
        if window.workspaces.isEmpty { needsLayout = true; return }
        for (index, group) in groups.enumerated() {
            if index > 0 { add(GroupDivider(), height: Design.Space.sidebarGroupGap, inset: 0) }
            let header = MachineHeader(group: group)
            if group.machine?.state == .connected {
                header.onAdd = { [weak self] in self?.onAdd?(group.id) }
            }
            add(header, height: Design.Space.sidebarGroupHeight)
            let dimmed = group.machine.map { $0.state != .connected } ?? true
            for (number, ws) in group.workspaces {
                let info = infos[ws.id] ?? WorkspaceInfo(ws)
                let row = WorkspaceRow(
                    info, groupID: ws.machineID, number: number, of: window.workspaces.count,
                    marker: rollups[ws.id]?.marker, selected: ws.id == window.activeWorkspace, dimmed: dimmed
                ) { [weak self] in self?.onSelect?(ws.id) }
                row.menuProvider = { [weak self] in self?.onMenu?(ws.id) }
                row.setHint(hintsShown, duration: 0)
                add(row, height: row.height)
            }
            // A connected machine with nothing on it: one line and a link.
            if group.workspaces.isEmpty, group.machine?.state == .connected {
                add(EmptyMachineRow { [weak self] in self?.onAdd?(group.id) }, height: Design.Space.sidebarRowCompact)
            }
        }
        needsLayout = true
    }

    /// Shows or hides each row's ⌘N badge: a quick fade with layout motion
    /// on, at once with it off or Reduce Motion on.
    func setHints(_ on: Bool) {
        guard on != hintsShown else { return }
        hintsShown = on
        hintsFade = Motion.active ? Design.Motion.sidebarHint : 0
        for row in rows { row.setHint(on, duration: hintsFade) }
    }

    private func add(_ view: NSView, height: CGFloat, inset: CGFloat = Design.Space.sidebarPadding) {
        stack.addArrangedSubview(view)
        view.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            view.leadingAnchor.constraint(equalTo: stack.leadingAnchor, constant: inset),
            view.trailingAnchor.constraint(equalTo: stack.trailingAnchor, constant: -inset),
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

    // MARK: Workspaces | Files

    /// Kept across launches in `SettingsStore`, as the sidebar's width is.
    private static let modeKey = "sidebarMode"

    /// Switches the view; with `focus`, the Files tree takes the keyboard
    /// (once its panel is in, if it's still on its way). Leaving Files with
    /// the keyboard in it hands the keyboard back to the tiles.
    func setMode(_ mode: Mode, focus: Bool = false) {
        guard mode != self.mode else {
            if focus, mode == .files { focusTree() }
            return
        }
        let hadKeyboard = (window?.firstResponder as? NSView)?.isDescendant(of: self) == true
        self.mode = mode
        segments.selected = mode
        SettingsStore.shared.set(mode.rawValue, forKey: Self.modeKey)
        applyMode()
        if mode == .files, focus { focusTree() }
        if mode == .workspaces, hadKeyboard { onReturnFocus?() }
    }

    /// The Files tree takes the keyboard, now or when its panel arrives.
    private func focusTree() {
        if let panelView { panelView.focusTree() } else { focusPending = true }
    }

    private func applyMode() {
        list.isHidden = mode != .workspaces
        files.isHidden = mode != .files
        if mode == .files {
            emptyHint.isHidden = true
            syncPanel()
        } else {
            emptyHint.isHidden = window_.map { !$0.workspaces.isEmpty } ?? true
            closePanel()
            revealActive = true
        }
        needsLayout = true
    }

    /// The folder the Files view shows for the focused workspace (on its
    /// machine; nil for Local), or why it shows none.
    private func filesRoot() -> (root: String?, machine: String?, note: String?) {
        guard let window = window_, let ws = window.workspace else { return (nil, nil, "No workspace.") }
        let focused = ws.tab?.tile(ws.tab?.focused ?? "")
        if ws.machineID != MachineModel.local {
            // A remote workspace: its terminal's folder there.
            let machine = machines.first { $0.id == ws.machineID }
            let name = machine?.label ?? ws.machineID
            // While it reconnects the panel stays, with the last listing
            // and the core's error; off or gone from herdr, there's no link.
            guard let state = machine?.state, state != .off else {
                return (nil, nil, "\(name) isn't connected. \(ws.name)'s files show here once it is.")
            }
            let terminals = [focused].compactMap { $0 } + (ws.tab?.tiles ?? [])
            let candidates = terminals.flatMap { [$0.pane?.cwd, $0.cwd] } + [ws.cwd]
            guard let root = candidates.compactMap({ $0 }).first(where: { $0.hasPrefix("/") }) else {
                return (nil, nil, "Waiting for \(ws.name)'s folder on \(name).")
            }
            return (root, ws.machineID, nil)
        }
        let candidates = [ws.git?.repo, ws.cwd, focused?.cwd]
        if let root = candidates.compactMap({ $0 }).first(where: { !$0.isEmpty }) { return (root, nil, nil) }
        return (NSHomeDirectory(), nil, nil)
    }

    private func syncPanel() {
        let (root, machine, note) = filesRoot()
        filesNote.stringValue = note ?? ""
        filesNote.isHidden = note == nil
        guard let root else {
            closePanel()
            filesTitleText = ""
            filesHeader.isHidden = true
            needsLayout = true
            return
        }
        let label = machine.map { id in machines.first { $0.id == id }?.label ?? id }
        filesTitleText = label.map { "\($0): \(root)" } ?? Self.tilde(root)
        // The branch when the root is the workspace's repository, else where
        // the folder is.
        let ws = window_?.workspace
        let git = ws?.git.flatMap { $0.repo == root ? $0 : nil }
        filesHeader.isHidden = false
        filesHeader.show(
            name: (root as NSString).lastPathComponent.isEmpty ? root : (root as NSString).lastPathComponent,
            branch: git?.branch, detached: git?.detached == true,
            place: label.map { "\($0): \(Self.tilde(root))" } ?? Self.tilde(root),
            tip: label.map { "\(root) on \($0)" } ?? root,
            changed: panelView?.changedCount ?? 0)
        needsLayout = true
        if let opening = panelOpening {
            // On its way already; another root in flight is overtaken below.
            if opening.root == root, opening.machine == machine { return }
        } else if root == panelRoot, machine == panelMachine, panelView != nil {
            return
        }
        panelRequests += 1
        let request = panelRequests
        panelOpening = (root, machine, request)
        FilesPanelView.openPanel(root: root, machine: machine, reuse: panelID) { [weak self] id in
            guard let self, self.panelOpening?.request == request else {
                // Closed or re-rooted meanwhile: don't leave a stray panel
                // in the core.
                if let id, id != self?.panelID { FilesPanelView.closePanel(id) }
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
            view.setRoot(root)
        } else {
            panelView?.close()
            panelView?.removeFromSuperview()
            let view = FilesPanelView(panel: id, root: root, machine: machine)
            view.onEscape = { [weak self] in self?.onReturnFocus?() }
            view.onGitChange = { [weak self, weak view] in
                self?.filesHeader.setChanged(view?.changedCount ?? 0)
            }
            files.addSubview(view)
            panelView = view
        }
        filesHeader.setChanged(panelView?.changedCount ?? 0)
        panelID = id
        needsLayout = true
        if focusPending, let panelView {
            focusPending = false
            panelView.focusTree()
        }
    }

    /// Stops the Files view's panel (switching to Workspaces, closing the
    /// window).
    func closePanel() {
        panelOpening = nil
        focusPending = false
        if let panelID { FilesPanelView.closePanel(panelID) }
        panelView?.close()
        panelView?.removeFromSuperview()
        panelView = nil
        panelID = nil
        panelRoot = nil
        panelMachine = nil
    }

    /// A status icon on the window's colour (⌘K's rows). Filled symbols
    /// (blocked's `exclamationmark.circle.fill`) get a dark glyph on the
    /// status colour, as in the mockup, rather than one flat disc.
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

    var dragRows: [WorkspaceDragRow] { mode == .workspaces ? rows : [] }
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

    /// Automation: the footer as it reads ("2 waiting · 1 working").
    var footerWords: String { footer.words }

    /// Automation: the Files view's panel, when it's shown.
    var filesPanel: FilesPanelView? { mode == .files ? panelView : nil }

    /// Automation: the Files view's note ("" when none shows).
    var filesNoteText: String { filesNote.isHidden ? "" : filesNote.stringValue }

    /// Automation: the Files view's header ("sample-api · main · 3 changed").
    var filesHeaderText: String { filesHeader.isHidden ? "" : filesHeader.summary }

    /// Automation: the pills showing ("↑ 2 waiting above|↓ 9 more · 1 waiting", "-" for none).
    var pills: String {
        [topPill, bottomPill].map { $0.isHidden ? "-" : $0.text }.joined(separator: "|")
    }

    /// Automation: the list's edge fades ("top", "bottom", "both", "none").
    var fades: String {
        let visible = list.documentVisibleRect
        let top = visible.minY > 0.5, bottom = visible.maxY < document.frame.height - 0.5
        return top && bottom ? "both" : top ? "top" : bottom ? "bottom" : "none"
    }

    /// Automation: each row's status icon centre less its text block's
    /// centre, in points ("0|0|0.5").
    var iconOffsets: String {
        rows.map { String(format: "%g", ($0.iconOffset * 100).rounded() / 100) }.joined(separator: "|")
    }

    /// Automation: each row's height ("40|30|40").
    var heights: String { rows.map { "\(Int($0.height))" }.joined(separator: "|") }

    /// Automation: what each row shows, as laid out: the name, its time and
    /// count, and the second line ("sample-api 2m ×2 ¦ fix/u…retry •").
    var rowTexts: [String] { rows.map(\.shownText) }

    /// Automation: the empty list's hint, or "-".
    var emptyText: String { emptyHint.isHidden ? "-" : emptyHint.text }

    /// Automation: the active row lies in the visible part of the list.
    var activeVisible: Bool {
        guard let row = rows.first(where: \.selected) else { return false }
        return list.documentVisibleRect.contains(document.convert(row.bounds, from: row).insetBy(dx: 1, dy: 1))
    }

    /// Automation: scroll the list to its top, its bottom or row N.
    func scroll(_ to: String) -> String? {
        switch to {
        case "top": document.scroll(.zero)
        case "bottom": document.scroll(NSPoint(x: 0, y: max(0, document.frame.height - list.contentSize.height)))
        default:
            guard let n = Int(to), rows.indices.contains(n - 1) else { return "sidebar scroll takes top, bottom or N" }
            document.scrollToVisible(document.convert(rows[n - 1].bounds, from: rows[n - 1]))
        }
        updateOverflow()
        return nil
    }

    /// Automation: press a pill.
    func pressPill(top: Bool) -> String? {
        let pill = top ? topPill : bottomPill
        guard !pill.isHidden else { return "no \(top ? "top" : "bottom") pill" }
        pill.onPress?()
        return nil
    }

    /// The branch a row shows: its name, `@abc1234` when detached, and
    /// a dot when tracked files changed.
    static func branch(_ git: GitModel?) -> String? {
        guard let git, let branch = git.branch, !branch.isEmpty else { return nil }
        let name = git.detached == true ? "@\(branch)" : branch
        return git.dirty == true ? "\(name) •" : name
    }
}

/// A flipped plain view: the sidebar list's document grows downwards.
final class FlippedView: NSView {
    override var isFlipped: Bool { true }
}

// MARK: - Groups

/// A machine group's header: a machine glyph (a laptop for Local), its name
/// in bold and a connection dot; on the right the workspace count and "+"
/// while connected, else the state in its colour ("Reconnecting" with a
/// turning arc, "Can't connect", "Off"). Below `space.sidebarCompactBelow`
/// the state's words fold to their icons ("Off" stays a word).
@MainActor
final class MachineHeader: NSView, ThemeAware {
    /// Automation: what the header says ("dev-server ● Reconnecting…").
    let summary: String
    /// "+": a new workspace there (connected machines only).
    var onAdd: (() -> Void)? {
        didSet { plus.isHidden = onAdd == nil }
    }

    private let local: Bool
    private let glyph = NSImageView()
    private let name: NSTextField
    private let dot: DotView?
    private let count: NSTextField?
    private let stateIcon: NSView?
    private let stateWord: NSTextField?
    /// The state's word stays when the header folds (Off, Not in herdr).
    private let wordStays: Bool
    private let plus = NSButton()

    override var isFlipped: Bool { true }

    init(group: SidebarView.Group) {
        let machine = group.machine
        local = group.id == MachineModel.local
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
        name = SidebarText.label(title, font: Design.Font.sidebarGroup, color: Design.Color.textSecondary)
        let connected = machine?.state == .connected
        dot = connected ? DotView(color: Design.Color.statusConnected) : nil
        count = connected ? SidebarText.label("\(group.workspaces.count)", font: Design.Font.groupLabel,
                                              color: Design.Color.textMuted) : nil
        let words: String? = switch machine?.state {
        case .connecting: "Connecting"
        case .reconnecting: "Reconnecting"
        case .error: "Can’t connect"
        case .off: "Off"
        case nil: "Not in herdr"
        case .connected: nil
        }
        let stays = machine?.state == .off || machine == nil
        wordStays = stays
        stateWord = words.map {
            SidebarText.label($0, font: Design.Font.sidebarMachineState,
                              color: stays ? Design.Color.textMuted : status.color)
        }
        switch machine?.state {
        case .connecting?, .reconnecting?:
            // The app's own arc, not the system spinner, which can't take a
            // colour. It holds still with motion off or Reduce Motion on.
            stateIcon = StatusIconView(status, image: {
                Design.customSymbol("azulejo.arc", color: status.color, pointSize: Design.Symbol.machineRing)
            }, spins: true)
        case .error?:
            stateIcon = StatusIconView(status, image: {
                SidebarGlyph.status(.error, pointSize: Design.Symbol.machineRing)
            }, spins: false)
        default:
            stateIcon = nil
        }
        super.init(frame: .zero)
        toolTip = machine?.message
        alphaValue = wordStays ? 0.7 : 1
        plus.image = NSImage(systemSymbolName: "plus", accessibilityDescription: "New workspace on \(title)")?
            .withSymbolConfiguration(.init(pointSize: Design.Symbol.sidebarAdd, weight: .medium))
        plus.isBordered = false
        plus.imagePosition = .imageOnly
        plus.contentTintColor = Design.Color.textMuted
        plus.refusesFirstResponder = true
        plus.toolTip = "New workspace on \(title)"
        plus.target = self
        plus.action = #selector(added)
        plus.isHidden = true
        for view in [glyph, name, dot, count, stateIcon, stateWord, plus].compactMap({ $0 }) { addSubview(view) }
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        setAccessibilityLabel(summary)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        glyph.image = SidebarGlyph.machine(local: local, color: Design.Color.textSecondary)
    }

    /// Narrower than `space.sidebarCompactBelow` (the card, that is).
    var compact: Bool { bounds.width + Design.Space.sidebarPadding * 2 < Design.Space.sidebarCompactBelow }

    override func layout() {
        super.layout()
        let mid = bounds.height / 2
        var x: CGFloat = 8
        glyph.frame = NSRect(x: x, y: mid - 7, width: 14, height: 14)
        x += 14 + 6
        var right = bounds.width - 8
        if !plus.isHidden {
            plus.frame = NSRect(x: right - 14, y: mid - 7, width: 14, height: 14)
            right -= 14 + 6
        }
        if let count {
            let w = SidebarText.width(count.stringValue, font: Design.Font.groupLabel) + 4
            count.frame = NSRect(x: right - w + 2, y: mid - 7, width: w, height: 14)
            right -= w - 4 + 6
        }
        if let stateWord {
            let show = !compact || wordStays
            stateWord.isHidden = !show
            if show {
                let w = min(SidebarText.width(stateWord.stringValue, font: Design.Font.sidebarMachineState) + 4,
                            max(0, right - x - 40))
                stateWord.frame = NSRect(x: right - w + 2, y: mid - 7, width: w, height: 14)
                right -= w - 4 + 4
            }
        }
        if let stateIcon {
            let side = Design.Symbol.machineRing + 3
            stateIcon.frame = NSRect(x: right - side, y: mid - side / 2, width: side, height: side)
            right -= side + 6
        }
        let dotRoom: CGFloat = dot == nil ? 0 : 12
        let nameWidth = min(SidebarText.width(name.stringValue, font: Design.Font.sidebarGroup) + 4,
                            max(0, right - x - dotRoom + 2))
        name.frame = NSRect(x: x - 2, y: mid - 7.5, width: nameWidth, height: 15)
        if let dot {
            let side = Design.Radius.machineDot * 2
            dot.frame = NSRect(x: name.frame.maxX - 2 + 6, y: mid - side / 2, width: side, height: side)
        }
    }

    @objc private func added() { onAdd?() }
}

/// The hairline between two machine groups, in the middle of its room
/// (`space.sidebarGroupGap`), across the card.
@MainActor
final class GroupDivider: NSView {
    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        Design.Color.border.setFill()
        NSRect(x: 0, y: floor((bounds.height - 1) / 2), width: bounds.width, height: 1).fill()
    }
}

/// A connected machine with no workspaces: "No workspaces" and a New… link.
@MainActor
private final class EmptyMachineRow: NSView {
    private let handler: () -> Void

    init(handler: @escaping () -> Void) {
        self.handler = handler
        super.init(frame: .zero)
        let text = SidebarText.label("No workspaces", font: Design.Font.detail, color: Design.Color.textMuted)
        let link = NSButton(title: "", target: nil, action: nil)
        link.isBordered = false
        link.attributedTitle = NSAttributedString(string: "New…", attributes: [
            .font: Design.Font.sidebarMachineState, .foregroundColor: Design.Color.accentText,
        ])
        link.refusesFirstResponder = true
        link.target = self
        link.action = #selector(clicked)
        link.setAccessibilityLabel("New workspace")
        let row = NSStackView(views: [text, link])
        row.orientation = .horizontal
        row.spacing = 6
        row.alignment = .firstBaseline
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 30),
            row.centerYAnchor.constraint(equalTo: centerYAnchor),
            row.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -8),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc private func clicked() { handler() }
}

/// No workspaces at all: a centred hint in place of the list.
@MainActor
private final class EmptyListHint: NSView, ThemeAware {
    private let icon = NSImageView()
    private let title = SidebarText.label("No workspaces yet", font: Design.Font.sidebarEmptyTitle,
                                          color: Design.Color.textSecondary)
    private let line = NSStackView()
    var chord: String? {
        didSet { rebuild() }
    }

    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        title.alignment = .center
        line.orientation = .horizontal
        line.spacing = 4
        line.alignment = .centerY
        for view in [icon, title, line] { addSubview(view) }
        rebuild()
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    var text: String { "\(title.stringValue) ¦ \(chord.map { "\($0) starts one" } ?? "New Workspace starts one")" }

    private func rebuild() {
        line.arrangedSubviews.forEach { $0.removeFromSuperview() }
        if let chord { line.addArrangedSubview(KeyChip(chord)) }
        line.addArrangedSubview(SidebarText.label(chord == nil ? "New Workspace starts one" : "starts one",
                                                  font: Design.Font.detail, color: Design.Color.textMuted))
        needsLayout = true
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        icon.image = NSImage(systemSymbolName: Design.Icon.workspace, accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: 15, weight: .regular)
                .applying(.init(paletteColors: [Theme.resolved(Design.Color.textMuted)])))
    }

    override func layout() {
        super.layout()
        let top = max(8, bounds.height / 2 - 52)
        icon.frame = NSRect(x: (bounds.width - 20) / 2, y: top, width: 20, height: 18)
        title.frame = NSRect(x: 8, y: icon.frame.maxY + 6, width: bounds.width - 16, height: 17)
        let size = line.fittingSize
        line.frame = NSRect(x: (bounds.width - size.width) / 2, y: title.frame.maxY + 6, width: size.width,
                            height: size.height)
    }
}

// MARK: - Rows

/// A workspace: status icon, name and time in state, then the branch on its
/// own line in mono (cut in the middle, keeping a `feature/` prefix), a dot
/// when the work tree is dirty; a detached HEAD shows its short hash and
/// "detached", a workspace herdr hasn't made yet "Starting…". Without a
/// branch the row is one line (`space.sidebarRowCompact`). The status icon
/// sits on the middle of the text block (both lines, or the one). Several
/// agents in the winning state show their count on a chip. A click focuses
/// it; a drag reorders it within its group (`WorkspaceDrag`). No hover card;
/// a plain tooltip with the full name and branch when either is cut short.
@MainActor
final class WorkspaceRow: NSView, ContextMenuProviding, ThemeAware, WorkspaceDragRow {
    /// Its number in the sidebar (⌘1…⌘8), top to bottom across groups.
    let number: Int
    let info: WorkspaceInfo
    var status: Design.Status { info.status }
    let workspaceID: String
    let groupID: String
    let dragName: String
    let selected: Bool
    /// Right-click: Rename, New Tab, Close and the folder's items.
    var menuProvider: (() -> NSMenu?)?
    /// Automation: the attention marker (" ● 2"), or "".
    let markerText: String
    /// Automation: the branch, or "".
    let detailText: String
    /// The chord that reaches it (⌘1…⌘8, ⌘9 for the last), shown in a key
    /// badge while ⌘ is held alone (`WorkspaceHints`); nil past ⌘8 but the
    /// last.
    let hintText: String?
    /// Two lines with a branch (or Starting…), one without.
    let height: CGFloat
    /// It waits for the user (the pills count these).
    var waiting: Bool { info.status == .blocked }
    /// The key badge is showing (or fading in).
    private(set) var hintShown = false
    private let icon: StatusIconView
    private let name: NSTextField
    private let time: NSTextField
    private let chip: CountChip?
    private let glyph: NSImageView?
    private let detail: NSTextField?
    private let detachedWord: NSTextField?
    private let dirtyDot: DotView?
    private let hint = HintBadge()
    private let handler: () -> Void
    private var pressing = RowPress()
    private var hovering = false {
        didSet { Theme.reapply(self) }
    }

    override var isFlipped: Bool { true }

    init(_ info: WorkspaceInfo, groupID: String, number: Int, of count: Int, marker: NSAttributedString?,
         selected: Bool, dimmed: Bool, handler: @escaping () -> Void) {
        self.info = info
        self.number = number
        self.selected = selected
        self.handler = handler
        self.groupID = groupID
        workspaceID = info.id
        dragName = info.name
        markerText = marker?.string ?? ""
        detailText = info.branch.map { name in
            (info.detached ? "@\(name)" : name) + (info.dirty ? " •" : "")
        } ?? ""
        hintText = SidebarView.chord(number, of: count)
        let shown = info.shown
        let twoLines = info.branch != nil || shown == .starting
        height = twoLines ? Design.Space.sidebarRowHeight : Design.Space.sidebarRowCompact
        icon = StatusIconView(shown) { SidebarGlyph.status(shown, pointSize: Design.Symbol.sidebarRow) }
        name = SidebarText.label(info.name, font: selected ? Design.Font.sidebarRowSelected : Design.Font.sidebarRow,
                                 color: Design.Color.textPrimary)
        time = SidebarText.label("", font: Design.Font.sidebarTime, color: info.timeColor)
        time.alignment = .right
        chip = info.count > 1 ? CountChip(count: info.count, status: info.status) : nil
        if shown == .starting {
            glyph = nil
            detail = SidebarText.label("Starting…", font: Design.Font.detail, color: Design.Color.textMuted)
            detachedWord = nil
            dirtyDot = nil
        } else if info.branch != nil {
            glyph = NSImageView()
            detail = SidebarText.label("", font: Design.Font.sidebarBranch, color: Design.Color.textMuted)
            detail?.lineBreakMode = .byClipping
            detachedWord = info.detached
                ? SidebarText.label("detached", font: Design.Font.detail, color: Design.Color.textMuted) : nil
            dirtyDot = info.dirty ? DotView(color: Design.Color.gitModified, side: 5) : nil
        } else {
            glyph = nil
            detail = nil
            detachedWord = nil
            dirtyDot = nil
        }
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.row + 1
        layer?.cornerCurve = .continuous
        alphaValue = dimmed ? Design.Alpha.dimmed : shown == .starting ? 0.75 : 1
        setAccessibilityRole(.button)
        setAccessibilityLabel("\(info.name), \(shown == .blocked ? "waiting" : shown.rawValue)")
        for view in [icon, name, time, chip, glyph, detail, detachedWord, dirtyDot].compactMap({ $0 }) {
            addSubview(view)
        }
        if let hintText {
            // At the row's end, over the time, which fades out meanwhile.
            hint.text = hintText
            hint.alphaValue = 0
            hint.isHidden = true
            addSubview(hint)
        }
        refreshTime()
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// The time in state again (the sidebar's clock, twice a minute).
    func refreshTime() {
        let text = info.sinceMs.map { SidebarText.age(sinceMs: $0) } ?? ""
        guard text != time.stringValue else { return }
        time.stringValue = text
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let twoLines = height > Design.Space.sidebarRowCompact
        let lineHeight: CGFloat = 17
        let top: CGFloat = twoLines ? 5 : (bounds.height - lineHeight) / 2
        // Once the text is placed: the icon on its middle.
        defer { placeIcon() }
        let textX: CGFloat = 32
        let right = bounds.width - 8
        // A label draws its text 2 points in from each side of its frame.
        let nameHeight = ceil(name.cell?.cellSize.height ?? 16)
        let nameY = top + (lineHeight - nameHeight) / 2
        var edge = right
        let nameFont = name.font ?? Design.Font.sidebarRow
        let timeFont = Design.Font.sidebarTime
        // The time sits on the name's baseline.
        let timeY = nameY + nameFont.ascender - timeFont.ascender
        if !time.stringValue.isEmpty {
            let w = SidebarText.width(time.stringValue, font: timeFont) + 4
            time.frame = NSRect(x: edge - w + 2, y: timeY, width: w, height: ceil(timeFont.ascender - timeFont.descender) + 2)
            edge -= w - 4 + 6
        } else {
            time.frame = .zero
        }
        if let chip {
            let w = chip.width
            let side = Design.Space.sidebarCountChip
            chip.frame = NSRect(x: edge - w, y: top + (lineHeight - side) / 2, width: w, height: side)
            edge -= w + 6
        }
        if hintText != nil {
            let size = hint.intrinsicContentSize
            hint.frame = NSRect(x: right - size.width + 2, y: top + (lineHeight - size.height) / 2,
                                width: size.width, height: size.height)
        }
        name.frame = NSRect(x: textX - 2, y: nameY, width: max(0, edge - textX + 2), height: nameHeight)
        // A label draws its text 2 points in from each side of its frame.
        let nameCut = SidebarText.width(info.name, font: nameFont) > name.frame.width - 4 + 0.5
        defer { updateTip(nameCut: nameCut) }
        guard twoLines, let detail else { return }
        let lineTop = top + lineHeight + 2
        var x = textX
        if let glyph {
            glyph.frame = NSRect(x: x, y: lineTop + 2, width: 10, height: 10)
            x += 14
        }
        let font = detail.font ?? Design.Font.sidebarBranch
        let detailHeight = ceil(detail.cell?.cellSize.height ?? 14)
        let detachedWidth = detachedWord.map { SidebarText.width($0.stringValue, font: Design.Font.detail) + 4 } ?? 0
        let dirtyWidth: CGFloat = dirtyDot == nil ? 0 : 5 + 4
        let room = max(0, right - x - detachedWidth - dirtyWidth)
        if let branch = info.branch, info.shown != .starting {
            detail.stringValue = SidebarText.middle(branch, font: font, width: room)
        }
        let w = min(room, SidebarText.width(detail.stringValue, font: font)) + 4
        detail.frame = NSRect(x: x - 2, y: lineTop + (14 - detailHeight) / 2, width: w, height: detailHeight)
        x += w - 4
        if let detachedWord {
            let wordHeight = ceil(detachedWord.cell?.cellSize.height ?? 14)
            detachedWord.frame = NSRect(x: x + 4 - 2, y: lineTop + (14 - wordHeight) / 2, width: detachedWidth,
                                        height: wordHeight)
            x += detachedWidth
        }
        dirtyDot?.frame = NSRect(x: x + 4, y: lineTop + 4.5, width: 5, height: 5)
    }

    /// The text block as laid out: the name's frame, joined by the second
    /// line's (branch or Starting…, and "detached") on a two-line row.
    private var textBlock: NSRect {
        [detail, detachedWord].compactMap { $0 }.filter { !$0.frame.isEmpty }
            .reduce(name.frame) { $0.union($1.frame) }
    }

    /// The status icon centred on the text block, on whole device pixels.
    private func placeIcon() {
        let side: CGFloat = 16
        let frame = NSRect(x: 8, y: textBlock.midY - side / 2, width: side, height: side)
        icon.frame = window == nil ? frame : backingAlignedRect(frame, options: .alignAllEdgesNearest)
    }

    /// Automation: the status icon's centre less the text block's, in
    /// points (0 when it sits on its middle).
    var iconOffset: CGFloat { icon.frame.midY - textBlock.midY }

    /// A plain tooltip with the full name and branch when either is cut
    /// short, else none.
    private func updateTip(nameCut: Bool) {
        let branchCut = info.shown != .starting && info.branch.map { $0 != detail?.stringValue } == true
        toolTip = nameCut || branchCut ? [info.name, info.branch].compactMap { $0 }.joined(separator: "\n") : nil
    }

    /// Shows or hides the key badge, cross-fading it with the time and the
    /// count over `duration` (0: at once). A row with no chord keeps them.
    func setHint(_ on: Bool, duration: TimeInterval) {
        guard hintText != nil, on != hintShown else { return }
        hintShown = on
        if on { hint.isHidden = false }
        let others: [NSView] = [time, chip].compactMap { $0 }
        guard duration > 0 else {
            hint.alphaValue = on ? 1 : 0
            others.forEach { $0.alphaValue = on ? 0 : 1 }
            return settleHint()
        }
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = duration
            ctx.timingFunction = CAMediaTimingFunction(name: .easeOut)
            hint.animator().alphaValue = on ? 1 : 0
            others.forEach { $0.animator().alphaValue = on ? 0 : 1 }
        }, completionHandler: { [weak self] in MainActor.assumeIsolated { self?.settleHint() } })
    }

    /// A faded-out badge leaves the view tree's drawing.
    private func settleHint() {
        if !hintShown { hint.isHidden = true }
    }

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
        glyph?.image = info.detached ? SidebarGlyph.commit(Design.Color.textMuted)
            : SidebarGlyph.branch(Design.Color.textMuted)
    }

    /// Automation: what a click does.
    func press() { handler() }

    /// Automation: the pointer over it, or not (its hover fill).
    func hover(_ on: Bool) { hovering = on }

    /// Automation: the row as laid out ("sample-api 2m ×2 ¦ fix/u…retry •").
    var shownText: String {
        var first = [info.name]
        if !time.stringValue.isEmpty { first.append(time.stringValue) }
        if info.count > 1 { first.append("×\(info.count)") }
        guard let detail else { return first.joined(separator: " ") }
        var second = [detail.stringValue]
        if detachedWord != nil { second.append("detached") }
        if dirtyDot != nil { second.append("•") }
        return first.joined(separator: " ") + " ¦ " + second.joined(separator: " ")
    }
}

@MainActor
extension WorkspaceInfo {
    /// What a rail row's hover card says: name and chord, the branch (and
    /// whether it has uncommitted changes), the state with its time and
    /// agent, the path.
    func cardLines(chord: String?) -> [HoverLine] {
        var lines: [HoverLine] = [.title(name, key: chord)]
        if let branch { lines.append(.branch(branch, detached: detached, dirty: dirty)) }
        // "Waiting 11m"; a state under a minute old has no time.
        let age = sinceMs.map { SidebarText.age(sinceMs: $0) }.flatMap { $0 == "now" ? nil : $0 }
        let words = [stateWord, age].compactMap { $0 }.joined(separator: " ")
        let trailing = count > 1 ? "\(count) agents" : agent
        let color = shown == .idle || shown == .starting ? Design.Color.textSecondary : shown.color
        lines.append(.state(shown, words, color: color, trailing: trailing))
        if let path { lines.append(.path(path)) }
        return lines
    }
}

/// How many agents share a row's state: a small chip in the state's colour.
@MainActor
private final class CountChip: NSView, ThemeAware {
    private let label: NSTextField
    private let status: Design.Status

    init(count: Int, status: Design.Status) {
        self.status = status
        label = SidebarText.label("\(count)", font: Design.Font.sidebarCount, color: status.color)
        label.alignment = .center
        super.init(frame: .zero)
        wantsLayer = true
        addSubview(label)
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    var width: CGFloat {
        max(Design.Space.sidebarCountChip, SidebarText.width(label.stringValue, font: Design.Font.sidebarCount) + 8)
    }

    override func layout() {
        super.layout()
        layer?.cornerRadius = bounds.height / 2
        let h = ceil(label.cell?.cellSize.height ?? 12)
        label.frame = NSRect(x: 0, y: (bounds.height - h) / 2, width: bounds.width, height: h)
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        layer?.backgroundColor = Theme.faded(status.color, 0.16).cgColor
    }
}

// MARK: - Files header

/// The Files view's header: the folder (its icon and name) with how many
/// files changed on the right, then its branch in mono (or where the folder
/// is), over a hairline.
@MainActor
private final class FilesHeader: NSView, ThemeAware {
    private let icon = NSImageView()
    private let name = SidebarText.label(font: Design.Font.sidebarRowSelected, color: Design.Color.textPrimary)
    private let changed = SidebarText.label(font: Design.Font.detail, color: Design.Color.gitModified)
    private let glyph = NSImageView()
    private let sub = SidebarText.label(font: Design.Font.sidebarBranch, color: Design.Color.textMuted)
    private var detached = false
    private var hasBranch = false
    private(set) var summary = ""

    override var isFlipped: Bool { true }

    /// Its height: 4 above, the name's line, 3 between, the branch's line, 10 below and the hairline.
    var height: CGFloat { 4 + 18 + 3 + 14 + 10 + 1 }

    override init(frame: NSRect) {
        super.init(frame: frame)
        changed.alignment = .right
        sub.lineBreakMode = .byTruncatingMiddle
        icon.imageScaling = .scaleProportionallyUpOrDown
        for view in [icon, name, changed, glyph, sub] { addSubview(view) }
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func show(name: String, branch: String?, detached: Bool, place: String, tip: String, changed count: Int) {
        self.name.stringValue = name
        hasBranch = branch != nil
        self.detached = detached
        sub.stringValue = branch ?? place
        sub.font = branch == nil ? Design.Font.detail : Design.Font.sidebarBranch
        toolTip = tip
        applyTheme()
        setChanged(count)
    }

    func setChanged(_ count: Int) {
        changed.stringValue = count > 0 ? "\(count) changed" : ""
        summary = [name.stringValue, sub.stringValue, changed.stringValue].filter { !$0.isEmpty }
            .joined(separator: " · ")
        needsLayout = true
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        icon.image = NSWorkspace.shared.icon(for: .folder)
        glyph.image = hasBranch
            ? (detached ? SidebarGlyph.commit(Design.Color.textMuted) : SidebarGlyph.branch(Design.Color.textMuted))
            : nil
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        Design.Color.border.setFill()
        NSRect(x: 0, y: bounds.height - 1, width: bounds.width, height: 1).fill()
    }

    override func layout() {
        super.layout()
        let left: CGFloat = 16, right = bounds.width - 16
        let top: CGFloat = 4
        icon.frame = NSRect(x: left, y: top + 1, width: 16, height: 16)
        var edge = right
        if !changed.stringValue.isEmpty {
            let w = SidebarText.width(changed.stringValue, font: Design.Font.detail) + 4
            changed.frame = NSRect(x: edge - w + 2, y: top + 2, width: w, height: 14)
            edge -= w - 4 + 8
        } else {
            changed.frame = .zero
        }
        let nameHeight = ceil(name.cell?.cellSize.height ?? 16)
        name.frame = NSRect(x: left + 22 - 2, y: top + (18 - nameHeight) / 2, width: max(0, edge - left - 22 + 2),
                            height: nameHeight)
        let lineTop = top + 18 + 3
        var x = left
        if glyph.image != nil {
            glyph.frame = NSRect(x: x, y: lineTop + 2, width: 10, height: 10)
            x += 14
        } else {
            glyph.frame = .zero
        }
        let subHeight = ceil(sub.cell?.cellSize.height ?? 14)
        sub.frame = NSRect(x: x - 2, y: lineTop + (14 - subHeight) / 2, width: max(0, right - x + 4), height: subHeight)
    }
}

// MARK: - Workspaces | Files switch

/// The segmented switch under the traffic lights: Workspaces (list icon)
/// and Files (folder icon) on a dark track, the selected one lighter.
@MainActor
private final class SidebarSegments: NSView, ThemeAware {
    var onChange: ((SidebarView.Mode) -> Void)?
    var selected: SidebarView.Mode = .workspaces {
        didSet { update() }
    }
    private let workspaces = SegmentButton(title: "Workspaces", symbol: "list.bullet")
    private let files = SegmentButton(title: "Files", symbol: "folder")

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.sidebarSegment
        layer?.cornerCurve = .continuous
        Theme.track(self)
        workspaces.target = self
        workspaces.action = #selector(workspacesClicked)
        workspaces.toolTip = "Workspaces"
        files.target = self
        files.action = #selector(filesClicked)
        files.toolTip = "Files"
        addSubview(workspaces)
        addSubview(files)
        update()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func layout() {
        super.layout()
        let inset: CGFloat = 2
        let half = (bounds.width - inset * 2) / 2
        workspaces.frame = NSRect(x: inset, y: inset, width: half, height: bounds.height - inset * 2)
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
        workspaces.isOn = selected == .workspaces
        files.isOn = selected == .files
    }

    @objc private func workspacesClicked() { onChange?(.workspaces) }
    @objc private func filesClicked() { onChange?(.files) }
}

@MainActor
private final class SegmentButton: NSButton, ThemeAware {
    var isOn = false {
        didSet { restyle() }
    }
    private let label: String
    private let symbol: String
    /// The icon fits beside the label at this width (a narrow sidebar
    /// drops it).
    private var iconFits = true

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

    override func layout() {
        super.layout()
        let needed = SidebarText.width(label, font: Design.Font.sidebarSegment) + Design.Symbol.sidebarSegment + 8 + 12
        let fits = bounds.width >= needed
        if fits != iconFits {
            iconFits = fits
            restyle()
        }
    }

    private func restyle() {
        let color = isOn ? Design.Color.textPrimary : Design.Color.textSecondary
        // The icon rides in the title, so icon and label centre together.
        let text = NSMutableAttributedString()
        if iconFits, let image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: Design.Symbol.sidebarSegment, weight: .regular)
                .applying(.init(paletteColors: [Theme.resolved(color)]))) {
            let attachment = NSTextAttachment()
            attachment.image = image
            attachment.bounds = NSRect(x: 0, y: -1.5, width: image.size.width, height: image.size.height)
            text.append(NSAttributedString(attachment: attachment))
            text.append(NSAttributedString(string: "  "))
        }
        let style = NSMutableParagraphStyle()
        style.alignment = .center
        style.lineBreakMode = .byTruncatingTail
        text.append(NSAttributedString(string: label, attributes: [
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

/// Workspaces by state, in words: "3 waiting  1 working  1 done  4 idle",
/// each number in its state's colour (idle's in the secondary text colour),
/// zero counts left out.
@MainActor
private final class SidebarFooter: NSView {
    private static let order: [(Design.Status, String)] = [
        (.blocked, "waiting"), (.working, "working"), (.done, "done"), (.idle, "idle"),
    ]
    private let label = NSTextField(labelWithString: "")
    /// Automation: every count, zeros too ("working=2,blocked=1,idle=3,done=0").
    private(set) var summary = ""
    /// Automation: what it reads ("1 waiting · 2 working").
    private(set) var words = ""

    override init(frame: NSRect) {
        super.init(frame: frame)
        label.lineBreakMode = .byTruncatingTail
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            label.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -4),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func apply(_ statuses: [Design.Status]) {
        let counts = Dictionary(grouping: statuses, by: { $0 }).mapValues(\.count)
        summary = [Design.Status.working, .blocked, .idle, .done]
            .map { "\($0.rawValue)=\(counts[$0] ?? 0)" }.joined(separator: ",")
        let text = NSMutableAttributedString()
        var parts: [String] = []
        for (status, word) in Self.order {
            let n = counts[status] ?? 0
            guard n > 0 else { continue }
            if text.length > 0 {
                text.append(NSAttributedString(string: "   ", attributes: [.font: Design.Font.footerLabel]))
            }
            text.append(NSAttributedString(string: "\(n)", attributes: [
                .font: Design.Font.footerCount,
                .foregroundColor: status == .idle ? Design.Color.textSecondary : status.color,
            ]))
            text.append(NSAttributedString(string: " " + word, attributes: [
                .font: Design.Font.footerLabel, .foregroundColor: Design.Color.textSecondary,
            ]))
            parts.append("\(n) \(word)")
        }
        label.attributedStringValue = text
        words = parts.joined(separator: " · ")
        toolTip = words
    }
}

// MARK: - Automation

/// `sidebar mode workspaces|files` ("agents" too, the old name), `sidebar
/// select <n>` (the nth workspace row, 1-based), `sidebar focus` (the Files
/// tree takes the keyboard) and `sidebar files <verb> [arg]`
/// (`FilesPanelView.automate`). `sidebar hover N` gives row N its hover fill
/// (0: none; a full sidebar row has no card); `sidebar scroll top|bottom|N` scrolls the list;
/// `sidebar pill top|bottom` presses an overflow pill.
/// State: `sidebar.mode`, `sidebar.statuses`, `sidebar.footer`,
/// `sidebar.footer.words`, `sidebar.numbers` (each row's number and name:
/// `1 alpha|2 bravo`), `sidebar.rows` (each row as laid out),
/// `sidebar.heights`, `sidebar.icons` (each status icon's centre less its
/// text block's, in points: `0|0|0.5`), `sidebar.icons.centred` (all
/// within half a point), `sidebar.overflow` (rows above and below the visible
/// part and the waiting ones among them: `0/0 9/1`), `sidebar.pills`,
/// `sidebar.fades`, `sidebar.active.visible`, `sidebar.empty`,
/// `sidebar.card` (the hover card showing, a rail row's or letter's, in
/// full), `sidebar.card.drawn` (its lines as drawn, cut to fit),
/// `sidebar.card.lines` (text lines each takes: `1|1|1|1`),
/// `sidebar.card.overflow` (`none`, or the lines AppKit cuts or wraps or
/// that leave its padding), `sidebar.card.width`,
/// `sidebar.tips` (each row's tooltip, or "-"), and the Files view's keys
/// (`FilesPanelView.automationState`) as `sidebar.files.*`.
/// ⌘ held alone: `sidebar hints cmd|none|MODS|key on|off|resign|deactivate`
/// (`hintsCommand`); state `sidebar.hints`, `sidebar.hints.pending`,
/// `sidebar.hints.fade`.
/// The rail: `sidebar rail select N|hover N|group N|scroll top|bottom|marker
/// top|bottom|wait|expand`; state `rail.shown`, `rail.rows`, `rail.groups`,
/// `rail.tips`, `rail.group.tips`, `rail.fits`, `rail.spinning`,
/// `rail.keyboard`, `rail.offsets` (how far each row's content sits off its
/// middle) and `rail.insets` (each row's left/right inset in the rail),
/// `rail.centred` (both within a point), `rail.overflow`, `rail.markers`,
/// `rail.wait`, `rail.active.visible`, `rail.heights` (each workspace
/// row's), `rail.dirty` (each row's work tree: `-|•|-`), `rail.hints` (its ⌘N badges showing, as `sidebar.hints` writes
/// them, or `hidden`), `rail.hints.fade`, `rail.hints.placed` (each badge
/// centred on its row inside the rail, its icon out). Also
/// `workspace.active`: the active workspace's name.
/// Dragging rows to reorder, in whichever of the two shows:
/// `sidebar drag N|to N|out|drop` and the `drag.*` keys (`DragAutomation`).
@MainActor
enum SidebarAutomation {
    static func run(_ arg: String, controller: WindowController?, app: AppController? = nil) -> String? {
        guard let sidebar = controller?.sidebarView else { return "no window" }
        let parts = arg.split(separator: " ", maxSplits: 1).map(String.init)
        let rest = parts.count > 1 ? parts[1] : ""
        switch parts.first ?? "" {
        case "hints":
            guard let controller, let hints = app?.workspaceHints else { return "no app" }
            return hintsCommand(rest, hints: hints, controller: controller)
        case "mode":
            guard let mode = SidebarView.Mode(name: rest) else { return "mode is workspaces or files" }
            sidebar.setMode(mode)
        case "select":
            guard let n = Int(rest), sidebar.rows.indices.contains(n - 1) else { return "no row \(rest)" }
            sidebar.rows[n - 1].press()
        case "hover":
            // `sidebar hover N` (0 for none): the hover fill (a full
            // sidebar row has no card).
            let n = Int(rest) ?? 0
            HoverCard.cancel()
            HoverCard.hide()
            for (i, row) in sidebar.rows.enumerated() { row.hover(i == n - 1) }
        case "scroll":
            return sidebar.scroll(rest)
        case "pill":
            return sidebar.pressPill(top: rest == "top")
        case "focus":
            guard let panel = sidebar.filesPanel else { return "the Files view isn't shown" }
            panel.focusTree()
        case "files":
            guard let panel = sidebar.filesPanel else { return "the Files view isn't shown" }
            let bits = rest.split(separator: " ", maxSplits: 1).map(String.init)
            return panel.automate(bits.first ?? "", bits.count > 1 ? bits[1] : "")
        case "rail":
            guard let rail = controller?.railView, !rail.isHiddenOrHasHiddenAncestor else { return "the rail isn't shown" }
            return rail.automate(rest.split(separator: " ").map(String.init))
        case "drag":
            return DragAutomation.run(rest.split(separator: " ").map(String.init), host: dragHost(controller))
        default:
            return "unknown sidebar verb \(parts.first ?? "")"
        }
        return nil
    }

    /// `sidebar hints cmd|none|cmd+shift…`: the modifiers now held, as a
    /// flagsChanged event handed to `WorkspaceHints` in-process (nothing
    /// system-wide); `key on|off`: count the window as key in a background
    /// run, which has no key window; `resign`: the window resigns key (its
    /// delegate's call); `deactivate`: the app goes to the background.
    private static func hintsCommand(_ arg: String, hints: WorkspaceHints,
                                     controller: WindowController) -> String? {
        let bits = arg.split(separator: " ").map(String.init)
        switch bits.first ?? "" {
        case "key":
            WorkspaceHints.assumeKey = bits.count > 1 && bits[1] == "on"
        case "resign":
            controller.windowDidResignKey(Notification(name: NSWindow.didResignKeyNotification, object: controller.window))
        case "deactivate":
            // Going to the background lets go of what automation held.
            InputState.hold([])
            hints.appResignedActive()
        default:
            var flags: NSEvent.ModifierFlags = []
            for part in (bits.first ?? "").split(separator: "+") {
                switch part {
                case "cmd": flags.insert(.command)
                case "shift": flags.insert(.shift)
                case "alt": flags.insert(.option)
                case "ctrl": flags.insert(.control)
                case "none": break
                default: return "sidebar hints takes cmd|none|MODS, key on|off, resign or deactivate"
                }
            }
            let code: UInt16 = flags.contains(.command) ? 55 : flags.contains(.shift) ? 56
                : flags.contains(.option) ? 58 : flags.contains(.control) ? 59 : 55
            guard let event = NSEvent.keyEvent(
                with: .flagsChanged, location: .zero, modifierFlags: flags,
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: controller.window.windowNumber,
                context: nil, characters: "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: code)
            else { return "couldn't make a flagsChanged event" }
            // From here on these are held (the sidebar opening with ⌘ held
            // reads them).
            InputState.hold(flags)
            hints.flagsChanged(event)
        }
        return nil
    }

    /// `sidebar.hints`: the badges showing, as `sidebar.numbers` writes rows
    /// ("1 l1|…|9 t1", the chord's digit, rows without one left out), or
    /// `hidden`; `sidebar.hints.pending`: the wait after ⌘ is running;
    /// `sidebar.hints.fade`: the last show or hide's fade in ms (0 with
    /// motion off).
    static func hintState(app: AppController, controller: WindowController) -> [String: String] {
        let sidebar = controller.sidebarView
        let shown = sidebar.rows.compactMap { row in
            row.hintShown ? row.hintText.map { "\($0.dropFirst()) \(row.dragName)" } : nil
        }
        return [
            "sidebar.hints": sidebar.hintsShown ? shown.joined(separator: "|") : "hidden",
            "sidebar.hints.pending": app.workspaceHints.isPending ? "yes" : "no",
            "sidebar.hints.fade": "\(Int((sidebar.hintsFade * 1000).rounded()))",
        ]
    }

    /// The rows a drag works on: the rail when it shows, else the full
    /// sidebar's Workspaces list.
    private static func dragHost(_ controller: WindowController?) -> (NSView & WorkspaceDragHost)? {
        guard let controller else { return nil }
        if !controller.railView.isHiddenOrHasHiddenAncestor { return controller.railView }
        let sidebar = controller.sidebarView
        return sidebar.isHiddenOrHasHiddenAncestor || sidebar.mode != .workspaces ? nil : sidebar
    }

    static func state(_ controller: WindowController) -> [String: String] {
        let sidebar = controller.sidebarView
        // Rows a refresh just rebuilt get their frames on the next layout
        // pass: now, so geometry reads (rail.centred, sidebar.overflow)
        // never see them unplaced.
        sidebar.layoutSubtreeIfNeeded()
        controller.railView.layoutSubtreeIfNeeded()
        var s = [
            "sidebar.mode": sidebar.mode.rawValue,
            "sidebar.statuses": sidebar.statuses.joined(separator: "|"),
            "sidebar.footer": sidebar.footerCounts,
            "sidebar.footer.words": sidebar.footerWords.isEmpty ? "-" : sidebar.footerWords,
            "sidebar.numbers": sidebar.numbers.joined(separator: "|"),
            "sidebar.rows": sidebar.rowTexts.joined(separator: " ‖ "),
            "sidebar.heights": sidebar.heights,
            // The status icons on their rows' text: each one's centre less
            // its text block's, and whether all sit within half a point.
            "sidebar.icons": sidebar.iconOffsets,
            "sidebar.icons.centred": sidebar.rows.allSatisfy { abs($0.iconOffset) <= 0.5 } ? "yes" : "no",
            "sidebar.overflow": sidebar.overflow.summary,
            "sidebar.pills": sidebar.pills,
            "sidebar.fades": sidebar.fades,
            "sidebar.active.visible": sidebar.activeVisible ? "yes" : "no",
            "sidebar.empty": sidebar.emptyText,
            "sidebar.card": HoverCard.text.isEmpty ? "-" : HoverCard.text,
            // The card as laid out: each line as drawn (cut to fit), how many
            // text lines each takes, any AppKit would cut or wrap itself or
            // that leave the card's padding, and its width.
            "sidebar.card.drawn": HoverCard.drawn.isEmpty ? "-" : HoverCard.drawn,
            "sidebar.card.lines": HoverCard.lineCounts.isEmpty ? "-" : HoverCard.lineCounts,
            "sidebar.card.overflow": HoverCard.overflow.isEmpty ? "-" : HoverCard.overflow,
            "sidebar.card.width": HoverCard.width.isEmpty ? "-" : HoverCard.width,
            // Each row's tooltip (its full name and branch when cut short), or "-".
            "sidebar.tips": sidebar.rows.map { $0.toolTip?.replacingOccurrences(of: "\n", with: " ¦ ") ?? "-" }
                .joined(separator: " ‖ "),
        ]
        // The rail: rows, groups, tooltips (lines joined by " ¦ ", rows by
        // " ‖ "), whether it all fits, spinning working rings, whether the
        // keyboard is in it (never), and how centred its rows are.
        let rail = controller.railView
        s["rail.shown"] = rail.isHiddenOrHasHiddenAncestor ? "no" : "yes"
        s["rail.rows"] = rail.rowSummary
        s["rail.groups"] = rail.groupSummary
        s["rail.tips"] = rail.workspaceRows.map(\.tipText).map { $0.replacingOccurrences(of: "\n", with: " ¦ ") }
            .joined(separator: " ‖ ")
        s["rail.group.tips"] = rail.groups.map(\.tipText).map { $0.replacingOccurrences(of: "\n", with: " ¦ ") }
            .joined(separator: " ‖ ")
        s["rail.fits"] = rail.fits ? "yes" : "no"
        s["rail.spinning"] = "\(rail.workspaceRows.filter(\.isSpinning).count)"
        s["rail.offsets"] = rail.contentOffsets
        s["rail.insets"] = rail.rowInsets
        s["rail.centred"] = rail.centred ? "yes" : "no"
        s["rail.overflow"] = rail.overflow.summary
        s["rail.markers"] = rail.markers
        s["rail.wait"] = rail.waitText
        s["rail.active.visible"] = rail.activeVisible ? "yes" : "no"
        // Each rail row's work tree: • with uncommitted changes, else -.
        s["rail.dirty"] = rail.workspaceRows.map { $0.info.dirty ? "•" : "-" }.joined(separator: "|")
        s["rail.heights"] = rail.workspaceRows.map { "\(Int($0.frame.height.rounded()))" }.joined(separator: "|")
        // The ⌘N badges: as `sidebar.hints` writes them (the chord's digit
        // and the workspace's name), or `hidden`; `rail.hints.placed`: every
        // badge showing sits inside the rail, centred on its row, its icon
        // faded out (and every hidden one's icon back).
        s["rail.hints"] = rail.hintsShown
            ? rail.workspaceRows.filter(\.hintShown)
                .compactMap { row in row.chord.map { "\($0.dropFirst()) \(row.info.name)" } }.joined(separator: "|")
            : "hidden"
        s["rail.hints.fade"] = "\(Int((rail.hintsFade * 1000).rounded()))"
        s["rail.hints.placed"] = rail.hintsPlaced ? "yes" : "no"
        s["workspace.active"] = controller.model?.workspace?.name ?? "-"
        s.merge(DragAutomation.state(host: dragHost(controller))) { $1 }
        let responder = controller.window.firstResponder as? NSView
        s["rail.keyboard"] = responder.map { $0.isDescendant(of: rail) } == true ? "yes" : "no"
        if sidebar.mode == .files {
            s["sidebar.files.title"] = sidebar.filesTitleText
            s["sidebar.files.header"] = sidebar.filesHeaderText.isEmpty ? "-" : sidebar.filesHeaderText
            s["sidebar.files.note"] = sidebar.filesNoteText.isEmpty ? "-" : sidebar.filesNoteText
        }
        if let panel = sidebar.filesPanel {
            for (key, value) in panel.automationState() {
                s["sidebar.files." + key] = value
            }
            let responder = controller.window.firstResponder as? NSView
            s["sidebar.files.keyboard"] = responder.map { $0.isDescendant(of: panel) } == true ? "yes" : "no"
        }
        return s
    }
}
