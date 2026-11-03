import AppKit

/// Lays out the active tab's tiles from the core's tree. Keeps one view per
/// tile for every tab in the window, so switching tabs or rearranging tiles
/// never recreates a tile (a terminal keeps its surface, a browser its page).
@MainActor
final class TileContainerView: NSView {
    static let gap: CGFloat = 4
    static let inset: CGFloat = 4

    private var views: [String: any TileView] = [:]
    private var tab: TabModel?
    private let ring = FocusRingView()

    override var isFlipped: Bool { false }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        addSubview(ring)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// The focused tile's view, if any.
    var focusedView: (any TileView)? {
        tab.flatMap { views[$0.focused] }
    }

    /// Shows `window`'s active tab. Tiles of other tabs keep their views,
    /// hidden; tiles gone from the window are closed and dropped.
    func apply(_ window: WindowModel) {
        let all = window.workspaces.flatMap { $0.tabs }.flatMap { $0.tiles }
        let alive = Set(all.map(\.id))
        for (id, view) in views where !alive.contains(id) {
            view.tileWillClose()
            view.removeFromSuperview()
            views[id] = nil
        }
        for tile in all {
            if let view = views[tile.id] {
                view.update(tile)
            } else {
                views[tile.id] = TileViews.make(tile)
            }
        }

        let previousFocus = tab?.focused
        tab = window.workspace?.tab
        let visible = Set(visibleTiles)
        for (id, view) in views {
            if visible.contains(id) {
                if view.superview !== self { addSubview(view, positioned: .below, relativeTo: ring) }
            } else if view.superview != nil {
                view.removeFromSuperview()
            }
        }
        needsLayout = true
        layoutSubtreeIfNeeded()

        if let tab {
            for (id, view) in views where visible.contains(id) {
                view.setFocused(id == tab.focused)
            }
        }
        if previousFocus != tab?.focused { ring.pulse() }
    }

    /// Tile IDs on screen: the zoomed tile alone, or every tile in the tab.
    private var visibleTiles: [String] {
        guard let tab else { return [] }
        if let zoomed = tab.zoomed { return [zoomed] }
        return tab.order
    }

    /// Where each visible tile sits, in this view's coordinates.
    func tileFrames() -> [String: CGRect] {
        guard let tab else { return [:] }
        let bounds = self.bounds.insetBy(dx: Self.inset, dy: Self.inset)
        if let zoomed = tab.zoomed { return [zoomed: bounds] }
        return tab.root.frames(in: bounds, gap: Self.gap)
    }

    override func layout() {
        super.layout()
        let frames = tileFrames()
        for (id, frame) in frames { views[id]?.frame = frame }
        if let tab, let focused = frames[tab.focused], frames.count > 1 || tab.zoomed != nil {
            ring.isHidden = false
            ring.frame = focused.insetBy(dx: -2, dy: -2)
            ring.zoomed = tab.zoomed != nil
        } else {
            ring.isHidden = true
        }
    }

    /// Clicking a tile focuses it in the core.
    var onClickTile: ((String) -> Void)?

    override func hitTest(_ point: NSPoint) -> NSView? {
        let hit = super.hitTest(point)
        return hit === ring ? nil : hit
    }

    /// Tiles that don't handle clicks pass them up the responder chain to here.
    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        if let id = tileFrames().first(where: { $0.value.contains(point) })?.key {
            onClickTile?(id)
        }
        super.mouseDown(with: event)
    }
}

/// The accent border around the focused tile.
@MainActor
final class FocusRingView: NSView {
    var zoomed = false { didSet { needsDisplay = true } }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        isHidden = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func draw(_ dirtyRect: NSRect) {
        let path = NSBezierPath(roundedRect: bounds.insetBy(dx: 1, dy: 1), xRadius: 7, yRadius: 7)
        path.lineWidth = 2
        (zoomed ? NSColor.systemOrange : NSColor.controlAccentColor).setStroke()
        path.stroke()
    }

    /// A short fade-in when focus moves, so the eye follows it.
    func pulse() {
        alphaValue = 0.35
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.12
            animator().alphaValue = 1
        }
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}
