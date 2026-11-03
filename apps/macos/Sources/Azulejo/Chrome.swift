import AppKit

/// The workspace list down the left edge: one row per workspace with its
/// 1-based number (what `cmd+1..8` switches to) and a dot when its agents
/// need a look (red: blocked, green: done, blue: working).
///
/// With herdr machines besides Local (ADR 0011) the rows are grouped by
/// machine, each group headed by its name, a connection dot and its state
/// ("Reconnecting…"), with a "+" that creates a workspace there. Minimal
/// visuals; the mockup's styling comes with M2.5.
@MainActor
final class SidebarView: NSView {
    static let width = Design.Space.sidebarWidth
    var onSelect: ((String) -> Void)?
    /// "+" on a machine group: create a workspace on that machine.
    var onAdd: ((String) -> Void)?
    private let stack = NSStackView()
    private var shown: [String] = []

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = Design.Color.sidebar.cgColor
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 2
        stack.edgeInsets = NSEdgeInsets(top: 10, left: 8, bottom: 8, right: 8)
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    var stackRows: [RowButton] { stack.arrangedSubviews.compactMap { $0 as? RowButton } }

    /// A machine group: its machine (nil for workspaces whose machine left
    /// herdr's list) and its workspaces with their window-order numbers.
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
        for (i, ws) in window.workspaces.enumerated() {
            if known.insert(ws.machineID).inserted {
                groups.append(Group(id: ws.machineID, machine: nil, workspaces: []))
            }
            if let g = groups.firstIndex(where: { $0.id == ws.machineID }) {
                groups[g].workspaces.append((i + 1, ws))
            }
        }
        return groups
    }

    func apply(_ window: WindowModel, machines: [MachineModel] = []) {
        let rollups = Dictionary(uniqueKeysWithValues: window.workspaces.map {
            ($0.id, AttentionStyle.Rollup($0.tabs.flatMap(\.tiles)))
        })
        let groups = Self.groups(window, machines: machines)
        // Only Local and no stray machines: the plain list, as before.
        let grouped = groups.count > 1
        var key = window.workspaces.map { ws in
            let r = rollups[ws.id]
            return "\(ws.id)|\(ws.name)|\(ws.machineID)|\(ws.id == window.activeWorkspace)|\(r?.blocked ?? 0)|\(r?.attention ?? 0)|\(r?.working ?? 0)"
                + "|\(ws.git?.branch ?? "")|\(ws.git?.dirty == true)"
        }
        if grouped {
            key += groups.map { "\($0.id)|\($0.machine?.label ?? "")|\($0.machine?.state.rawValue ?? "")|\($0.machine?.message ?? "")" }
        }
        guard key != shown else { return }
        shown = key
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        guard grouped else {
            addHeader(MachineHeader(title: "WORKSPACES"))
            for (i, ws) in window.workspaces.enumerated() {
                addRow(number: i + 1, ws, window: window, rollup: rollups[ws.id])
            }
            return
        }
        for group in groups {
            let header = MachineHeader(group: group)
            // No "+" where nothing would run: off, or gone from herdr's list.
            header.onAdd = group.machine.flatMap { m in
                m.state == .off ? nil : { [weak self] in self?.onAdd?(m.id) }
            }
            addHeader(header)
            for (number, ws) in group.workspaces {
                addRow(number: number, ws, window: window, rollup: rollups[ws.id])
            }
        }
    }

    private func addHeader(_ header: NSView) {
        stack.addArrangedSubview(header)
        header.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -16).isActive = true
    }

    private func addRow(number: Int, _ ws: WorkspaceModel, window: WindowModel, rollup: AttentionStyle.Rollup?) {
        let row = RowButton(
            title: "\(number)  \(ws.name)", selected: ws.id == window.activeWorkspace,
            marker: rollup?.marker, detail: Self.branch(ws.git),
            detailWidth: Self.width - 16
        ) { [weak self] in self?.onSelect?(ws.id) }
        stack.addArrangedSubview(row)
        row.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -16).isActive = true
    }
}

