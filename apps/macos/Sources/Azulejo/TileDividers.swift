import AppKit
import QuartzCore

/// A tile's view that keeps its own pointer shape (a terminal's, from
/// libghostty). A divider sets the resize cursor over it; leaving the
/// gutter puts this one back, since nothing else would.
@MainActor
protocol PointerShaped: AnyObject {
    var pointerCursor: NSCursor { get }
}

extension TerminalSurfaceView: PointerShaped {}

/// One split's divider: the gutter between its two sides, which the pointer
/// drags (`layout.set_ratio`). `path` names the split in the tab's tree the
/// way the core reads it: `a`/`b` steps from the root.
struct TileDivider: Equatable {
    let path: String
    let axis: LayoutNode.Axis
    let ratio: Double
    /// The rectangle the split divides, in the container's coordinates.
    let area: CGRect
    /// The gap between its two sides.
    let gutter: CGRect
    let gap: CGFloat

    /// Where the gutter's centre line sits along the axis.
    var centre: CGFloat { axis == .horizontal ? gutter.midX : gutter.midY }

    /// The ratio that puts the gutter's centre at `position` (x for a
    /// side-by-side split, y for a stacked one), within 0…1.
    func ratio(at position: CGFloat) -> Double {
        switch axis {
        case .horizontal:
            let w = area.width - gap
            guard w > 0 else { return ratio }
            return Double(min(max((position - gap / 2 - area.minX) / w, 0), 1))
        case .vertical:
            let h = area.height - gap
            guard h > 0 else { return ratio }
            return Double(min(max((area.maxY - position - gap / 2) / h, 0), 1))
        }
    }

    /// The gutter widened by `slack` on both sides, across the divider.
    func zone(slack: CGFloat) -> CGRect {
        axis == .horizontal ? gutter.insetBy(dx: -slack, dy: 0) : gutter.insetBy(dx: 0, dy: -slack)
    }

    /// Whether `point` lies along the divider (ignoring how far across).
    func spans(_ point: CGPoint) -> Bool {
        axis == .horizontal
            ? point.y >= gutter.minY && point.y <= gutter.maxY
            : point.x >= gutter.minX && point.x <= gutter.maxX
    }

    func coordinate(of point: CGPoint) -> CGFloat { axis == .horizontal ? point.x : point.y }

    var cursor: NSCursor { axis == .horizontal ? .resizeLeftRight : .resizeUpDown }
}

extension LayoutNode {
    /// Every split's divider inside `bounds`, laid out as `frames(in:gap:)`
    /// lays out the tiles.
    func dividers(in bounds: CGRect, gap: CGFloat, path: String = "") -> [TileDivider] {
        guard case .split(let axis, let ratio, let a, let b) = self else { return [] }
        let (first, second, gutter) = Self.split(bounds, axis: axis, ratio: ratio, gap: gap)
        let own = TileDivider(path: path, axis: axis, ratio: ratio, area: bounds, gutter: gutter, gap: gap)
        return [own] + a.dividers(in: first, gap: gap, path: path + "a")
            + b.dividers(in: second, gap: gap, path: path + "b")
    }
}

/// A divider drag under way (`TileContainerView.beginDividerDrag`).
struct DividerDrag {
    var divider: TileDivider
    let tab: String
    /// The pointer's offset from the gutter's centre line when the drag
    /// began, so the divider doesn't jump to the pointer.
    let grab: CGFloat
    /// Where along the divider the pointer took hold (finds it again if the
    /// tree changes shape mid-drag).
    let anchor: CGPoint
    let started: CFTimeInterval
    var lastSent: Double?
    var commands = 0
    var events = 0
    /// Terminal tiles' PTY resize counts when the drag began, and their size.
    var resizesAtStart: [String: (count: Int, size: CGSize)] = [:]
    /// Resize counts seen while the pointer was still down (live reflow).
    var liveResizes: [String: Int] = [:]
}

/// Dragging the gutters between tiles. Each split's gutter (plus `slack`
/// on either side, where that's a card's empty margin rather than content
/// or a header) takes the pointer; dragging it moves that split's divider
/// through the core (`layout.set_ratio`) on every pointer event, and the
/// tiles follow at once: no motion, terminals reflowing by whole cells as
/// in a window drag (`liveResizeWillStart`/`liveResizeTick`, no lead), then
/// one exact resize on mouse-up. A double-click evens the split out. The
/// herdr mirror waits for the tab to be quiet (ADR 0012), so it pushes once
/// after the drag.
extension TileContainerView {
    /// How far the pointer area reaches past the gutter into a card's empty
    /// margin (a terminal's padding is 8 pt).
    static let dividerSlack: CGFloat = 4
    private static let hintThickness: CGFloat = 2
    private static let hintInset: CGFloat = 10

