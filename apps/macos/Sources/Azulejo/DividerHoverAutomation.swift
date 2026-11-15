import AppKit
import GhosttyKit

/// Automation for divider hover (`divider hover …` lines in `.azs` scripts):
/// a pointer that lives only in the app. Automation windows are never key,
/// so AppKit sends no tracking events of its own; this walks the window's
/// tracking areas as AppKit does and hands each owner the mouseEntered,
/// mouseExited and mouseMoved events the move would bring. Nothing goes
/// through the window server and the real pointer is untouched.
///
///     divider hover to TARGET          move there a step at a time
///     divider hover jump TARGET        in one step (a fast flick)
///     divider hover step POINTS        how far each step goes (default 1:
///                                      a slow hand, every point seen)
///     divider hover out                leave the window
///     divider hover order exit-first|enter-first|moved-first
///                                      the order a step's events go out in
///                                      (AppKit promises none)
///     divider hover moved on|off       off: owners get no mouseMoved (what
///                                      the old trackingArea check amounted to)
///     divider hover shape text|pointer|arrow
///                                      libghostty sets every terminal's
///                                      pointer shape (its mouse-shape action)
///
/// TARGET: `gutter PATH [ALONG]` (PATH's gutter, `-` the root, ALONG 0…1
/// from its top or left end, default 0.5), `tile N [FX FY]` (the Nth tile
/// in the tab's order, FX FY from its top left, default the centre), or
/// `off DX DY` (from where the pointer is, y up).
///
/// `expect` keys: `hover` (the hovered divider's path, `-` the root,
/// `none`), `cursor` (the app's cursor: arrow, ibeam, pointer, resize-lr,
/// resize-ud, other), `hint` (the divider whose gutter the hint line sits
/// on, `none` while it's hidden), `shape` (libghostty's shape for the
/// terminal under the pointer, as a cursor name; `none` off a terminal).
@MainActor
enum DividerHoverAutomation {
    private enum Order: String { case exitFirst = "exit-first", enterFirst = "enter-first", movedFirst = "moved-first" }

    /// Where the pointer is, in window coordinates; nil: outside the window.
    private static var pointer: CGPoint?
    /// The tracking areas the pointer is inside.
    private static var inside: Set<ObjectIdentifier> = []
    private static var order = Order.exitFirst
    private static var sendMoved = true
    /// Points per step for `to`.
    private static var step: CGFloat = 1

    static func run(_ parts: [String], tiles: TileContainerView, note: (String) -> Void) -> String? {
        guard let window = tiles.window else { return "no window" }
        let verb = parts.first ?? ""
        let rest = Array(parts.dropFirst())
        switch verb {
        case "to", "jump":
            guard let target = point(rest, tiles: tiles) else { return "no target \(rest.joined(separator: " "))" }
            let end = tiles.convert(target, to: nil)
            let start = pointer ?? end
            let n = verb == "jump" ? 1 : max(Int((hypot(end.x - start.x, end.y - start.y) / step).rounded(.up)), 1)
            var changes: [String] = []
            for i in 1...n {
                let t = CGFloat(i) / CGFloat(n)
                let before = cursorName()
                let p = CGPoint(x: start.x + (end.x - start.x) * t, y: start.y + (end.y - start.y) * t)
                move(to: p, in: window)
                if cursorName() != before { changes.append("\(cursorName()) at \(text(tiles.convert(p, from: nil)))") }
            }
            note("pointer at \(text(target)): hover \(hover(tiles)), cursor \(cursorName()), hint \(hint(tiles)), "
                + "shape \(shape(tiles)); on the way: \(changes.isEmpty ? "-" : changes.joined(separator: ", "))")
            return nil
        case "out":
            move(to: nil, in: window)
            return nil
        case "step":
            guard let points = rest.first.flatMap(Double.init), points > 0 else { return "step POINTS" }
            step = CGFloat(points)
            return nil
        case "order":
            guard let o = Order(rawValue: rest.first ?? "") else { return "order exit-first|enter-first|moved-first" }
            order = o
            return nil
        case "moved":
            sendMoved = rest.first != "off"
            return nil
        case "shape":
            let shape = switch rest.first ?? "" {
            case "text": GHOSTTY_MOUSE_SHAPE_TEXT
            case "pointer": GHOSTTY_MOUSE_SHAPE_POINTER
            default: GHOSTTY_MOUSE_SHAPE_DEFAULT
            }
            for (_, view) in tiles.liveTileViews {
                guard let surface = ((view as? TileFrame)?.content as? TerminalTileView)?.surfaceView
                    as? TerminalSurfaceView else { continue }
                surface.setMouseShape(shape)
            }
            return nil
        default:
            return "divider hover to|jump|out|step|order|moved|shape"
        }
    }

