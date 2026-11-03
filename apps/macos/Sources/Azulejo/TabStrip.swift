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