    /// Dividers of the tab on screen: none while zoomed, in Monocle, or
    /// while tiles move.
    var dividers: [TileDivider] {
        guard let tab = shownTab, tab.zoomed == nil, tab.kind != .monocle, window != nil else { return [] }
        let dx = min(Self.inset, bounds.width / 2), dy = min(Self.inset, bounds.height / 2)
        return tab.root.dividers(in: bounds.insetBy(dx: dx, dy: dy), gap: Self.gap)
    }

    /// The divider that takes a click at `point` (container coordinates).
    /// `under`: the view the click would reach otherwise; outside the gutter
    /// proper the divider only takes points that view wouldn't use (a card's
    /// margin, the gutter), so terminals, web views and headers keep every
    /// click right up to their edges.
    func divider(at point: CGPoint, under: NSView?) -> TileDivider? {
        guard dividerDrag == nil ? !isMoving : true else { return nil }
        let inert = under == nil || under === self || under is TileFrame
        var best: (divider: TileDivider, distance: CGFloat)?
        for d in dividers {
            let inGutter = d.gutter.contains(point)
            guard inGutter || (inert && d.zone(slack: Self.dividerSlack).contains(point)) else { continue }
            let distance = inGutter ? -1 : abs(d.coordinate(of: point) - d.centre)
            if best == nil || distance < best!.distance { best = (d, distance) }
        }
        return best?.divider
    }

    // MARK: Drag

    /// Mouse-down on a divider: a double-click evens it out, a single one
    /// starts a drag.
    func dividerMouseDown(_ divider: TileDivider, event: NSEvent) {
        guard let tab = shownTab else { return }
        if event.clickCount == 2 {
            onSetRatio?(tab.id, divider.path, nil)
            return
        }
        let point = convert(event.locationInWindow, from: nil)
        beginDividerDrag(divider, tab: tab.id, at: point)
    }

    func beginDividerDrag(_ divider: TileDivider, tab: String, at point: CGPoint) {
        stopAllMotion()
        var drag = DividerDrag(divider: divider, tab: tab, grab: divider.coordinate(of: point) - divider.centre,
                               anchor: point, started: CACurrentMediaTime())
        // Terminals follow the tile live, by whole cells (no lead: the
        // pointer has no known path ahead).
        for (id, view) in liveTileViews {
            guard let aware = view as? MotionAware else { continue }
            drag.resizesAtStart[id] = (aware.surfaceResizes, view.frame.size)
            aware.liveResizeWillStart(final: view.frame.size)
        }
        dividerDrag = drag
        divider.cursor.set()
        dividerCursorSet = true
        showDividerHint(divider, dragging: true)
        MotionStats.shared.started(kind: "divider_drag", setup: 0)
        MotionStats.shared.movingUntil = max(MotionStats.shared.movingUntil, CACurrentMediaTime() + 0.25)
        Perf.motionStarted = CACurrentMediaTime()
        wakeClock()
    }

    func dividerMouseDragged(_ event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        dragDivider(to: point)
    }

    /// Moves the dragged divider's centre to follow `point`.
    func dragDivider(to point: CGPoint) {
        guard var drag = dividerDrag else { return }
        drag.events += 1
        let ratio = drag.divider.ratio(at: drag.divider.coordinate(of: point) - drag.grab)
        MotionStats.shared.movingUntil = max(MotionStats.shared.movingUntil, CACurrentMediaTime() + 0.25)
        drag.divider.cursor.set()
        guard drag.lastSent.map({ abs($0 - ratio) > 1e-4 }) ?? true else {
            dividerDrag = drag
            return
        }
        drag.lastSent = ratio
        drag.commands += 1
        dividerDrag = drag
        onSetRatio?(drag.tab, drag.divider.path, ratio)
    }

    func dividerMouseUp(_ event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        dragDivider(to: point)
        endDividerDrag()
        // The hover follows the pointer again: the divider under it or,
        // released away from any, what lies below gets its cursor back.
        dividerHover(at: point)
    }

