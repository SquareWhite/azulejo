import AppKit

/// Draws a layout tree as coloured placeholder tiles. It holds no layout state of
/// its own: every change arrives as a `layout.changed` event from the core.
@MainActor
final class TileTreeView: NSView {
    private static let gap: CGFloat = 4
    private static let palette: [NSColor] = [
        .systemTeal, .systemIndigo, .systemOrange, .systemPink, .systemGreen, .systemPurple,
        .systemBrown, .systemBlue,
    ]

    private var tree: LayoutNode?
    private var focused = ""
    private var tiles: [String: TileView] = [:]

    override var isFlipped: Bool { true }

    func apply(_ event: LayoutChanged) {
        tree = event.tree
        focused = event.focused
        var seen = Set<String>()
        collectLeaves(event.tree, into: &seen)
        for (id, view) in tiles where !seen.contains(id) {
            view.removeFromSuperview()
            tiles[id] = nil
        }
        for id in seen where tiles[id] == nil {
            let view = TileView(id: id, colour: Self.colour(for: id))
            addSubview(view)
            tiles[id] = view
        }
        needsLayout = true
    }

    override func layout() {
        super.layout()
        guard let tree else { return }
        place(tree, in: bounds.insetBy(dx: Self.gap, dy: Self.gap))
    }

    private func place(_ node: LayoutNode, in rect: NSRect) {
        switch node {
        case let .leaf(id):
            tiles[id]?.frame = rect
            tiles[id]?.isFocused = id == focused
        case let .split(axis, ratio, a, b):
            let gap = Self.gap
            switch axis {
            case .horizontal:
                let width = (rect.width - gap) * ratio
                place(a, in: NSRect(x: rect.minX, y: rect.minY, width: width, height: rect.height))
                place(
                    b,
                    in: NSRect(
                        x: rect.minX + width + gap, y: rect.minY,
                        width: rect.width - width - gap, height: rect.height))
            case .vertical:
                let height = (rect.height - gap) * ratio
                place(a, in: NSRect(x: rect.minX, y: rect.minY, width: rect.width, height: height))
                place(
                    b,
                    in: NSRect(
                        x: rect.minX, y: rect.minY + height + gap,
                        width: rect.width, height: rect.height - height - gap))
            }
        }
    }

    private func collectLeaves(_ node: LayoutNode, into ids: inout Set<String>) {
        switch node {
        case let .leaf(id): ids.insert(id)
        case let .split(_, _, a, b):
            collectLeaves(a, into: &ids)
            collectLeaves(b, into: &ids)
        }
    }

    private static func colour(for id: String) -> NSColor {
        let n = Int(id.dropFirst()) ?? 0
        return palette[n % palette.count]
    }
}

/// One placeholder tile: a coloured box with its ID.
@MainActor
private final class TileView: NSView {
    private let label: NSTextField
    var isFocused = false {
        didSet { layer?.borderWidth = isFocused ? 3 : 0 }
    }

    init(id: String, colour: NSColor) {
        label = NSTextField(labelWithString: id)
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = colour.withAlphaComponent(0.75).cgColor
        layer?.cornerRadius = 6
        layer?.borderColor = NSColor.white.cgColor
        label.font = .monospacedSystemFont(ofSize: 20, weight: .semibold)
        label.textColor = .white
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        NSLayoutConstraint.activate([
            label.centerXAnchor.constraint(equalTo: centerXAnchor),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }
}