extension SidebarView {
    /// Automation: each workspace row's marker, in order.
    var markers: [String] { stackRows.map(\.markerText) }

    /// Automation: each machine group's header text ("dev-server ● Reconnecting…").
    var machineHeaders: [String] {
        stack.arrangedSubviews.compactMap { ($0 as? MachineHeader)?.summary }
    }

    /// Automation: each workspace row's branch, in order ("" outside a repo).
    var branches: [String] { stackRows.map(\.detailText) }

    /// The branch a row shows (F26): its name, `@abc1234` when detached, and
    /// a dot when tracked files changed.
    static func branch(_ git: GitModel?) -> String? {
        guard let git, let branch = git.branch, !branch.isEmpty else { return nil }
        let name = git.detached == true ? "@\(branch)" : branch
        return git.dirty == true ? "\(name) •" : name
    }
}

/// A sidebar section header: "WORKSPACES", or a machine's name with its
/// connection dot, its state when it isn't connected, and "+".
@MainActor
final class MachineHeader: NSView {
    var onAdd: (() -> Void)? {
        didSet { add.isHidden = onAdd == nil }
    }
    /// Automation: what the header says.
    let summary: String
    private let add = NSButton()

    init(title: String) {
        summary = title
        super.init(frame: .zero)
        build(title: title, dot: nil, state: nil, stateColor: Design.Color.textMuted)
    }

    init(group: SidebarView.Group) {
        let machine = group.machine
        let (dot, state): (NSColor, String?) = switch machine?.state {
        case .connected: (Design.Status.connected.color, nil)
        case .connecting: (Design.Status.connecting.color, "Connecting…")
        case .reconnecting: (Design.Status.reconnecting.color, "Reconnecting…")
        case .error: (Design.Status.error.color, "Needs attention")
        case .off: (Design.Status.off.color, "Off")
        case nil: (Design.Status.off.color, "Not in herdr")
        }
        let title = machine?.label ?? "machine \(group.id.prefix(8))"
        summary = [title, "●", state].compactMap { $0 }.joined(separator: " ")
        super.init(frame: .zero)
        build(title: title, dot: dot, state: state,
              stateColor: machine.map { $0.state == .off ? Design.Color.textMuted : dot } ?? Design.Color.textMuted)
        toolTip = machine?.message
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private func build(title: String, dot: NSColor?, state: String?, stateColor: NSColor) {
        let row = NSStackView()
        row.orientation = .horizontal
        row.spacing = 5
        row.alignment = .centerY
        row.translatesAutoresizingMaskIntoConstraints = false
        let name = NSTextField(labelWithString: title)
        name.font = Design.Font.groupLabel
        name.textColor = Design.Color.textMuted
        name.lineBreakMode = .byTruncatingTail
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        row.addArrangedSubview(name)
        if let dot {
            let mark = NSTextField(labelWithString: "●")
            mark.font = Design.Font.dot
            mark.textColor = dot
            row.addArrangedSubview(mark)
        }
        if let state {
            let text = NSTextField(labelWithString: state)
            text.font = Design.Font.caption
            text.textColor = stateColor
            text.lineBreakMode = .byTruncatingTail
            text.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            row.addArrangedSubview(text)
        }
        let spacer = NSView()
        spacer.setContentHuggingPriority(.init(1), for: .horizontal)
        row.addArrangedSubview(spacer)
        add.title = "+"
        add.isBordered = false
        add.font = Design.Font.addButton
        add.contentTintColor = Design.Color.textMuted
        add.refusesFirstResponder = true
        add.target = self
        add.action = #selector(addClicked)
        add.isHidden = true
        add.toolTip = "New workspace on \(title)"
        row.addArrangedSubview(add)
        addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 2),
            row.trailingAnchor.constraint(equalTo: trailingAnchor),
            row.topAnchor.constraint(equalTo: topAnchor, constant: 6),
            row.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -2),
        ])
    }

    @objc private func addClicked() { onAdd?() }
}