    /// Ends the drag: every tile at its exact frame, and each terminal one
    /// exact resize to it.
    func endDividerDrag() {
        guard let drag = dividerDrag else { return }
        dividerDrag = nil
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        needsLayout = true
        layoutSubtreeIfNeeded()
        var tiles: [[String: Any]] = []
        for (id, view) in liveTileViews {
            guard let aware = view as? MotionAware else { continue }
            let live = aware.surfaceResizes - (drag.resizesAtStart[id]?.count ?? aware.surfaceResizes)
            aware.liveResizeDidEnd()
            guard let start = drag.resizesAtStart[id] else { continue }
            let total = aware.surfaceResizes - start.count
            let changed = abs(view.frame.width - start.size.width) >= 0.5
                || abs(view.frame.height - start.size.height) >= 0.5
            // Live: at most one per `Motion.reflowInterval`, plus the exact one.
            let duration = CACurrentMediaTime() - drag.started
            let maximum = changed ? Int((duration / Motion.reflowInterval).rounded(.up)) + 2 : 0
            MotionStats.shared.resizes(tile: id, expected: changed ? 1 : 0, minimum: changed ? 1 : 0,
                                       maximum: maximum, actual: total, kind: "divider_drag")
            tiles.append(["tile": Short.id(id), "live": live, "mid_drag": drag.liveResizes[id] ?? 0,
                          "total": total, "changed": changed, "max": maximum])
        }
        CATransaction.commit()
        let duration = CACurrentMediaTime() - drag.started
        lastDividerDrag = [
            "path": drag.divider.path, "axis": drag.divider.axis.rawValue, "ratio": drag.lastSent ?? drag.divider.ratio,
            "commands": drag.commands, "events": drag.events, "duration_ms": Int(duration * 1000),
            "terminals": tiles,
        ]
        MotionStats.shared.movingUntil = CACurrentMediaTime()
        showDividerHint(hoveredDivider.flatMap { path in dividers.first { $0.path == path } }, dragging: false)
        // The pointer areas follow the divider (a refresh skips them mid-drag).
        updateDividerTracking()
        Motion.log("divider drag \(drag.divider.path.isEmpty ? "root" : drag.divider.path): "
            + "\(drag.commands) command(s) over \(Int(duration * 1000)) ms")
    }