    static func state(_ tiles: TileContainerView) -> [String: String] {
        ["hover": hover(tiles), "cursor": cursorName(), "hint": hint(tiles), "shape": shape(tiles)]
    }

    /// Points the dividers' pointer lookups (`TileContainerView.pointerLocation`)
    /// at this pointer.
    static func install() {
        guard TileContainerView.pointerLocator == nil else { return }
        TileContainerView.pointerLocator = { _ in pointer }
    }

    /// A drag's pointer (`divider drag`) let go at `point` (window
    /// coordinates): the hover pointer is there now, inside the divider
    /// areas there, with no events (the mouse-up has had its say). Other
    /// areas there hear of it on the next step.
    static func place(at point: CGPoint, in window: NSWindow) {
        pointer = point
        inside = Set(trackingAreas(in: window)
            .filter { $0.area.owner is TileContainerView && $0.rect.contains(point) }.map(\.id))
    }

    // MARK: Pointer

    /// One pointer step: the areas left and entered hear of it, and the
    /// owners of mouse-moved areas the pointer is now in.
    private static func move(to point: CGPoint?, in window: NSWindow) {
        pointer = point
        let areas = trackingAreas(in: window)
        let now = Set(areas.filter { area in point.map { area.rect.contains($0) } ?? false }.map(\.id))
        // An area removed under the pointer sends nothing (as in AppKit).
        let live = Set(areas.map(\.id))
        let left = areas.filter { inside.contains($0.id) && !now.contains($0.id) }
        let entered = areas.filter { now.contains($0.id) && !inside.contains($0.id) }
        inside = now.intersection(live)
        let location = point ?? CGPoint(x: -100, y: -100)
        func exits() {
            for a in left { (a.area.owner as? NSResponder)?.mouseExited(with: event(.mouseExited, a.area, location, window)) }
        }
        func enters() {
            for a in entered {
                guard let owner = a.area.owner as? NSResponder else { continue }
                if a.area.options.contains(.mouseEnteredAndExited) {
                    owner.mouseEntered(with: event(.mouseEntered, a.area, location, window))
                }
                if a.area.options.contains(.cursorUpdate) {
                    owner.cursorUpdate(with: event(.cursorUpdate, a.area, location, window))
                }
            }
        }
        func moved() {
            guard sendMoved, point != nil else { return }
            var told: Set<ObjectIdentifier> = []
            for a in areas where now.contains(a.id) && a.area.options.contains(.mouseMoved) {
                guard let owner = a.area.owner as? NSResponder, told.insert(ObjectIdentifier(owner)).inserted
                else { continue }
                owner.mouseMoved(with: NSEvent.mouseEvent(
                    with: .mouseMoved, location: location, modifierFlags: [],
                    timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                    context: nil, eventNumber: 0, clickCount: 0, pressure: 0)!)
            }
        }
        switch order {
        case .exitFirst: exits(); enters(); moved()
        case .enterFirst: enters(); exits(); moved()
        case .movedFirst: moved(); enters(); exits()
        }
    }

    private struct Area {
        let area: NSTrackingArea
        let rect: CGRect
        var id: ObjectIdentifier { ObjectIdentifier(area) }
    }

