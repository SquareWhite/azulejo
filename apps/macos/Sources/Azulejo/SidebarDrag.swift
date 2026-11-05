import AppKit

/// Dragging a workspace row to reorder the window's workspaces, in the full
/// sidebar and in the rail. The order is the core's (`workspace.move`): the
/// sidebar, ⌘1…⌘8 and ⌘9 follow it, it's saved with the session, and the
/// API lists workspaces in it (ADR 0013).
///
/// A press that moves `dragThreshold` points lifts the row: a copy of it,
/// raised a little with a shadow, follows the pointer while the row's own
/// place stays empty. The other rows of its machine group slide aside to
/// open a slot where it would land, outlined in the accent colour (a row
/// never leaves its group: the machine is where its terminals run). A drop
/// in or near the sidebar card moves the workspace there and the copy
/// settles into its new place; a drop farther out, or Esc, cancels: the rows
/// slide back and the copy returns to where it came from. All of it in the
/// window, with in-app events only: no drag session, no pasteboard.
///
/// While a drag runs, its host holds back refreshes (the rows it moves must
/// stay), and shows the latest one when the drag ends.
@MainActor
protocol WorkspaceDragRow: NSView {
    /// The workspace the row shows.
    var workspaceID: String { get }
    /// Its machine group: a row lands only among its own group's rows.
    var groupID: String { get }
    /// What automation calls it (`drag.rows`): the name, or the rail's number.
    var dragName: String { get }
}

/// What holds draggable rows: the full sidebar or the rail.
@MainActor
protocol WorkspaceDragHost: AnyObject {
    /// Every draggable row, top down.
    var dragRows: [WorkspaceDragRow] { get }
    /// The window's workspaces in order, the latest the host was given.
    var dragOrder: [String] { get }
    /// Where a drop lands, in window coordinates; past it, a drop cancels.
    var dropArea: NSRect { get }
    /// Moves a workspace to `index` in its window (`workspace.move`); the
    /// rows are rebuilt in the new order before it returns.
    func dragMove(_ workspace: String, to index: Int)
    /// The drag is over: show what came in meanwhile.
    func dragEnded()
}

@MainActor
final class WorkspaceDrag {
    /// The drag in progress (one at a time): Esc goes to it first.
    private(set) static var current: WorkspaceDrag?
    /// Automation: how the last drag ended ("moved 1→3", "cancelled", "none").
    private(set) static var lastOutcome = "none"

    let workspaceID: String
    private weak var host: WorkspaceDragHost?
    private let row: WorkspaceDragRow
    /// Its group's rows, top down, `row` among them.
    private let rows: [WorkspaceDragRow]
    private let origin: Int
    /// The rows' frames in their superview when the drag began.
    private let frames: [NSRect]
    private let flipped: Bool
    /// The row's frame and the pointer when the drag began, in window points.
    private let startFrame: NSRect
    private let startPointer: NSPoint
    private let lift: DragLiftView
    private let slotView = DropSlotView()
    /// Where it would land: an index into `rows` once it has left its place.
    private(set) var slot: Int
    private(set) var pointer: NSPoint
    /// The pointer is where a drop lands (`dropArea`).
    private(set) var inside = true
    private(set) var finished = false

    private init(row: WorkspaceDragRow, rows: [WorkspaceDragRow], host: WorkspaceDragHost, pointer: NSPoint,
                 window: NSWindow) {
        self.row = row
        self.rows = rows
        self.host = host
        workspaceID = row.workspaceID
        origin = rows.firstIndex { $0 === row } ?? 0
        slot = origin
        frames = rows.map(\.frame)
        flipped = row.superview?.isFlipped ?? false
        startFrame = row.convert(row.bounds, to: nil)
        startPointer = pointer
        self.pointer = pointer
        lift = DragLiftView(snapshotOf: row)
    }

    /// Lifts `row`, pressed at `pointer` (window points). Nil when it can't
    /// move: another drag runs, or it's alone in its group.
    static func begin(_ row: WorkspaceDragRow, at pointer: NSPoint) -> WorkspaceDrag? {
        guard current == nil, let window = row.window, let content = window.contentView,
              let superview = row.superview,
              let host = sequence(first: superview, next: \.superview).first(where: { $0 is WorkspaceDragHost })
                as? WorkspaceDragHost
        else { return nil }
        let rows = host.dragRows.filter { $0.groupID == row.groupID && $0.superview === superview }
        guard rows.count > 1, rows.contains(where: { $0 === row }) else { return nil }
        let drag = WorkspaceDrag(row: row, rows: rows, host: host, pointer: pointer, window: window)
        current = drag
        drag.start(in: content, superview: superview)
        return drag
    }

