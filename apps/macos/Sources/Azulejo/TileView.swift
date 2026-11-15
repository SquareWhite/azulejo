import AppKit

/// What a tile kind implements to live in a `TileContainerView`. Terminal
/// (1.7), browser (1.8) and preview (1.9) tiles each provide one; until then
/// every kind gets a `PlaceholderTileView`.
///
/// Lifecycle: the container makes one view per tile ID and keeps it across
/// layout changes, so a terminal's surface or a browser's page survive splits,
/// swaps and moves. It only ever sets the view's frame; it never recreates the
/// view for a layout change.
///
/// 1. `TileViews.make` creates the view with the tile's first model.
/// 2. `update(_:)` runs whenever the core reports new state for the tile
///    (title, URL, path, herdr binding, `ended`). Cheap and idempotent:
///    compare with what you have.
/// 3. `setFocused(_:)` runs when the tile gains or loses focus in the core.
///    Focused tiles should take first responder (`window?.makeFirstResponder`)
///    so typing reaches them; the container draws the focus ring.
/// 4. `tileWillClose()` runs once before the view is removed for good: detach
///    surfaces, stop loads, close child processes.
///
/// Keys: the window's key router sees every keyDown first and only lets
/// unbound chords through, so a tile's `keyDown` gets plain typing. A tile can
/// report a different kind of context (e.g. a browser's address bar has focus)
/// via `keyContext`.
@MainActor
protocol TileView: NSView {
    var tileID: String { get }
    var kind: TileKind { get }
    func update(_ tile: TileModel)
    func setFocused(_ focused: Bool)
    func tileWillClose()
    /// The keymap context while this tile has focus (`"terminal"`, …). Defaults
    /// to the tile's kind.
    var keyContext: String { get }
}

extension TileView {
    var keyContext: String { kind.rawValue }
}

/// Makes tile views. Tile kinds register here as they land; anything not
/// registered gets a placeholder.
@MainActor
enum TileViews {
    typealias Maker = @MainActor (TileModel) -> any TileView
    private static var makers: [TileKind: Maker] = [:]

    static func register(_ kind: TileKind, _ maker: @escaping Maker) {
        makers[kind] = maker
    }

    /// The tile's view inside a `TileFrame`, which adds the header strip (F28).
    static func make(_ tile: TileModel) -> any TileView {
        let content = makers[tile.kind].map { $0(tile) } ?? PlaceholderTileView(tile)
        let frame = TileFrame.wrap(content)
        frame.prime(tile)
        return frame
    }
}

/// Stand-in for tile kinds that haven't landed yet: a coloured panel naming
/// the tile. Takes first responder when focused so the key router's context
/// and the Edit menu behave as they will with real tiles.
@MainActor
final class PlaceholderTileView: NSView, TileView, ThemeAware {
    let tileID: String
    let kind: TileKind
    private let label = NSTextField(labelWithString: "")
    private var model: TileModel

    init(_ tile: TileModel) {
        tileID = tile.id
        kind = tile.kind
        model = tile
        super.init(frame: .zero)
        wantsLayer = true
        Theme.track(self)
        label.font = Design.Font.tileLabel
        label.textColor = Design.Color.textOnAccent
        label.alignment = .center
        label.maximumNumberOfLines = 4
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        NSLayoutConstraint.activate([
            label.centerXAnchor.constraint(equalTo: centerXAnchor),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
            label.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor, constant: -16),
        ])
        refresh()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func applyTheme() {
        layer?.backgroundColor = Design.Color.placeholder(kind).cgColor
    }

    override var acceptsFirstResponder: Bool { true }

    func update(_ tile: TileModel) {
        guard tile != model else { return }
        model = tile
        refresh()
    }

    func setFocused(_ focused: Bool) {
        if focused, let window, window.firstResponder !== self {
            window.makeFirstResponder(self)
        }
    }

    func tileWillClose() {}

    private func refresh() {
        var lines = ["\(kind.rawValue) · \(Short.id(tileID))"]
        if let title = model.title, !title.isEmpty { lines.append(title) }
        switch kind {
        case .terminal:
            if model.ended == true {
                lines.append("session ended")
            } else if let herdr = model.herdr {
                lines.append("herdr \(herdr.paneId)")
            } else {
                lines.append("no herdr pane")
            }
        case .browser: if let url = model.url { lines.append(url) }
        case .preview: if let path = model.path { lines.append(path) }
        case .files: if let root = model.root { lines.append(root) }
        }
        label.stringValue = lines.joined(separator: "\n")
    }
}

/// Short, stable labels for IDs in placeholder views and logs: the last four
/// characters of the ULID (`tile_01J…ABCD` → `ABCD`).
enum Short {
    static func id(_ id: String) -> String { String(id.suffix(4)) }
}