    /// Every enabled tracking area in the window, in window coordinates.
    private static func trackingAreas(in window: NSWindow) -> [Area] {
        guard let root = window.contentView else { return [] }
        var out: [Area] = []
        func walk(_ view: NSView) {
            guard !view.isHidden else { return }
            for area in view.trackingAreas {
                let rect = area.options.contains(.inVisibleRect) ? view.visibleRect : area.rect
                out.append(Area(area: area, rect: view.convert(rect, to: nil)))
            }
            view.subviews.forEach(walk)
        }
        walk(root)
        return out
    }

    /// An entered, exited or cursor-update event for `area`, which it
    /// answers as `event.trackingArea` (AppKit's own carry the area that way).
    private static func event(_ type: NSEvent.EventType, _ area: NSTrackingArea, _ location: CGPoint,
                              _ window: NSWindow) -> NSEvent {
        NSEvent.enterExitEvent(
            with: type, location: location, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: window.windowNumber, context: nil, eventNumber: 0,
            trackingNumber: Int(bitPattern: Unmanaged.passUnretained(area).toOpaque()), userData: nil)!
    }

    // MARK: Targets

    /// TARGET in container coordinates.
    private static func point(_ rest: [String], tiles: TileContainerView) -> CGPoint? {
        let numbers = rest.dropFirst().compactMap { CGFloat(Double($0) ?? .nan) }.filter { !$0.isNaN }
        switch rest.first ?? "" {
        case "gutter":
            guard rest.count > 1 else { return nil }
            let path = rest[1] == "-" ? "" : rest[1]
            guard let d = tiles.dividers.first(where: { $0.path == path }) else { return nil }
            let along = rest.count > 2 ? CGFloat(Double(rest[2]) ?? 0.5) : 0.5
            let g = d.gutter
            return d.axis == .horizontal
                ? CGPoint(x: g.midX, y: g.maxY - g.height * along)
                : CGPoint(x: g.minX + g.width * along, y: g.midY)
        case "tile":
            guard let n = numbers.first, let tab = tiles.shownTab, Int(n) >= 1, Int(n) <= tab.order.count,
                  let f = tiles.visibleTileFrames()[tab.order[Int(n) - 1]] else { return nil }
            let fx = numbers.count > 2 ? numbers[1] : 0.5, fy = numbers.count > 2 ? numbers[2] : 0.5
            return CGPoint(x: f.minX + f.width * fx, y: f.maxY - f.height * fy)
        case "off":
            guard numbers.count >= 2, let pointer else { return nil }
            let at = tiles.convert(pointer, from: nil)
            return CGPoint(x: at.x + numbers[0], y: at.y + numbers[1])
        default:
            return nil
        }
    }

    // MARK: State

    private static func hover(_ tiles: TileContainerView) -> String {
        tiles.hoveredDivider.map { $0.isEmpty ? "-" : $0 } ?? "none"
    }

    /// libghostty's shape for the terminal under the pointer, as a cursor
    /// name; `none` off any terminal.
    private static func shape(_ tiles: TileContainerView) -> String {
        guard let pointer else { return "none" }
        var view = tiles.hitTestBelowDividers(tiles.convert(pointer, from: nil))
        while let v = view, v !== tiles {
            if let shaped = v as? PointerShaped { return cursorName(shaped.pointerCursor) }
            view = v.superview
        }
        return "none"
    }

    private static func cursorName(_ c: NSCursor = NSCursor.current) -> String {
        if c === NSCursor.arrow { return "arrow" }
        if c === NSCursor.iBeam { return "ibeam" }
        if c === NSCursor.pointingHand { return "pointer" }
        if c === NSCursor.resizeLeftRight { return "resize-lr" }
        if c === NSCursor.resizeUpDown { return "resize-ud" }
        return "other"
    }

    private static func hint(_ tiles: TileContainerView) -> String {
        guard tiles.dividerHint.opacity > 0 else { return "none" }
        let f = tiles.dividerHint.frame
        let centre = CGPoint(x: f.midX, y: f.midY)
        guard let d = tiles.dividers.first(where: { $0.gutter.insetBy(dx: -0.5, dy: -0.5).contains(centre) })
        else { return "elsewhere" }
        return d.path.isEmpty ? "-" : d.path
    }

    private static func text(_ p: CGPoint) -> String { String(format: "(%.1f,%.1f)", p.x, p.y) }
}
