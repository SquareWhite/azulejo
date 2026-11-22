import AppKit
import QuartzCore

/// The full sidebar's width. Dragging the card's inner edge
/// (`SidebarEdgeView`) sets it, between `Design.Space.sidebarMinWidth` and
/// `sidebarMaxWidth`; a double-click on the edge puts the default
/// (`sidebarWidth`) back. One width for every window, kept across launches
/// in `SettingsStore` (a dev instance keeps its own, as it does the
/// sidebar's Agents | Files mode).
@MainActor
enum SidebarWidth {
    /// Posted when the saved width changes: every window lays out again.
    static let changed = Notification.Name("azj.sidebarWidthChanged")

    private static let key = "sidebarWidth"

    /// The width the sidebar opens at: the saved one, else the default.
    private(set) static var saved: CGFloat = {
        let stored = SettingsStore.shared.double(forKey: key)
        return stored > 0 ? clamp(CGFloat(stored)) : Design.Space.sidebarWidth
    }()

    /// Whether a width other than the default is saved.
    static var isCustom: Bool { SettingsStore.shared.object(forKey: key) != nil }

    static func clamp(_ width: CGFloat) -> CGFloat {
        min(max(width.rounded(), Design.Space.sidebarMinWidth), Design.Space.sidebarMaxWidth)
    }

    /// `width` as a window `windowWidth` wide shows it: clamped, and never
    /// leaving the tiles less than `sidebarTilesMinWidth` (down to the
    /// sidebar's own minimum).
    static func fitted(_ width: CGFloat, windowWidth: CGFloat) -> CGFloat {
        let room = windowWidth - 2 * Design.Space.windowInset - Design.Space.sidebarTilesMinWidth
        return min(clamp(width), max(Design.Space.sidebarMinWidth, room.rounded(.down)))
    }

    /// Saves `width` (nil: back to the default) and lays every window out
    /// again.
    static func save(_ width: CGFloat?) {
        if let width {
            saved = clamp(width)
            SettingsStore.shared.set(Double(saved), forKey: key)
        } else {
            saved = Design.Space.sidebarWidth
            SettingsStore.shared.removeObject(forKey: key)
        }
        NotificationCenter.default.post(name: changed, object: nil)
    }
}

/// The full sidebar card's inner edge: a strip over the gutter between the
/// card and the tiles (and `sidebarEdgeInset` into the card) that shows the
/// resize cursor and drags the sidebar's width. The tile area follows live,
/// its terminals reflowing by whole cells as in a divider drag; the width
/// is saved on release. A double-click puts the default width back.
@MainActor
final class SidebarEdgeView: NSView {
    /// A drag began, moved the edge `dx` points from where it began, or ended.
    var onBegin: (() -> Void)?
    var onDrag: ((CGFloat) -> Void)?
    var onEnd: (() -> Void)?
    /// A double-click: the default width.
    var onReset: (() -> Void)?
    private var start: CGFloat?
    static let cursor = NSCursor.resizeLeftRight

    override var mouseDownCanMoveWindow: Bool { false }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: Self.cursor)
    }

    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 2 {
            start = nil
            onReset?()
            return
        }
        start = event.locationInWindow.x
        InputState.setCursor(Self.cursor)
        onBegin?()
    }

    override func mouseDragged(with event: NSEvent) {
        guard let start else { return }
        // Off the strip the pointer keeps the resize cursor until release.
        InputState.setCursor(Self.cursor)
        onDrag?(event.locationInWindow.x - start)
    }

    override func mouseUp(with event: NSEvent) {
        guard let start else { return }
        onDrag?(event.locationInWindow.x - start)
        self.start = nil
        onEnd?()
        window?.invalidateCursorRects(for: self)
    }

    /// Automation: a drag is under way.
    var isDragging: Bool { start != nil }
}

/// The edge's drag, in the chrome: the card and the tiles take the new
/// width at once (no motion), terminals follow live by whole cells
/// (`liveResizeWillStart`/`liveResizeTick`) and get one exact resize on
/// release.
extension ChromeLayoutView {
    func beginSidebarResize() {
        let tiles = tiles as? TileContainerView
        tiles?.stopAllMotion()
        sidebarResize = SidebarResizeDrag(startWidth: sidebarWidth)
        for (_, view) in tiles?.liveTileViews ?? [] {
            (view as? MotionAware)?.liveResizeWillStart(final: view.frame.size)
        }
    }

    func dragSidebarResize(by dx: CGFloat) {
        guard var drag = sidebarResize else { return }
        let width = SidebarWidth.fitted(drag.startWidth + dx, windowWidth: bounds.width)
        drag.events += 1
        guard width != sidebarWidth else {
            sidebarResize = drag
            return
        }
        drag.width = width
        drag.steps += 1
        sidebarResize = drag
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        needsLayout = true
        layoutSubtreeIfNeeded()
        CATransaction.commit()
        for (_, view) in (tiles as? TileContainerView)?.liveTileViews ?? [] {
            (view as? MotionAware)?.liveResizeTick()
        }
    }