    private func start(in content: NSView, superview: NSView) {
        lift.frame = content.convert(startFrame, from: nil)
        content.addSubview(lift)
        lift.raise(animated: Self.animates)
        slotView.frame = frames[origin]
        superview.addSubview(slotView, positioned: .below, relativeTo: rows.first)
        row.alphaValue = 0
    }

    // MARK: Moving

    /// The pointer moved (window points): the copy follows; the rows make
    /// room where it would land, or close up while it's out of reach.
    func move(to pointer: NSPoint) {
        guard !finished, let host, let content = lift.superview else { return }
        self.pointer = pointer
        let frame = startFrame.offsetBy(dx: pointer.x - startPointer.x, dy: pointer.y - startPointer.y)
        lift.frame = content.convert(frame, from: nil)
        let inside = host.dropArea.contains(pointer)
        let slot = inside ? slot(for: frame) : origin
        guard slot != self.slot || inside != self.inside else { return }
        self.slot = slot
        self.inside = inside
        arrange(animated: Self.animates)
    }

    /// The place the copy's middle is over, among the other rows' middles.
    private func slot(for liftFrame: NSRect) -> Int {
        guard let superview = row.superview else { return origin }
        let middle = superview.convert(NSPoint(x: liftFrame.midX, y: liftFrame.midY), from: nil).y
        let down = { (y: CGFloat) in self.flipped ? y : -y }
        return frames.indices.filter { $0 != origin && down(frames[$0].midY) < down(middle) }.count
    }

    /// Rows in the order they show now: the others, with the dragged one at `slot`.
    private var shownOrder: [Int] {
        var order = rows.indices.filter { $0 != origin }
        order.insert(origin, at: min(slot, order.count))
        return order
    }

    /// Slides every other row to its place in `shownOrder`, and the slot to
    /// the dragged row's.
    private func arrange(animated: Bool) {
        for (place, i) in shownOrder.enumerated() {
            if i == origin {
                slotView.move(to: frames[place], shown: inside, animated: animated)
            } else {
                Self.slide(rows[i], by: frames[place].minY - frames[i].minY, animated: animated)
            }
        }
    }

    // MARK: Ending

    /// The button came up: move the workspace to its slot, or go back.
    func drop() {
        guard !finished else { return }
        finish()
        guard inside, slot != origin, let host else { return settleBack() }
        // Before or after a neighbour, by ID: the window's order is the one
        // the host has now, not the one the drag started from.
        let others = rows.indices.filter { $0 != origin }.map { rows[$0].workspaceID }
        var order = host.dragOrder.filter { $0 != workspaceID }
        let index: Int
        if slot < others.count, let before = order.firstIndex(of: others[slot]) {
            index = before
        } else if let after = others.last.flatMap({ order.firstIndex(of: $0) }) {
            index = after + 1
        } else {
            return settleBack()
        }
        order.insert(workspaceID, at: index)
        host.dragMove(workspaceID, to: index)
        // The host rebuilt its rows: land on the new one. If it didn't (the
        // move failed), go back.
        guard !rows.contains(where: { $0.window != nil && $0.superview != nil }),
              let landed = host.dragRows.first(where: { $0.workspaceID == workspaceID }),
              landed.window != nil
        else { return settleBack() }
        Self.lastOutcome = "moved \(origin + 1)→\(slot + 1)"
        slotView.removeFromSuperview()
        landed.alphaValue = 0
        let target = landed.convert(landed.bounds, to: nil)
        land(at: target) { [weak landed, weak host] in
            landed?.alphaValue = 1
            host?.dragEnded()
        }
    }

    /// Esc, or a drop out of reach: everything goes back.
    func cancel() {
        guard !finished else { return }
        finish()
        settleBack()
    }

    /// Whether `host` holds the dragged row (it holds back refreshes).
    func hosted(by host: AnyObject) -> Bool { self.host === host }

    private func finish() {
        finished = true
        if Self.current === self { Self.current = nil }
    }

    private func settleBack() {
        Self.lastOutcome = "cancelled"
        slot = origin
        inside = false
        arrange(animated: Self.animates)
        land(at: startFrame) { [weak self] in
            guard let self else { return }
            self.row.alphaValue = 1
            self.slotView.removeFromSuperview()
            self.host?.dragEnded()
        }
    }

    /// What runs once the copy has landed.
    private var landed: (() -> Void)?