/// The active workspace's tabs, across the top.
@MainActor
final class TabBarView: NSView {
    static let height = Design.Space.tabBarHeight
    var onSelect: ((String) -> Void)?
    private let stack = NSStackView()
    private var shown: [String] = []

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = Design.Color.window.cgColor
        stack.orientation = .horizontal
        stack.spacing = 4
        stack.edgeInsets = NSEdgeInsets(top: 3, left: 6, bottom: 3, right: 6)
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Automation: each tab's marker, in order.
    var markers: [String] { stack.arrangedSubviews.compactMap { ($0 as? RowButton)?.markerText } }

    func apply(_ workspace: WorkspaceModel?) {
        let tabs = workspace?.tabs ?? []
        let active = workspace?.tab?.id
        let rollups = tabs.map { AttentionStyle.Rollup($0.tiles) }
        let key = zip(tabs, rollups).map { tab, r in
            "\(tab.id)|\(tab.label)|\(tab.kind)|\(tab.id == active)|\(tab.tiles.count)|\(r.blocked)|\(r.attention)|\(r.working)"
        }
        guard key != shown else { return }
        shown = key
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for (i, tab) in tabs.enumerated() {
            let count = tab.tiles.count > 1 ? " ·\(tab.tiles.count)" : ""
            // A subtle hint of the tab's layout kind (polish comes later).
            let kind = tab.kind.glyph.map { " \($0)" } ?? ""
            stack.addArrangedSubview(RowButton(
                title: "\(i + 1) \(tab.label)\(count)\(kind)", selected: tab.id == active,
                marker: rollups[i].marker
            ) { [weak self] in self?.onSelect?(tab.id) })
        }
    }
}

/// A flat, highlightable row used by the sidebar and tab bar.
@MainActor
final class RowButton: NSButton {
    private let handler: () -> Void

    /// Automation: the marker's text (" ● 2"), or "".
    let markerText: String
    /// Automation: the right-aligned detail (a branch), or "".
    let detailText: String

    /// `detail` shows dim and right-aligned (a workspace's branch); it needs
    /// `detailWidth`, the row's width.
    init(title: String, selected: Bool, marker: NSAttributedString? = nil,
         detail: String? = nil, detailWidth: CGFloat = 0, handler: @escaping () -> Void) {
        self.handler = handler
        markerText = marker?.string ?? ""
        detailText = detail ?? ""
        super.init(frame: .zero)
        self.title = title
        isBordered = false
        alignment = .left
        font = selected ? Design.Font.rowSelected : Design.Font.row
        contentTintColor = selected ? Design.Color.textPrimary : Design.Color.textSecondary
        if marker != nil || detail != nil {
            let style = NSMutableParagraphStyle()
            style.lineBreakMode = .byTruncatingTail
            if detail != nil {
                style.tabStops = [NSTextTab(textAlignment: .right, location: max(detailWidth - 16, 40))]
            }
            let text = NSMutableAttributedString(string: title, attributes: [
                .font: font ?? Design.Font.row,
                .foregroundColor: selected ? Design.Color.textPrimary : Design.Color.textSecondary,
                .paragraphStyle: style,
            ])
            if let marker { text.append(marker) }
            if let detail {
                text.append(NSAttributedString(string: "\t" + Self.shorten(detail, to: 16), attributes: [
                    .font: Design.Font.detail,
                    .foregroundColor: Design.Color.textMuted,
                    .paragraphStyle: style,
                ]))
                toolTip = detail
            }
            attributedTitle = text
        }
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.row
        layer?.backgroundColor = selected ? Design.Color.selected.cgColor : nil
        target = self
        action = #selector(clicked)
        refusesFirstResponder = true
        setContentHuggingPriority(.defaultHigh, for: .horizontal)
        heightAnchor.constraint(equalToConstant: Design.Space.rowHeight).isActive = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: NSSize {
        let size = super.intrinsicContentSize
        return NSSize(width: size.width + 14, height: 22)
    }

    @objc private func clicked() { handler() }

    /// `text` cut to `max` characters with an ellipsis.
    static func shorten(_ text: String, to max: Int) -> String {
        text.count <= max ? text : String(text.prefix(max - 1)) + "…"
    }
}
