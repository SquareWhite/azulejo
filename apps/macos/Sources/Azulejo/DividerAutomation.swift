import AppKit

/// Automation for divider drags (`divider …` lines in `.azs` scripts). Mouse
/// events are made in the app and handed straight to the view that hit-tests
/// at the point, as AppKit would deliver them: nothing goes through the
/// window server, no window is raised and nothing outside the app sees them.
///
///     divider list                   log every divider (path, axis, ratio, gutter)
///     divider drag PATH RATIO [MS] [HOLD]
///                                    press on PATH's gutter (`-` is the root),
///                                    move the pointer so the divider's centre
///                                    goes to RATIO over MS ms (default 300), one
///                                    event per display frame, rest HOLD ms
///                                    (default 0), release; waits for it
///     divider double PATH            double-click PATH's gutter (evens it out)
///     divider probe                  check hit-testing along every divider: the
///                                    gutter and a card's empty margin take the
///                                    pointer; terminal surfaces, web views,
///                                    headers keep every point up to their edges
///     divider check                  every tile at its frame, every terminal's
///                                    surface at its tile's exact size
///     divider stats NAME             the last drag's numbers → <out>/NAME.json
///
/// `expect` keys: `dividers` (how many), `divider.PATH` (that split's ratio,
/// `divider.-` for the root), `rect.N` (the Nth tile in the tab's order,
/// `x,y,wxh` in a 1000×1000 tab, top-left origin), `columns` (the tab's
/// column widths, left to right), `drag.commands`, `drag.mid` (terminals'
/// PTY resizes while the pointer was still down, summed), `drag.terminals`.
@MainActor
enum DividerAutomation {
    /// Runs a verb. Returns a problem, or nil, and how long to wait before
    /// the next line.
    static func run(_ arg: String, app: AppController?, out: URL,
                    note: @escaping (String) -> Void) -> (problem: String?, wait: TimeInterval) {
        guard let controller = app?.focusedController else { return ("no window", 0) }
        let tiles = controller.tiles
        let parts = arg.split(separator: " ").map(String.init)
        let path = parts.count > 1 ? (parts[1] == "-" ? "" : parts[1]) : ""
        switch parts.first ?? "" {
        case "list":
            for d in tiles.dividers {
                note("divider \(d.path.isEmpty ? "-" : d.path) \(d.axis.rawValue) "
                    + "\(String(format: "%.3f", d.ratio)) gutter \(text(d.gutter))")
            }
            return (nil, 0)
        case "drag":
            guard parts.count >= 3, let ratio = Double(parts[2]) else {
                return ("divider drag PATH RATIO [MS] [HOLD]", 0)
            }
            let ms = parts.count > 3 ? Double(parts[3]) ?? 300 : 300
            let hold = parts.count > 4 ? Double(parts[4]) ?? 0 : 0
            guard let divider = tiles.dividers.first(where: { $0.path == path }) else {
                return ("no divider at \(path.isEmpty ? "the root" : path)", 0)
            }
            if let problem = drag(tiles, divider, to: ratio, over: ms / 1000, hold: hold / 1000) {
                return (problem, 0)
            }
            return (nil, (ms + hold) / 1000 + 0.15)
        case "double":
            guard let divider = tiles.dividers.first(where: { $0.path == path }) else {
                return ("no divider at \(path.isEmpty ? "the root" : path)", 0)
            }
            let at = CGPoint(x: divider.gutter.midX, y: divider.gutter.midY)
            guard let target = target(tiles, at: at) else { return ("the gutter doesn't take the pointer", 0) }
            for clicks in 1...2 {
                target.mouseDown(with: event(.leftMouseDown, at: at, in: tiles, clicks: clicks))
                target.mouseUp(with: event(.leftMouseUp, at: at, in: tiles, clicks: clicks))
            }
            return (nil, 0)
        case "probe":
            return (probe(tiles, note: note), 0)
        case "check":
            return (check(tiles), 0)
        case "stats":
            let name = parts.count > 1 ? parts[1] : "drag-stats"
            let stats = tiles.lastDividerDrag
            if let data = try? JSONSerialization.data(withJSONObject: stats, options: [.prettyPrinted, .sortedKeys]) {
                try? data.write(to: out.appendingPathComponent("\(name).json"))
            }
            note("drag \(stats["path"] ?? "?"): \(stats["commands"] ?? 0) command(s), "
                + "\(stats["duration_ms"] ?? 0) ms, terminals \(stats["terminals"] ?? [])")
            return (nil, 0)
        default:
            return ("divider list|drag|double|probe|check|stats", 0)
        }
    }