    /// The copy settles at `target` (window points) and goes.
    private func land(at target: NSRect, then done: @escaping () -> Void) {
        guard let content = lift.superview else { return done() }
        let frame = content.convert(target, from: nil)
        landed = done
        guard Self.animates else {
            lift.frame = frame
            lift.settle(animated: false)
            return finishLanding()
        }
        lift.settle(animated: true)
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = Self.duration
            ctx.timingFunction = Motion.timing
            ctx.allowsImplicitAnimation = true
            lift.animator().frame = frame
        }, completionHandler: { MainActor.assumeIsolated { self.finishLanding() } })
    }

    private func finishLanding() {
        lift.removeFromSuperview()
        let done = landed
        landed = nil
        done?()
    }

    // MARK: Motion

    /// Lifting, sliding aside and landing: `[animation]` on, unless Reduce
    /// Motion is.
    static var animates: Bool { Motion.active }
    static var duration: CFTimeInterval {
        Motion.durationOverride.map { $0 * 0.75 } ?? Design.Motion.drag
    }

    private static let slideKey = "azulejo.drag.slide"

    /// Moves a row `dy` points from its own place (in its superview's
    /// coordinates) with a layer transform, so Auto Layout never sees it.
    static func slide(_ view: NSView, by dy: CGFloat, animated: Bool) {
        guard let layer = view.layer else { return }
        let to = CATransform3DMakeTranslation(0, dy, 0)
        let from = layer.presentation()?.transform ?? layer.transform
        guard !CATransform3DEqualToTransform(to, layer.transform) else { return }
        layer.transform = to
        layer.removeAnimation(forKey: slideKey)
        guard animated else { return }
        let move = CABasicAnimation(keyPath: "transform")
        move.fromValue = from
        move.toValue = to
        move.duration = duration
        move.timingFunction = Motion.timing
        layer.add(move, forKey: slideKey)
    }
}

// MARK: - The lifted copy and the slot

/// The lifted copy of a row: its picture on a raised card with a shadow.
/// The view follows the pointer; its card scales, so the frame never has a
/// transform on it.
@MainActor
private final class DragLiftView: NSView, ThemeAware {
    private let card = NSView()
    private static let lifted: CGFloat = 1.04

    init(snapshotOf row: NSView) {
        super.init(frame: row.bounds)
        wantsLayer = true
        card.wantsLayer = true
        card.frame = bounds
        card.autoresizingMask = [.width, .height]
        addSubview(card)
        let image = row.bitmapImageRepForCachingDisplay(in: row.bounds).map { rep -> NSImage in
            row.cacheDisplay(in: row.bounds, to: rep)
            let image = NSImage(size: row.bounds.size)
            image.addRepresentation(rep)
            return image
        }
        let picture = CALayer()
        picture.contents = image
        picture.frame = card.bounds
        picture.autoresizingMask = [.layerWidthSizable, .layerHeightSizable]
        picture.contentsGravity = .resize
        card.layer?.addSublayer(picture)
        card.layer?.cornerRadius = Design.Radius.row + 1
        card.layer?.cornerCurve = .continuous
        card.layer?.shadowRadius = 8
        card.layer?.shadowOffset = CGSize(width: 0, height: -3)
        card.layer?.shadowOpacity = 0
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { false }
    /// Clicks go through to what's below (never happens mid-drag anyway).
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        card.layer?.backgroundColor = Design.Color.control.cgColor
        card.layer?.borderColor = Design.Color.border.cgColor
        card.layer?.borderWidth = Design.Border.hairline
        card.layer?.shadowColor = Design.Color.shadow.cgColor
    }

    /// Up: a little larger, with its shadow.
    func raise(animated: Bool) {
        animate(scale: Self.lifted, shadow: Float(Design.Alpha.dragShadow), animated: animated)
    }

    /// Down again: its own size, no shadow.
    func settle(animated: Bool) {
        animate(scale: 1, shadow: 0, animated: animated)
    }

    private func animate(scale: CGFloat, shadow: Float, animated: Bool) {
        guard let layer = card.layer else { return }
        // Round the card's middle, whatever its anchor point.
        let size = card.bounds.size
        let pivot = CGPoint(x: (0.5 - layer.anchorPoint.x) * size.width, y: (0.5 - layer.anchorPoint.y) * size.height)
        var to = CATransform3DMakeTranslation(pivot.x, pivot.y, 0)
        to = CATransform3DScale(to, scale, scale, 1)
        to = CATransform3DTranslate(to, -pivot.x, -pivot.y, 0)
        let fromTransform = layer.presentation()?.transform ?? layer.transform
        let fromShadow = layer.presentation()?.shadowOpacity ?? layer.shadowOpacity
        layer.transform = to
        layer.shadowOpacity = shadow
        guard animated else { return }
        let grow = CABasicAnimation(keyPath: "transform")
        grow.fromValue = fromTransform
        grow.toValue = to
        let fade = CABasicAnimation(keyPath: "shadowOpacity")
        fade.fromValue = fromShadow
        fade.toValue = shadow
        let group = CAAnimationGroup()
        group.animations = [grow, fade]
        group.duration = WorkspaceDrag.duration
        group.timingFunction = Motion.timing
        layer.add(group, forKey: "azulejo.drag.lift")
    }
}