    func endSidebarResize() {
        guard let drag = sidebarResize else { return }
        sidebarResize = nil
        let tiles = tiles as? TileContainerView
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        // Saving lays every window out at the new width (this one: the same).
        SidebarWidth.save(drag.width ?? drag.startWidth)
        needsLayout = true
        layoutSubtreeIfNeeded()
        for (_, view) in tiles?.liveTileViews ?? [] { (view as? MotionAware)?.liveResizeDidEnd() }
        CATransaction.commit()
        lastSidebarResize = (drag.steps, drag.events)
    }
}

/// A drag of the sidebar's edge under way.
struct SidebarResizeDrag {
    let startWidth: CGFloat
    /// The width it's at (nil until it moves).
    var width: CGFloat?
    /// Pointer events, and the ones that changed the width.
    var events = 0
    var steps = 0
}

/// Automation for the sidebar's edge (`sidebar width …` in `.azs` scripts).
/// Mouse events are made in the app and handed to whatever view hit-tests
/// at the edge, as AppKit would deliver them: nothing goes through the
/// window server.
///
///     sidebar width drag W [MS]   press on the edge, move it so the card is
///                                 W points wide over MS ms (default 300), one
///                                 event per display frame, release; waits
///     sidebar width double        double-click the edge (the default width)
///
/// Keys: `sidebar.width` (the card's width as laid out), `sidebar.width.saved`
/// (the saved width, `default` when none is), `sidebar.edge` (what a click on
/// the edge's middle reaches: `SidebarEdgeView`, else the view's class, or
/// `hidden`), `sidebar.edge.steps` (the last drag's width changes),
/// `sidebar.gap` (from the card's right edge to the tiles' left one).
@MainActor
enum SidebarEdgeAutomation {
    static func run(_ arg: String, controller: WindowController?) -> (problem: String?, wait: TimeInterval) {
        guard let controller else { return ("no window", 0) }
        let edge = controller.sidebarEdge
        let parts = arg.split(separator: " ").map(String.init)
        guard !edge.isHiddenOrHasHiddenAncestor, let superview = edge.superview else {
            return ("the sidebar's edge isn't shown", 0)
        }
        let at = CGPoint(x: edge.frame.midX, y: edge.frame.midY)
        guard superview.hitTest(at) === edge else { return ("the edge doesn't take the pointer", 0) }
        switch parts.first ?? "" {
        case "double":
            for clicks in 1...2 {
                edge.mouseDown(with: event(.leftMouseDown, at: at, in: superview, clicks: clicks))
                edge.mouseUp(with: event(.leftMouseUp, at: at, in: superview, clicks: clicks))
            }
            return (nil, 0.1)
        case "drag":
            guard parts.count >= 2, let width = Double(parts[1]) else { return ("sidebar width drag W [MS]", 0) }
            let ms = parts.count > 2 ? Double(parts[2]) ?? 300 : 300
            let dx = CGFloat(width) - controller.sidebarCardWidth
            let end = CGPoint(x: at.x + dx, y: at.y)
            edge.mouseDown(with: event(.leftMouseDown, at: at, in: superview))
            guard edge.isDragging else { return ("mouse-down didn't start a drag", 0) }
            let duration = ms / 1000
            let frame = 1 / Double(Motion.refreshRate)
            let began = CACurrentMediaTime()
            func step() {
                let t = min((CACurrentMediaTime() - began) / max(duration, frame), 1)
                if t < 1 {
                    let p = CGPoint(x: at.x + (end.x - at.x) * t, y: at.y)
                    edge.mouseDragged(with: event(.leftMouseDragged, at: p, in: superview))
                    DispatchQueue.main.asyncAfter(deadline: .now() + frame) { step() }
                    return
                }
                edge.mouseUp(with: event(.leftMouseUp, at: end, in: superview))
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + frame) { step() }
            return (nil, duration + 0.15)
        default:
            return ("sidebar width drag W [MS] | double", 0)
        }
    }

    static func state(_ controller: WindowController) -> [String: String] {
        let edge = controller.sidebarEdge
        var s = [
            "sidebar.width": "\(Int(controller.sidebarCardWidth.rounded()))",
            "sidebar.width.saved": SidebarWidth.isCustom ? "\(Int(SidebarWidth.saved))" : "default",
            "sidebar.edge.steps": "\(controller.lastSidebarResize.steps)",
            "sidebar.gap": "\(Int(controller.sidebarTilesGap.rounded()))",
        ]
        if edge.isHiddenOrHasHiddenAncestor {
            s["sidebar.edge"] = "hidden"
        } else if let superview = edge.superview {
            let hit = superview.hitTest(CGPoint(x: edge.frame.midX, y: edge.frame.midY))
            s["sidebar.edge"] = hit.map { "\(type(of: $0))" } ?? "nil"
        }
        return s
    }

    private static func event(_ type: NSEvent.EventType, at point: CGPoint, in view: NSView,
                              clicks: Int = 1) -> NSEvent {
        NSEvent.mouseEvent(
            with: type, location: view.convert(point, to: nil), modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: view.window?.windowNumber ?? 0,
            context: nil, eventNumber: 0, clickCount: clicks, pressure: type == .leftMouseUp ? 0 : 1)!
    }
}