    static func state(_ controller: WindowController) -> [String: String] {
        let tiles = controller.tiles
        var s: [String: String] = ["dividers": "\(tiles.dividers.count)"]
        guard let tab = tiles.shownTab else { return s }
        for d in tab.root.dividers(in: CGRect(x: 0, y: 0, width: 1000, height: 1000), gap: 0) {
            s["divider.\(d.path.isEmpty ? "-" : d.path)"] = String(format: "%.2f", d.ratio)
        }
        // Exact rects (no rounding at each split), top-left origin.
        var rects: [String: CGRect] = [:]
        func place(_ node: LayoutNode, _ r: CGRect) {
            switch node {
            case .leaf(let id): rects[id] = r
            case .split(let axis, let ratio, let a, let b):
                let k = CGFloat(ratio)
                if axis == .horizontal {
                    place(a, CGRect(x: r.minX, y: r.minY, width: r.width * k, height: r.height))
                    place(b, CGRect(x: r.minX + r.width * k, y: r.minY, width: r.width * (1 - k), height: r.height))
                } else {
                    place(a, CGRect(x: r.minX, y: r.minY, width: r.width, height: r.height * k))
                    place(b, CGRect(x: r.minX, y: r.minY + r.height * k, width: r.width, height: r.height * (1 - k)))
                }
            }
        }
        place(tab.root, CGRect(x: 0, y: 0, width: 1000, height: 1000))
        func n(_ v: CGFloat) -> Int { Int(v.rounded()) }
        for (i, id) in tab.order.enumerated() {
            guard let f = rects[id] else { continue }
            s["rect.\(i + 1)"] = "\(n(f.minX)),\(n(f.minY)),\(n(f.width))x\(n(f.height))"
        }
        // Columns: each distinct left edge and width, left to right, as a
        // share of the tab ("0.25,0.50,0.25").
        var columns: [CGFloat: CGFloat] = [:]
        for r in rects.values { columns[(r.minX).rounded()] = r.width }
        s["columns"] = columns.sorted { $0.key < $1.key }
            .map { String(format: "%.2f", $0.value / 1000) }.joined(separator: ",")
        let drag = tiles.lastDividerDrag
        s["drag.commands"] = "\(drag["commands"] ?? 0)"
        let terminals = drag["terminals"] as? [[String: Any]] ?? []
        s["drag.terminals"] = "\(terminals.count)"
        s["drag.mid"] = "\(terminals.reduce(0) { $0 + ($1["mid_drag"] as? Int ?? 0) })"
        return s
    }

    // MARK: Drag

    /// The view a click at `point` reaches; nil unless it's the container
    /// (the divider takes it).
    private static func target(_ tiles: TileContainerView, at point: CGPoint) -> NSView? {
        guard let superview = tiles.superview,
              let hit = tiles.hitTest(tiles.convert(point, to: superview)), hit === tiles
        else { return nil }
        return hit
    }

    private static func event(_ type: NSEvent.EventType, at point: CGPoint, in view: NSView,
                              clicks: Int = 1) -> NSEvent {
        NSEvent.mouseEvent(
            with: type, location: view.convert(point, to: nil), modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: view.window?.windowNumber ?? 0,
            context: nil, eventNumber: 0, clickCount: clicks, pressure: type == .leftMouseUp ? 0 : 1)!
    }

    private static func drag(_ tiles: TileContainerView, _ divider: TileDivider, to ratio: Double,
                             over duration: TimeInterval, hold: TimeInterval) -> String? {
        // Take hold a little off the centre line, as a hand would.
        let offset: CGFloat = 1.5
        let start = divider.axis == .horizontal
            ? CGPoint(x: divider.gutter.midX + offset, y: divider.gutter.midY)
            : CGPoint(x: divider.gutter.midX, y: divider.gutter.midY - offset)
        guard let target = target(tiles, at: start) else {
            return "the gutter at \(text(divider.gutter)) doesn't take the pointer"
        }
        // Where the centre line goes for `ratio`, as frames(in:gap:) lays it out.
        let a = divider.area, gap = divider.gap
        let end: CGPoint
        switch divider.axis {
        case .horizontal:
            let x = a.minX + ((a.width - gap) * CGFloat(ratio)).rounded() + gap / 2 + offset
            end = CGPoint(x: x, y: start.y)
        case .vertical:
            let y = a.maxY - ((a.height - gap) * CGFloat(ratio)).rounded() - gap / 2 - offset
            end = CGPoint(x: start.x, y: y)
        }
        target.mouseDown(with: event(.leftMouseDown, at: start, in: tiles))
        guard tiles.dividerDrag != nil else { return "mouse-down didn't start a drag" }
        let frame = 1 / Double(Motion.refreshRate)
        let began = CACurrentMediaTime()
        // One pointer event per display frame, as a trackpad sends them.
        func step() {
            let t = min((CACurrentMediaTime() - began) / max(duration, frame), 1)
            let p = CGPoint(x: start.x + (end.x - start.x) * t, y: start.y + (end.y - start.y) * t)
            if t < 1 || CACurrentMediaTime() - began < duration + hold {
                target.mouseDragged(with: event(.leftMouseDragged, at: p, in: tiles))
                DispatchQueue.main.asyncAfter(deadline: .now() + frame) { step() }
                return
            }
            target.mouseUp(with: event(.leftMouseUp, at: end, in: tiles))
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + frame) { step() }
        return nil
    }