/// Where the dragged row would land: an accent outline on a faint accent
/// fill, in the rows' own place.
@MainActor
private final class DropSlotView: NSView, ThemeAware {
    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.row + 1
        layer?.cornerCurve = .continuous
        layer?.borderWidth = Design.Border.hairline * 1.5
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
    }

    func applyTheme() {
        layer?.backgroundColor = Design.Color.accent.withAlphaComponent(Design.Alpha.dropSlot).cgColor
        layer?.borderColor = Design.Color.accent.withAlphaComponent(Design.Alpha.dropSlotBorder).cgColor
    }

    /// Automation: whether it shows.
    var shown: Bool { superview != nil && alphaValue > 0 }

    func move(to frame: NSRect, shown: Bool, animated: Bool) {
        guard animated else {
            self.frame = frame
            alphaValue = shown ? 1 : 0
            return
        }
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = WorkspaceDrag.duration
            ctx.timingFunction = Motion.timing
            ctx.allowsImplicitAnimation = true
            animator().frame = frame
            animator().alphaValue = shown ? 1 : 0
        }
    }
}

// MARK: - Presses

/// A workspace row's mouse press: a click, or once the pointer has moved
/// `dragThreshold` points, a drag.
@MainActor
struct RowPress {
    private var start: NSPoint?
    private var drag: WorkspaceDrag?

    mutating func down(_ event: NSEvent) {
        start = event.locationInWindow
        drag = nil
    }

    mutating func dragged(_ event: NSEvent, row: WorkspaceDragRow) {
        guard let start else { return }
        let p = event.locationInWindow
        if drag == nil {
            guard hypot(p.x - start.x, p.y - start.y) >= Design.Space.dragThreshold else { return }
            drag = WorkspaceDrag.begin(row, at: start)
            // Can't move (alone in its group): no drag, and no click either.
            if drag == nil { self.start = nil; return }
        }
        drag?.move(to: p)
    }

    /// True when the press was a click on `row`.
    mutating func up(_ event: NSEvent, in row: NSView) -> Bool {
        defer { start = nil; drag = nil }
        if let drag {
            drag.drop()
            return false
        }
        guard start != nil else { return false }
        return row.bounds.contains(row.convert(event.locationInWindow, from: nil))
    }
}

// MARK: - Automation

/// `sidebar drag N` presses the Nth workspace row of the sidebar or rail
/// on show and moves it far enough to lift it; `sidebar drag to N` moves it
/// over the Nth row's place, `sidebar drag out` out of the card's reach,
/// `sidebar drag drop` lets go. Esc (`key esc`) cancels. `sidebar drag
/// click N` presses and lets go without moving: a click. The mouse events
/// go straight to the row's own handlers, not through the window: nothing
/// is activated and the real pointer is never involved.
/// State: `drag.active`, `drag.slot` (1-based in the group, `out` past
/// the card), `drag.rows` (the group as it shows, the dragged row in
/// brackets at its slot), `drag.indicator` (`shown` or `hidden`),
/// `drag.last` (`moved 1→3`, `cancelled`, `none`).
@MainActor
enum DragAutomation {
    private static weak var pressed: NSView?
    private static var pointer = NSPoint.zero