    /// The core answered a drag step (or anything else changed mid-drag)
    /// with the tab's new tree: tiles take their frames at once, and the
    /// dragged divider is found again in case the tree changed shape.
    func layoutForDrag() {
        guard var drag = dividerDrag else { return }
        let all = dividers
        if let same = all.first(where: { $0.path == drag.divider.path && $0.axis == drag.divider.axis
            && $0.spans(drag.anchor) }) {
            drag.divider = same
        } else if let nearest = all.filter({ $0.axis == drag.divider.axis && $0.spans(drag.anchor) })
            .min(by: { abs($0.centre - drag.divider.centre) < abs($1.centre - drag.divider.centre) }) {
            drag.divider = nearest
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        needsLayout = true
        layoutSubtreeIfNeeded()
        CATransaction.commit()
        for (id, view) in liveTileViews {
            guard let aware = view as? MotionAware, let start = drag.resizesAtStart[id] else { continue }
            drag.liveResizes[id] = aware.surfaceResizes - start.count
        }
        dividerDrag = drag
        showDividerHint(drag.divider, dragging: true)
    }

    /// Every display frame while dragging: a terminal whose last live
    /// resize was throttled catches up with its tile once the pointer rests.
    func tickDividerDrag() {
        guard dividerDrag != nil else { return }
        for (_, view) in liveTileViews { (view as? MotionAware)?.liveResizeTick() }
    }

    // MARK: Hover

    /// Tracking areas over every divider's zone, for the cursor and the
    /// hint; rebuilt whenever the tab or the container's size changes.
    func updateDividerTracking() {
        let all = dividers
        // Most refreshes (a title, a header, focus) leave the dividers where
        // they are: keep the areas, AppKit needn't recompute them.
        let same = all.count == dividerAreas.count && zip(all, dividerAreas).allSatisfy { d, area in
            area.rect == d.zone(slack: Self.dividerSlack) && area.userInfo?["divider"] as? String == d.path
        }
        if !same {
            for area in dividerAreas { removeTrackingArea(area) }
            dividerAreas = all.map { d in
                NSTrackingArea(rect: d.zone(slack: Self.dividerSlack),
                               options: [.mouseEnteredAndExited, .mouseMoved, .activeInKeyWindow],
                               owner: self, userInfo: ["divider": d.path])
            }
            for area in dividerAreas { addTrackingArea(area) }
        }
        // The dividers moved under a resting pointer (a new tile, a resize
        // from the keyboard): hover what's under it now. Mid-motion nothing
        // takes the pointer, so the hover only follows its divider there.
        guard let hovered = hoveredDivider, dividerDrag == nil else { return }
        if !isMoving, let point = pointerLocation {
            dividerHover(at: point)
        } else if !all.contains(where: { $0.path == hovered }) {
            hoverDivider(nil, at: nil)
        }
        if hoveredDivider == hovered { showDividerHint(all.first { $0.path == hovered }, dragging: false) }
    }

    /// Automation's in-app pointer (`DividerHoverAutomation`), in window
    /// coordinates, in place of the real one; it answers nil when that
    /// pointer is off the window.
    static var pointerLocator: ((NSWindow) -> CGPoint?)?

    /// Where the pointer is, in this view's coordinates.
    var pointerLocation: CGPoint? {
        guard let window else { return nil }
        guard let locate = Self.pointerLocator else {
            return convert(window.mouseLocationOutsideOfEventStream, from: nil)
        }
        return locate(window).map { convert($0, from: nil) }
    }

    /// Whether a mouse-moved event at `point` is the dividers' to handle:
    /// inside a divider's zone, or just out of one the hover still shows.
    /// The point decides: a mouse-moved event has no tracking area (asking
    /// it for one raises, and AppKit drops the rest of the event).
    func dividerHoverWants(_ point: CGPoint) -> Bool {
        hoveredDivider != nil || dividerCursorSet || dividerAreas.contains { $0.rect.contains(point) }
    }

    /// The pointer moved over, within or off a divider's zone. Only the
    /// point counts, never whose area sent the event: zones overlap where
    /// dividers meet, and AppKit orders their entered/exited events as it
    /// likes.
    func dividerHover(at point: CGPoint) {
        guard dividerDrag == nil else { return }
        let under = hitTestBelowDividers(point)
        hoverDivider(divider(at: point, under: under), at: point)
    }

    /// Hovers `divider` (nil: none). Its resize cursor goes up; when it
    /// goes, what lies under the pointer gets its own cursor back (a
    /// terminal's shape from libghostty, the arrow elsewhere).
    func hoverDivider(_ divider: TileDivider?, at point: CGPoint?) {
        let was = hoveredDivider
        hoveredDivider = divider?.path
        if let divider {
            divider.cursor.set()
            dividerCursorSet = true
        } else if dividerCursorSet {
            dividerCursorSet = false
            cursorBelowDividers(at: point).set()
        }
        if was != hoveredDivider { showDividerHint(divider, dragging: false) }
    }

    /// The cursor of what lies under `point` once no divider holds the
    /// pointer: a view that keeps its own shape (`PointerShaped`), else
    /// the arrow.
    func cursorBelowDividers(at point: CGPoint?) -> NSCursor {
        guard let point, bounds.contains(point) else { return .arrow }
        var view = hitTestBelowDividers(point)
        while let v = view, v !== self {
            if let shaped = v as? PointerShaped { return shaped.pointerCursor }
            view = v.superview
        }
        return .arrow
    }

    /// Whether a tile's `view` may set its own cursor: not while a divider
    /// holds the pointer (hovered or dragged). `pointer` (window
    /// coordinates): where the pointer came into or left the view; the
    /// hover is checked against it first, since a divider's own exit can
    /// go missing (a flick out of an area rebuilt under the pointer).
    static func tileMaySetCursor(_ view: NSView, pointer: CGPoint? = nil) -> Bool {
        var next = view.superview
        while let v = next {
            if let container = v as? TileContainerView {
                if let pointer { container.dividerHover(at: container.convert(pointer, from: nil)) }
                return container.dividerDrag == nil && !container.dividerCursorSet
            }
            next = v.superview
        }
        return true
    }

    /// A thin line along the divider: muted on hover, the accent while
    /// dragging; gone otherwise.
    func showDividerHint(_ divider: TileDivider?, dragging: Bool) {
        CATransaction.begin()
        CATransaction.setAnimationDuration(Design.Motion.focus)
        guard let divider else {
            dividerHint.opacity = 0
            CATransaction.commit()
            return
        }
        let t = Self.hintThickness, inset = Self.hintInset
        let g = divider.gutter
        let frame = divider.axis == .horizontal
            ? CGRect(x: g.midX - t / 2, y: g.minY + inset, width: t, height: max(0, g.height - 2 * inset))
            : CGRect(x: g.minX + inset, y: g.midY - t / 2, width: max(0, g.width - 2 * inset), height: t)
        if dragging {
            // Follows the pointer frame by frame: no implicit motion.
            CATransaction.setDisableActions(true)
        }
        dividerHint.frame = frame
        dividerHint.cornerRadius = t / 2
        applyDividerHintColour(dragging: dragging)
        dividerHint.opacity = Float(dragging ? Design.Alpha.workingMarker : Design.Alpha.rowHover)
        CATransaction.commit()
    }

    /// The hint's colour for the current theme.
    func applyDividerHintColour(dragging: Bool? = nil) {
        // theme: re-read on every show, and applyTheme() calls this.
        let accent = dragging ?? (dividerDrag != nil)
        var colour: CGColor?
        effectiveAppearance.performAsCurrentDrawingAppearance {
            colour = (accent ? Design.Color.accent : Design.Color.textMuted).cgColor
        }
        dividerHint.backgroundColor = colour
    }
}
