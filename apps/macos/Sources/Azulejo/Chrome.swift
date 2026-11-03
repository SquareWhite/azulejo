import AppKit

/// The workspace list down the left edge: one row per workspace with its
/// 1-based number (what `cmd+1..8` switches to) and a dot when its agents
/// need a look (red: blocked, green: done, blue: working).
@MainActor
final class SidebarView: NSView {
    static let width: CGFloat = 168
    var onSelect: ((String) -> Void)?
    private let stack = NSStackView()
    private var shown: [String] = []

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = NSColor.underPageBackgroundColor.cgColor
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

    func apply(_ window: WindowModel) {
        let rollups = window.workspaces.map { AttentionStyle.Rollup($0.tabs.flatMap(\.tiles)) }
        let key = zip(window.workspaces, rollups).map { ws, r in
            "\(ws.id)|\(ws.name)|\(ws.id == window.activeWorkspace)|\(r.blocked)|\(r.attention)|\(r.working)"
                + "|\(ws.git?.branch ?? "")|\(ws.git?.dirty == true)"
        }
        guard key != shown else { return }
        shown = key
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        let header = NSTextField(labelWithString: "WORKSPACES")
        header.font = .systemFont(ofSize: 10, weight: .semibold)
        header.textColor = .secondaryLabelColor
        stack.addArrangedSubview(header)
        for (i, ws) in window.workspaces.enumerated() {
            let row = RowButton(
                title: "\(i + 1)  \(ws.name)", selected: ws.id == window.activeWorkspace,
                marker: rollups[i].marker, detail: Self.branch(ws.git),
                detailWidth: Self.width - 16
            ) { [weak self] in self?.onSelect?(ws.id) }
            stack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -16).isActive = true
        }
    }
}

extension SidebarView {
    /// Automation: each workspace row's marker, in order.
    var markers: [String] { stackRows.map(\.markerText) }

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

/// The active workspace's tabs, across the top.
@MainActor
final class TabBarView: NSView {
    static let height: CGFloat = 30
    var onSelect: ((String) -> Void)?
    private let stack = NSStackView()
    private var shown: [String] = []

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
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
            "\(tab.id)|\(tab.label)|\(tab.id == active)|\(tab.tiles.count)|\(r.blocked)|\(r.attention)|\(r.working)"
        }
        guard key != shown else { return }
        shown = key
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for (i, tab) in tabs.enumerated() {
            let count = tab.tiles.count > 1 ? " ·\(tab.tiles.count)" : ""
            stack.addArrangedSubview(RowButton(
                title: "\(i + 1) \(tab.label)\(count)", selected: tab.id == active,
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
        font = .systemFont(ofSize: 12, weight: selected ? .semibold : .regular)
        contentTintColor = selected ? .labelColor : .secondaryLabelColor
        if marker != nil || detail != nil {
            let style = NSMutableParagraphStyle()
            style.lineBreakMode = .byTruncatingTail
            if detail != nil {
                style.tabStops = [NSTextTab(textAlignment: .right, location: max(detailWidth - 16, 40))]
            }
            let text = NSMutableAttributedString(string: title, attributes: [
                .font: font ?? .systemFont(ofSize: 12),
                .foregroundColor: selected ? NSColor.labelColor : NSColor.secondaryLabelColor,
                .paragraphStyle: style,
            ])
            if let marker { text.append(marker) }
            if let detail {
                text.append(NSAttributedString(string: "\t" + Self.shorten(detail, to: 16), attributes: [
                    .font: NSFont.systemFont(ofSize: 11),
                    .foregroundColor: NSColor.tertiaryLabelColor,
                    .paragraphStyle: style,
                ]))
                toolTip = detail
            }
            attributedTitle = text
        }
        wantsLayer = true
        layer?.cornerRadius = 5
        layer?.backgroundColor = selected
            ? NSColor.selectedContentBackgroundColor.withAlphaComponent(0.25).cgColor : nil
        target = self
        action = #selector(clicked)
        refusesFirstResponder = true
        setContentHuggingPriority(.defaultHigh, for: .horizontal)
        heightAnchor.constraint(equalToConstant: 22).isActive = true
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