    static func run(_ args: [String], host: (NSView & WorkspaceDragHost)?) -> String? {
        guard let host, let window = host.window else { return "no sidebar or rail on show" }
        switch args.first ?? "" {
        case "to":
            guard WorkspaceDrag.current != nil, let row = pressed as? WorkspaceDragRow else { return "no drag" }
            guard let n = args.dropFirst().first.flatMap(Int.init), host.dragRows.indices.contains(n - 1)
            else { return "no row \(args.dropFirst().first ?? "")" }
            // The row was pressed in its middle, so the copy's middle follows
            // the pointer: put it over that row's own place (frame without
            // its slide: rows moving aside don't move the target).
            let target = host.dragRows[n - 1]
            guard target.groupID == row.groupID, let superview = target.superview else {
                return "row \(n) is in another group"
            }
            // A quarter row past its middle, the way the row travels, so it
            // takes that place rather than the one next to it.
            let place = target.frame
            let from = host.dragRows.firstIndex { $0 === row } ?? 0
            let way: CGFloat = n - 1 > from ? 1 : n - 1 < from ? -1 : 0
            let down: CGFloat = superview.isFlipped ? 1 : -1
            let y = place.midY + way * down * place.height / 4
            let middle = superview.convert(NSPoint(x: place.midX, y: y), to: nil)
            drag(row, to: NSPoint(x: pointer.x, y: middle.y), window: window)
        case "out":
            guard WorkspaceDrag.current != nil, let row = pressed as? WorkspaceDragRow else { return "no drag" }
            drag(row, to: NSPoint(x: host.dropArea.maxX + 160, y: pointer.y), window: window)
        case "drop":
            guard let row = pressed else { return "nothing pressed" }
            row.mouseUp(with: event(.leftMouseUp, at: pointer, window: window))
            pressed = nil
        case "click":
            guard let n = args.dropFirst().first.flatMap(Int.init), host.dragRows.indices.contains(n - 1)
            else { return "no row \(args.dropFirst().first ?? "")" }
            let row = host.dragRows[n - 1]
            let middle = row.convert(NSPoint(x: row.bounds.midX, y: row.bounds.midY), to: nil)
            row.mouseDown(with: event(.leftMouseDown, at: middle, window: window))
            // A hand's wobble, under the threshold: still a click.
            row.mouseDragged(with: event(.leftMouseDragged, at: NSPoint(x: middle.x + 1, y: middle.y), window: window))
            row.mouseUp(with: event(.leftMouseUp, at: NSPoint(x: middle.x + 1, y: middle.y), window: window))
        default:
            guard let n = args.first.flatMap(Int.init), host.dragRows.indices.contains(n - 1)
            else { return "sidebar drag takes N, to N, out or drop" }
            guard WorkspaceDrag.current == nil else { return "a drag is running" }
            let row = host.dragRows[n - 1]
            let middle = row.convert(NSPoint(x: row.bounds.midX, y: row.bounds.midY), to: nil)
            pressed = row
            pointer = middle
            row.mouseDown(with: event(.leftMouseDown, at: middle, window: window))
            // Sideways, past the threshold: lifted, still over its own place.
            drag(row, to: NSPoint(x: middle.x + Design.Space.dragThreshold + 2, y: middle.y), window: window)
            guard WorkspaceDrag.current != nil else { return "row \(n) didn't lift" }
        }
        return nil
    }

    /// mouseDragged in a few steps from the pointer to `to`, as a hand would.
    private static func drag(_ row: NSView, to: NSPoint, window: NSWindow) {
        let from = pointer
        let steps = 6
        for i in 1...steps {
            let t = CGFloat(i) / CGFloat(steps)
            let p = NSPoint(x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t)
            row.mouseDragged(with: event(.leftMouseDragged, at: p, window: window))
        }
        pointer = to
    }

    private static func event(_ type: NSEvent.EventType, at point: NSPoint, window: NSWindow) -> NSEvent {
        NSEvent.mouseEvent(
            with: type, location: point, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
    }

    static func state(host: (NSView & WorkspaceDragHost)?) -> [String: String] {
        var s = ["drag.last": WorkspaceDrag.lastOutcome]
        guard let drag = WorkspaceDrag.current, let host else {
            s["drag.active"] = "no"
            return s
        }
        s["drag.active"] = "yes"
        s["drag.slot"] = drag.inside ? "\(drag.slot + 1)" : "out"
        let group = host.dragRows.filter { row in
            host.dragRows.first { $0.workspaceID == drag.workspaceID }.map { $0.groupID == row.groupID } ?? false
        }
        // Each row where it shows now (its place plus its slide).
        let shown = group.map { row -> (CGFloat, String) in
            let y = row.frame.minY + (row.layer?.transform.m42 ?? 0)
            let down = row.superview?.isFlipped == true ? y : -y
            return (down, row.workspaceID == drag.workspaceID ? "[\(row.dragName)]" : row.dragName)
        }
        var others = shown.filter { !$0.1.hasPrefix("[") }.sorted { $0.0 < $1.0 }.map(\.1)
        if let lifted = shown.first(where: { $0.1.hasPrefix("[") })?.1, drag.inside {
            others.insert(lifted, at: min(drag.slot, others.count))
        }
        s["drag.rows"] = others.joined(separator: "|")
        s["drag.indicator"] = drag.inside ? "shown" : "hidden"
        return s
    }
}
