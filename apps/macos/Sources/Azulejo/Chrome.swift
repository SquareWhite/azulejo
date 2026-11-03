import AppKit

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