    // MARK: Checks

    /// Hit-testing along every divider. Returns a problem, or nil.
    private static func probe(_ tiles: TileContainerView, note: (String) -> Void) -> String? {
        var problems: [String] = []
        var claimed = 0, kept = 0, edges = 0
        guard let superview = tiles.superview else { return "no superview" }
        func hit(_ p: CGPoint) -> NSView? { tiles.hitTest(tiles.convert(p, to: superview)) }
        for d in tiles.dividers {
            let name = d.path.isEmpty ? "root" : d.path
            let centre = CGPoint(x: d.gutter.midX, y: d.gutter.midY)
            if hit(centre) !== tiles { problems.append("\(name): the gutter's centre isn't the divider's") }
            for along in [0.2, 0.5, 0.8] as [CGFloat] {
                for side in [-1, 1] as [CGFloat] {
                    for depth in [0.5, 1, Self.slack - 0.5, Self.slack + 1, 6, 12] as [CGFloat] {
                        let p: CGPoint
                        switch d.axis {
                        case .horizontal:
                            let x = side < 0 ? d.gutter.minX - depth : d.gutter.maxX + depth
                            p = CGPoint(x: x, y: d.gutter.minY + d.gutter.height * along)
                        case .vertical:
                            let y = side < 0 ? d.gutter.minY - depth : d.gutter.maxY + depth
                            p = CGPoint(x: d.gutter.minX + d.gutter.width * along, y: y)
                        }
                        guard tiles.bounds.contains(p) else { continue }
                        let under = tiles.hitTestBelowDividers(p)
                        let got = hit(p)
                        // Which divider takes it, if any (a nested one's
                        // gutter can lie across this one's margin).
                        let owner = tiles.divider(at: p, under: under)?.path
                        let inert = under == nil || under === tiles || under is TileFrame
                        if !inert {
                            // Something that takes clicks: it keeps them.
                            if got !== under {
                                problems.append("\(name): \(describe(under)) at \(text(p)) lost its click")
                            }
                            kept += 1
                        } else if depth < Self.slack {
                            if got !== tiles || owner == nil {
                                problems.append("\(name): margin at \(text(p)) isn't a divider's")
                            }
                            claimed += 1
                        } else if owner == d.path {
                            problems.append("\(name): \(text(p)), past the slack, went to the divider")
                        }
                    }
                }
            }
        }
        // Every terminal's surface, half a point inside each edge, and
        // every header: never the divider's.
        for (id, view) in tiles.liveTileViews {
            guard let frame = view as? TileFrame,
                  let surface = (frame.content as? TerminalTileView)?.surfaceView else { continue }
            let r = surface.convert(surface.bounds, to: tiles)
            for p in [CGPoint(x: r.minX + 0.5, y: r.midY), CGPoint(x: r.maxX - 0.5, y: r.midY),
                      CGPoint(x: r.midX, y: r.minY + 0.5), CGPoint(x: r.midX, y: r.maxY - 0.5)] {
                let got = hit(p)
                if got == nil || !(got === surface || got!.isDescendant(of: surface)) {
                    problems.append("terminal \(Short.id(id)): \(text(p)) reached \(describe(got)), not its surface")
                }
                edges += 1
            }
            let header = frame.header.convert(frame.header.bounds, to: tiles)
            if header.height > 0, hit(CGPoint(x: header.midX, y: header.maxY - 0.5)) === tiles {
                problems.append("tile \(Short.id(id)): its header's top edge went to a divider")
            }
        }
        note("probe: \(claimed) margin point(s) taken, \(kept) content point(s) kept, \(edges) surface edge(s)")
        return problems.isEmpty ? nil : problems.prefix(6).joined(separator: "; ")
    }

    private static let slack = TileContainerView.dividerSlack

    /// Every tile at its frame from the core's tree; every terminal's
    /// surface at its tile's exact size (the final resize landed).
    private static func check(_ tiles: TileContainerView) -> String? {
        guard tiles.dividerDrag == nil else { return "a drag is still under way" }
        let frames = tiles.visibleTileFrames()
        var problems: [String] = []
        for (id, want) in frames {
            guard let view = tiles.tileView(id) else { continue }
            if view.frame != want {
                problems.append("tile \(Short.id(id)) at \(text(view.frame)), not \(text(want))")
            }
            if let terminal = (view as? TileFrame)?.content as? TerminalTileView, !terminal.surfaceFitsTile {
                problems.append("terminal \(Short.id(id)): its surface isn't at the tile's size")
            }
        }
        return problems.isEmpty ? nil : problems.joined(separator: "; ")
    }

    private static func describe(_ view: NSView?) -> String {
        view.map { "\(type(of: $0))" } ?? "nothing"
    }

    private static func text(_ r: CGRect) -> String {
        "\(Int(r.minX)),\(Int(r.minY)),\(Int(r.width))x\(Int(r.height))"
    }

    private static func text(_ p: CGPoint) -> String {
        String(format: "(%.1f,%.1f)", p.x, p.y)
    }
}
