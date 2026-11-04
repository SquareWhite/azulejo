import AppKit
import QuartzCore

/// Lays out the active tab's tiles from the core's tree. Keeps one view per
/// tile for every tab in the window, so switching tabs or rearranging tiles
/// never recreates a tile (a terminal keeps its surface, a browser its page).
///
/// Layout changes animate (F4, `Motion`): each tile jumps to its final frame
/// at the start — one PTY resize per terminal, hidden under its snapshot
/// (ADR 0004) — and a transform carries it there visually. A change that
/// arrives mid-motion starts from where every tile appears on screen, so
/// holding a key never queues up motion.
@MainActor
final class TileContainerView: NSView {
    static let gap = Design.Space.gutter
    static let inset = Design.Space.tileInset
    private static let motionKey = "azj.motion"

    private var views: [String: any TileView] = [:]
    private var tab: TabModel?
    /// Where the active tab sits, for the direction of a slide.
    private var position: (workspace: Int, tab: Int) = (0, 0)
    private let ring = FocusRingView()

    /// Views fading out: closed tiles (removed for good) and tiles leaving the
    /// screen (zoom). Never hit-tested.
    private var leaving: [(view: NSView, closed: Bool)] = []
    /// Terminal tiles behind a snapshot until the current motion ends.
    private var frozen: [ObjectIdentifier: any MotionAware] = [:]
    /// Bumped by every motion; a finished motion only cleans up if it's still
    /// the latest.
    private var generation = 0
    private var motionEnds: CFTimeInterval = 0
    /// Per-terminal resize counts at the start of the current motion chain.
    private var resizeCheck: [String: (start: Int, expected: Int)] = [:]
    private var resizeKind = ""

    let clock = MotionClock()
    private var link: CADisplayLink?
    /// Display-link ticks so far; a motion that saw none ran with the
    /// display asleep or the screen locked.
    private(set) var linkTicks = 0
    private var ticksAtMotionStart = 0

    override var isFlipped: Bool { false }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = Design.Color.window.cgColor
        addSubview(ring)
        ring.layer?.zPosition = 2  // over moving tiles and outline ghosts
        clock.onChange = { [weak self] in self?.wake() }
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
        let t0 = CACurrentMediaTime()
        let newTab = window.workspace?.tab
        let active = Perf.measure("motion.active") { Motion.active }
        let animate = active && self.window != nil && bounds.width > 1 && bounds.height > 1
        let sameTab = tab != nil && newTab?.id == tab?.id
        let switching = tab != nil && newTab != nil && !sameTab

        // Where everything appears right now (mid-motion: the presentation).
        let before = animate && sameTab ? Perf.measure("tiles.visual") { visualFrames() } : [:]
        let ringBefore = ring.isHidden ? nil : visual(ring)
        let previousFocus = tab?.focused
        let tilesBefore = tab?.tiles.count ?? 0
        let zoomBefore = tab?.zoomed
        let axisBefore = tab?.root.rootAxis
        Perf.measure("tiles.finishLeaving") { finishLeaving() }

        if switching, animate { addTabTransition(to: window) }

        let all = window.workspaces.flatMap { $0.tabs }.flatMap { $0.tiles }
        let alive = Set(all.map(\.id))
        var closing: [(NSView, CGRect)] = []
        for (id, view) in views where !alive.contains(id) {
            if animate, sameTab, view.superview === self, let from = before[id] {
                // A closing terminal keeps a snapshot of its last frame to fade.
                (view as? MotionAware)?.motionWillStart(freeze: true, fill: Motion.style == .scale)
                closing.append((view, from))
            } else {
                view.removeFromSuperview()
            }
            view.tileWillClose()
            views[id] = nil
        }
        var created: Set<String> = []
        Perf.measure("tiles.update") {
            for tile in all {
                if let view = views[tile.id] {
                    view.update(tile)
                } else {
                    views[tile.id] = TileViews.make(tile)
                    created.insert(tile.id)
                }
            }
        }

        tab = newTab
        position = Self.position(of: window)
        let visible = Set(visibleTiles)
        var appearing: Set<String> = []
        var hiding: [(NSView, CGRect)] = []
        for (id, view) in views {
            if visible.contains(id) {
                if view.superview !== self {
                    addSubview(view, positioned: .below, relativeTo: ring)
                    appearing.insert(id)
                }
            } else if view.superview === self {
                if animate, sameTab, let from = before[id] {
                    hiding.append((view, from))
                } else {
                    view.removeFromSuperview()
                }
            }
        }

        if animate && sameTab {
            var kind = Self.kind(tilesBefore: tilesBefore, tilesAfter: tab?.tiles.count ?? 0,
                                 zoomBefore: zoomBefore, zoomAfter: tab?.zoomed,
                                 focusChanged: previousFocus != tab?.focused)
            if kind == "layout", let axisBefore, let after = tab?.root.rootAxis, after != axisBefore {
                kind = "rotate"
            }
            Perf.measure("tiles.runMotion") {
                runMotion(kind: kind, before: before, appearing: appearing, created: created,
                          closing: closing, hiding: hiding,
                          ringBefore: ringBefore, focusChanged: previousFocus != tab?.focused, t0: t0)
            }
        } else {
            cancelMotion()
            needsLayout = true
            layoutSubtreeIfNeeded()
            if !animate, previousFocus != tab?.focused { ring.layer?.removeAllAnimations() }
        }

        Perf.measure("tiles.focus") {
            guard let tab else { return }
            // Every refresh re-asserts focus; TileFocus decides whether the
            // focused tile may take the keyboard. A change of focused tile
            // always does.
            TileFocus.focusMoving = previousFocus != tab.focused
            defer { TileFocus.focusMoving = false }
            for (id, view) in views where visible.contains(id) {
                view.setFocused(id == tab.focused)
            }
        }
        Perf.measure("tiles.dim") { applyDim(animate: animate) }
    }

    /// Tile IDs on screen: the zoomed tile alone, the focused one in a
    /// Monocle tab, or every tile in the tab.
    private var visibleTiles: [String] {
        guard let tab else { return [] }
        if let zoomed = tab.zoomed { return [zoomed] }
        if tab.kind == .monocle { return [tab.focused] }
        return tab.order
    }

    /// Where each visible tile sits, in this view's coordinates.
    func tileFrames() -> [String: CGRect] {
        guard let tab else { return [:] }
        // insetBy on a rect smaller than the inset gives CGRect.null, whose
        // origin is infinite; AppKit clamps that to 2^44 and logs "constant
        // exceeds internal limits". The first layout pass runs at zero size.
        let dx = min(Self.inset, self.bounds.width / 2), dy = min(Self.inset, self.bounds.height / 2)
        let bounds = self.bounds.insetBy(dx: dx, dy: dy)
        if let zoomed = tab.zoomed { return [zoomed: bounds] }
        // Monocle: every tile full size, so switching between them restacks
        // and never resizes a terminal; only the focused one is on screen.
        if tab.kind == .monocle {
            return Dictionary(uniqueKeysWithValues: tab.tiles.map { ($0.id, bounds) })
        }
        return tab.root.frames(in: bounds, gap: Self.gap)
    }

    /// PTY resizes so far across this window's terminal tiles, for checks
    /// that a change resized nothing (Monocle focus switches).
    var totalSurfaceResizes: Int {
        views.values.compactMap { ($0 as? MotionAware)?.surfaceResizes }.reduce(0, +)
    }

    /// A tile's header strip, for overlays drawn in it (the agent badge).
    func header(of id: String) -> TileHeaderView? { (views[id] as? TileFrame)?.header }

    /// Frames of the tiles actually on screen (Monocle and zoom hide the
    /// rest), for overlays such as attention badges.
    func visibleTileFrames() -> [String: CGRect] {
        let shown = Set(visibleTiles)
        return tileFrames().filter { shown.contains($0.key) }
    }

    override func layout() {
        super.layout()
        let frames = tileFrames()
        for (id, frame) in frames { views[id]?.frame = frame }
        if let tab, let focused = frames[tab.focused], visibleTiles.count > 1 || tab.zoomed != nil {
            ring.isHidden = false
            ring.frame = focused.insetBy(dx: -Design.Border.focus, dy: -Design.Border.focus)
            ring.zoomed = tab.zoomed != nil
        } else {
            ring.isHidden = true
        }
    }

    // MARK: Motion

    private func runMotion(kind: String, before: [String: CGRect], appearing: Set<String>, created: Set<String>,
                           closing: [(NSView, CGRect)], hiding: [(NSView, CGRect)],
                           ringBefore: CGRect?, focusChanged: Bool, t0: CFTimeInterval) {
        let shown = Set(visibleTiles)
        let frames = tileFrames().filter { shown.contains($0.key) }

        // Who moves, from where. A tile already on its way to the same frame
        // keeps its animation.
        var movers: [(id: String, view: NSView, from: CGRect, to: CGRect, new: Bool)] = []
        for (id, to) in frames {
            guard let view = views[id] else { continue }
            if let from = before[id], !appearing.contains(id) {
                // The frame is the target of any motion in flight: unchanged
                // means it's there already, or on its way.
                if !Self.close(view.frame, to) { movers.append((id, view, from, to, false)) }
            } else if created.contains(id) {
                let from = Motion.style == .scale
                    ? growFrom(to: to, before: before, frames: frames)
                    : sweepFrom(to: to, before: before, frames: frames)
                movers.append((id, view, from, to, true))
            } else {
                // Back on screen (unzoom): fade in where it belongs.
                let from = Motion.style == .scale ? to.insetBy(dx: to.width * 0.03, dy: to.height * 0.03) : to
                movers.append((id, view, from, to, true))
            }
        }
        let layoutMoves = !movers.isEmpty || !closing.isEmpty || !hiding.isEmpty
        guard layoutMoves || focusChanged else {
            // Nothing moves (a title, a URL): don't disturb motion in flight.
            Perf.measure("tiles.idle-layout") {
                CATransaction.begin()
                CATransaction.setDisableActions(true)
                needsLayout = true
                layoutSubtreeIfNeeded()
                CATransaction.commit()
            }
            return
        }
        let retarget = CACurrentMediaTime() < motionEnds
        if retarget { MotionStats.shared.retargeted() } else { resizeCheck = [:]; resizeKind = kind }
        let duration = layoutMoves ? Motion.duration : Motion.focusDuration
        let style = Motion.style

        // 1. Snapshot every terminal that ends at a new size, before it resizes.
        //    Crop and outline keep it at 1:1, so note where it shows now.
        var snapshotAt: [String: CGRect] = [:]
        for m in movers where !m.new && m.view.frame.size != m.to.size {
            guard let aware = m.view as? MotionAware else { continue }
            let (frame, bounds) = geometry(m.view)
            // Mid-motion: where the snapshot shows now. Otherwise it rests
            // where freezing put it (below a tile's header strip, F28).
            let showing = aware.snapshotFrame
            guard Perf.measure("motion.snapshot", { aware.motionWillStart(freeze: true, fill: style == .scale) })
            else { continue }
            let shown = showing ?? aware.snapshotFrame ?? CGRect(origin: .zero, size: m.view.bounds.size)
            snapshotAt[m.id] = Motion.toContainer(shown, frame: frame, bounds: bounds, flipped: m.view.isFlipped)
            frozen[ObjectIdentifier(aware)] = aware
            if resizeCheck[m.id] == nil {
                resizeCheck[m.id] = (aware.surfaceResizes, 0)
            }
            resizeCheck[m.id]?.expected = 1
        }
        for m in movers where !m.new {
            if let aware = m.view as? MotionAware, resizeCheck[m.id] == nil {
                resizeCheck[m.id] = (aware.surfaceResizes, 0)
            }
        }

        // 2. Final frames now: the one resize, under the snapshots.
        for m in movers { stopMotion(m.view) }
        Perf.measure("motion.layout") {
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            needsLayout = true
            layoutSubtreeIfNeeded()
            CATransaction.commit()
        }

        // The z positions set below (leaving tiles under the rest, a zoomed
        // tile over its neighbours, the ring on top) in the subview order
        // too: anything that draws the layer tree in list order, as the app's
        // own captures do (`render(in:)` ignores zPosition), would otherwise
        // show a fading tile over the one growing into its space (V20).
        restack(back: closing.map(\.0) + hiding.map(\.0), front: tab?.zoomed.flatMap { views[$0] })

        // 3. Carry everything there visually.
        generation += 1
        let gen = generation
        let tail = style == .outline && layoutMoves ? Motion.outlineFade : 0
        motionEnds = CACurrentMediaTime() + duration + tail
        MotionStats.shared.movingUntil = max(MotionStats.shared.movingUntil, motionEnds)
        Perf.motionStarted = CACurrentMediaTime()
        ticksAtMotionStart = linkTicks
        removeGhosts()
        CATransaction.begin()
        CATransaction.setCompletionBlock { [weak self] in self?.motionEnded(gen, via: "completion") }
        for m in movers {
            guard let layer = m.view.layer else { continue }
            switch style {
            case .scale:
                scaleMotion(layer, from: m.from, to: m.to, fadeIn: m.new, duration: duration)
            case .crop where kind == "rotate":
                crossFade(m.view, from: m.from, wasShown: !m.new, duration: duration)
            case .crop:
                cropMotion(m.view, id: m.id, from: m.from, to: m.to, fadeIn: m.new,
                           snapshotAt: snapshotAt[m.id], duration: duration)
            case .outline:
                outlineMotion(m.view, id: m.id, from: m.from, to: m.to, duration: duration)
            }
            // The zoomed tile travels over its neighbours.
            layer.zPosition = tab?.zoomed == m.id ? 1 : 0
        }
        for (view, from) in closing { leave(view, from: from, closed: true, frames: frames, duration: duration) }
        for (view, from) in hiding { leave(view, from: from, closed: false, frames: frames, duration: duration) }
        // The ring rides with the focused tile when it moves (a new tile
        // sweeping in, a neighbour growing into a closed tile's space): an
        // independent path from the old focus cut across the other cards.
        var ringFrom = ringBefore
        var ringFades = ringBefore == nil
        if layoutMoves, !(style == .crop && kind == "rotate"),
           let focused = tab?.focused, let m = movers.first(where: { $0.id == focused }) {
            ringFrom = m.from.insetBy(dx: -Design.Border.focus, dy: -Design.Border.focus)
            ringFades = ringFades || m.new
        }
        animateRing(from: ringFrom, fadeIn: ringFades, duration: focusChanged || layoutMoves ? duration : 0)
        CATransaction.commit()

        MotionStats.shared.started(kind: kind, setup: CACurrentMediaTime() - t0)
        wake()
    }

    /// Reorders the subviews: `back` first, `front` just under the ring, the
    /// ring last. Setting `subviews` to the same views reorders them without
    /// removing any, so first responder and surfaces stay put.
    private func restack(back: [NSView], front: NSView?) {
        let backIDs = Set(back.map(ObjectIdentifier.init))
        let current = subviews
        var ordered = current.filter { backIDs.contains(ObjectIdentifier($0)) }
        ordered += current.filter { !backIDs.contains(ObjectIdentifier($0)) && $0 !== front && $0 !== ring }
        if let front, front.superview === self, !backIDs.contains(ObjectIdentifier(front)) { ordered.append(front) }
        if ring.superview === self { ordered.append(ring) }
        guard ordered.count == current.count, !zip(ordered, current).allSatisfy({ $0 === $1 }) else { return }
        subviews = ordered
    }

    /// A new tile grows out of the edge it shares with the tile it split from;
    /// failing that, it scales up from its centre.
    private func growFrom(to: CGRect, before: [String: CGRect], frames: [String: CGRect]) -> CGRect {
        let centre = CGPoint(x: to.midX, y: to.midY)
        if let parent = before.first(where: { $0.value.contains(centre) && frames[$0.key] != nil }),
           let now = frames[parent.key] {
            let slack: CGFloat = Self.gap + 1
            if now.maxX <= to.minX + slack { return CGRect(x: to.minX, y: to.minY, width: 0, height: to.height) }
            if now.minX >= to.maxX - slack { return CGRect(x: to.maxX, y: to.minY, width: 0, height: to.height) }
            if now.maxY <= to.minY + slack { return CGRect(x: to.minX, y: to.minY, width: to.width, height: 0) }
            if now.minY >= to.maxY - slack { return CGRect(x: to.minX, y: to.maxY, width: to.width, height: 0) }
        }
        return to.insetBy(dx: to.width * 0.04, dy: to.height * 0.04)
    }

    /// Crop and outline: a new tile sweeps in with the divider. Its parent
    /// gives up the space from its far edge, so the new tile grows from there.
    private func sweepFrom(to: CGRect, before: [String: CGRect], frames: [String: CGRect]) -> CGRect {
        let centre = CGPoint(x: to.midX, y: to.midY)
        guard let parent = before.first(where: { $0.value.contains(centre) && frames[$0.key] != nil }),
              let now = frames[parent.key]
        else { return to }
        let slack: CGFloat = Self.gap + 1
        if now.maxX <= to.minX + slack { return CGRect(x: to.maxX, y: to.minY, width: 0, height: to.height) }
        if now.minX >= to.maxX - slack { return CGRect(x: to.minX, y: to.minY, width: 0, height: to.height) }
        if now.maxY <= to.minY + slack { return CGRect(x: to.minX, y: to.maxY, width: to.width, height: 0) }
        if now.minY >= to.maxY - slack { return CGRect(x: to.minX, y: to.minY, width: to.width, height: 0) }
        return to
    }

    /// Where a closing tile shrinks to: away from the neighbours that take
    /// its space, so their edge sweeps over it. Nothing takes it: in place.
    private func collapse(_ from: CGRect, frames: [String: CGRect]) -> CGRect {
        let takers = frames.values.filter { $0.intersects(from) }
        guard var u = takers.first else { return from }
        for r in takers.dropFirst() { u = u.union(r) }
        let left = u.minX < from.minX - 1, right = u.maxX > from.maxX + 1
        let below = u.minY < from.minY - 1, above = u.maxY > from.maxY + 1
        switch (left, right, below, above) {
        case (true, false, _, _): return CGRect(x: from.maxX, y: from.minY, width: 0, height: from.height)
        case (false, true, _, _): return CGRect(x: from.minX, y: from.minY, width: 0, height: from.height)
        case (true, true, _, _): return CGRect(x: from.midX, y: from.minY, width: 0, height: from.height)
        case (_, _, true, false): return CGRect(x: from.minX, y: from.maxY, width: from.width, height: 0)
        case (_, _, false, true): return CGRect(x: from.minX, y: from.minY, width: from.width, height: 0)
        case (_, _, true, true): return CGRect(x: from.minX, y: from.midY, width: from.width, height: 0)
        default: return from
        }
    }

    /// A view's frame and bounds as they show right now (mid-motion: the
    /// presentation), in container and tile coordinates.
    private func geometry(_ view: NSView) -> (frame: CGRect, bounds: CGRect) {
        guard let layer = view.layer, layer.animation(forKey: Self.motionKey) != nil,
              let p = layer.presentation()
        else { return (view.frame, CGRect(origin: .zero, size: view.bounds.size)) }
        return (p.frame, p.bounds)
    }

    /// Scale: a transform carries the tile from `from` (content stretches).
    private func scaleMotion(_ layer: CALayer, from: CGRect, to: CGRect, fadeIn: Bool, duration: CFTimeInterval) {
        let group = CAAnimationGroup()
        let move = CABasicAnimation(keyPath: "transform")
        move.fromValue = NSValue(caTransform3D: Motion.transform(from: from, to: to, anchor: layer.anchorPoint))
        move.toValue = NSValue(caTransform3D: CATransform3DIdentity)
        group.animations = [move]
        if fadeIn { group.animations?.append(fadeAnimation(from: 0, to: layer.opacity)) }
        Motion.configure(group, duration: duration)
        layer.add(group, forKey: Self.motionKey)
    }

    /// Crop: the tile's bounds and position animate, so its frame is a clip
    /// over content at 1:1. Content sticks to the edge that moves least. A
    /// frozen terminal's snapshot stays exactly where it shows now.
    private func cropMotion(_ view: NSView, id: String, from: CGRect, to: CGRect, fadeIn: Bool,
                            snapshotAt: CGRect?, duration: CFTimeInterval) {
        guard let layer = view.layer else { return }
        layer.masksToBounds = true
        let anchor = Motion.anchors(from: from, to: to)
        let flipped = view.isFlipped
        let start = Motion.cropBounds(clip: from.size, content: to.size, anchor: anchor, flipped: flipped)
        let end = CGRect(origin: .zero, size: to.size)
        let group = CAAnimationGroup()
        group.animations = clipAnimations(layer, from: from, fromBounds: start, to: to, toBounds: end)
        if fadeIn { group.animations?.append(fadeAnimation(from: 0, to: layer.opacity)) }
        Motion.configure(group, duration: duration)
        layer.add(group, forKey: Self.motionKey)
        // The card's rounded corners follow the moving clip (D.5).
        (view as? TileFrame)?.animateCorners(from: start, to: end, duration: duration, holdAtEnd: false)

        guard let aware = view as? MotionAware, let shown = snapshotAt else { return }
        let picture = Motion.toTile(shown, frame: from, bounds: start, flipped: flipped)
        aware.placeSnapshot(picture, cover: start.union(end).union(picture))
        // Growing: live content (laid out at the final size) is never
        // stretched, so it can show as soon as it has drawn.
        if to.width >= from.width - 0.5, to.height >= from.height - 0.5 {
            aware.revealWhenFresh(clock: clock)
        }
    }

    private func clipAnimations(_ layer: CALayer, from: CGRect, fromBounds: CGRect,
                                to: CGRect, toBounds: CGRect) -> [CAAnimation] {
        let bounds = CABasicAnimation(keyPath: "bounds")
        bounds.fromValue = NSValue(rect: fromBounds)
        bounds.toValue = NSValue(rect: toBounds)
        let position = CABasicAnimation(keyPath: "position")
        position.fromValue = NSValue(point: Motion.position(of: from, anchorPoint: layer.anchorPoint))
        position.toValue = NSValue(point: Motion.position(of: to, anchorPoint: layer.anchorPoint))
        return [bounds, position]
    }

    private func fadeAnimation(from: Float, to: Float) -> CABasicAnimation {
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = from
        fade.toValue = to
        return fade
    }

    /// Crop, tall rotate: sliding every tile across the tab looks wrong, so
    /// the tile fades in at its new place (its snapshot at 1:1, top-left)
    /// while a ghost of its last frame fades out at the old one.
    private func crossFade(_ view: NSView, from: CGRect, wasShown: Bool, duration: CFTimeInterval) {
        guard let layer = view.layer else { return }
        let fade = fadeAnimation(from: 0, to: layer.opacity)
        Motion.configure(fade, duration: duration)
        layer.add(fade, forKey: Self.motionKey)
        guard let aware = view as? MotionAware, let picture = aware.snapshotFrame else { return }
        let size = picture.size
        let top = view.isFlipped ? aware.headerInset : view.bounds.height - aware.headerInset - size.height
        aware.placeSnapshot(CGRect(x: aware.contentLeft, y: top, width: size.width, height: size.height),
                            cover: view.bounds)
        aware.revealWhenFresh(clock: clock)
        // The ghost stands for the card as it was on screen, so a tile that
        // wasn't (back from Monocle or zoom) leaves none. It's clipped to the
        // old card: a snapshot bigger than it never draws past its edges.
        guard wasShown, let contents = aware.snapshotContents else { return }
        let ghost = CALayer()
        ghost.frame = from
        ghost.masksToBounds = true
        ghost.cornerRadius = Design.Radius.tile
        ghost.zPosition = 0.5
        ghost.opacity = 0
        let image = CALayer()
        image.frame = CGRect(x: aware.contentLeft, y: from.height - aware.headerInset - size.height,
                             width: size.width, height: size.height)
        image.contents = contents
        image.contentsGravity = .resize
        image.contentsScale = window?.backingScaleFactor ?? 2
        image.backgroundColor = layer.backgroundColor
        ghost.addSublayer(image)
        let out = fadeAnimation(from: 1, to: 0)
        Motion.configure(out, duration: duration)
        ghost.add(out, forKey: Self.motionKey)
        // Not by tile: the tile itself is already where it shows.
        addGhost(ghost, for: nil)
    }

    /// Outline: the tile sits at its new place, hidden; an outline moves
    /// there instead, and the content fades in at the end.
    private func outlineMotion(_ view: NSView, id: String, from: CGRect, to: CGRect, duration: CFTimeInterval) {
        guard let layer = view.layer else { return }
        layer.add(hiddenUntilEnd(duration), forKey: Self.motionKey)
        if let aware = view as? MotionAware, let picture = aware.snapshotFrame {
            let top = view.isFlipped ? aware.headerInset : view.bounds.height - aware.headerInset - picture.height
            aware.placeSnapshot(CGRect(x: aware.contentLeft, y: top, width: picture.width, height: picture.height),
                                cover: view.bounds)
        }
        let ghost = outline(at: to)
        let group = CAAnimationGroup()
        group.animations = clipAnimations(ghost, from: from, fromBounds: CGRect(origin: .zero, size: from.size),
                                          to: to, toBounds: CGRect(origin: .zero, size: to.size))
        Motion.configure(group, duration: duration)
        ghost.add(group, forKey: Self.motionKey)
        addGhost(ghost, for: id)
    }

    /// Hidden for the motion, then an `outlineFade` fade in.
    private func hiddenUntilEnd(_ duration: CFTimeInterval) -> CAAnimation {
        let total = duration + Motion.outlineFade
        let hide = CAKeyframeAnimation(keyPath: "opacity")
        hide.values = [0, 0, 1]
        hide.keyTimes = [0, NSNumber(value: duration / total), 1]
        hide.duration = total
        let rate = Motion.refreshRate
        hide.preferredFrameRateRange = CAFrameRateRange(minimum: rate / 2, maximum: rate, preferred: rate)
        return hide
    }

    /// A tile-shaped outline for outline motion.
    private func outline(at frame: CGRect) -> CALayer {
        let ghost = CALayer()
        ghost.frame = frame
        ghost.cornerRadius = Design.Radius.ghost
        ghost.borderWidth = Design.Border.hairline
        ghost.borderColor = Design.Color.textMuted.cgColor
        ghost.backgroundColor = Design.Color.ghostFill.cgColor
        ghost.zPosition = 0.5
        return ghost
    }

    /// Fades a tile out under the rest; closed tiles go for good afterwards.
    /// Crop: its clip shrinks away from the neighbours taking its space.
    /// Scale: it shrinks slightly. Outline: its outline does, content hidden.
    private func leave(_ view: NSView, from: CGRect, closed: Bool, frames: [String: CGRect], duration: CFTimeInterval) {
        guard let layer = view.layer else {
            view.removeFromSuperview()
            return
        }
        stopMotion(view)
        let model = view.frame
        let target = closed ? collapse(from, frames: frames) : from
        let group = CAAnimationGroup()
        switch Motion.style {
        case .scale:
            let shrunk = from.insetBy(dx: from.width * 0.04, dy: from.height * 0.04)
            let move = CABasicAnimation(keyPath: "transform")
            move.fromValue = NSValue(caTransform3D: Motion.transform(from: from, to: model, anchor: layer.anchorPoint))
            move.toValue = NSValue(caTransform3D: Motion.transform(from: shrunk, to: model, anchor: layer.anchorPoint))
            group.animations = [move, fadeAnimation(from: layer.opacity, to: 0)]
        case .crop:
            let anchor = Motion.anchors(from: from, to: target)
            let start = Motion.cropBounds(clip: from.size, content: model.size, anchor: anchor, flipped: view.isFlipped)
            let end = Motion.cropBounds(clip: target.size, content: model.size, anchor: anchor, flipped: view.isFlipped)
            group.animations = clipAnimations(layer, from: from, fromBounds: start, to: target, toBounds: end)
                + [fadeAnimation(from: layer.opacity, to: 0) as CAAnimation]
            (view as? TileFrame)?.animateCorners(from: start, to: end, duration: duration, holdAtEnd: true)
        case .outline:
            group.animations = [fadeAnimation(from: 0, to: 0)]
            if closed {
                let ghost = outline(at: target)
                let shrink = CAAnimationGroup()
                shrink.animations = clipAnimations(
                    ghost, from: from, fromBounds: CGRect(origin: .zero, size: from.size),
                    to: target, toBounds: CGRect(origin: .zero, size: target.size))
                    + [fadeAnimation(from: 1, to: 0) as CAAnimation]
                shrink.fillMode = .forwards
                shrink.isRemovedOnCompletion = false
                Motion.configure(shrink, duration: duration)
                ghost.add(shrink, forKey: Self.motionKey)
                addGhost(ghost, for: nil)
            }
        }
        group.fillMode = .forwards
        group.isRemovedOnCompletion = false
        Motion.configure(group, duration: duration)
        layer.zPosition = -1
        layer.add(group, forKey: Self.motionKey)
        leaving.append((view, closed))
    }

    // MARK: Ghosts (outline motion, cross-fades)

    private var ghosts: [CALayer] = []
    /// Outline ghosts by tile, so a motion that retargets starts from where
    /// the outline shows.
    private var ghostByTile: [String: CALayer] = [:]

    private func addGhost(_ ghost: CALayer, for id: String?) {
        layer?.addSublayer(ghost)
        ghosts.append(ghost)
        if let id { ghostByTile[id] = ghost }
    }

    private func removeGhosts() {
        for ghost in ghosts { ghost.removeFromSuperlayer() }
        ghosts = []
        ghostByTile = [:]
    }

    private func animateRing(from: CGRect?, fadeIn: Bool, duration: CFTimeInterval) {
        guard let layer = ring.layer else { return }
        layer.removeAnimation(forKey: Self.motionKey)
        guard !ring.isHidden, duration > 0 else { return }
        let to = ring.frame
        let a = layer.anchorPoint
        let group = CAAnimationGroup()
        group.animations = []
        if let from {
            let bounds = CABasicAnimation(keyPath: "bounds.size")
            bounds.fromValue = NSValue(size: from.size)
            bounds.toValue = NSValue(size: to.size)
            let position = CABasicAnimation(keyPath: "position")
            position.fromValue = NSValue(point: CGPoint(x: from.minX + a.x * from.width, y: from.minY + a.y * from.height))
            position.toValue = NSValue(point: CGPoint(x: to.minX + a.x * to.width, y: to.minY + a.y * to.height))
            group.animations?.append(contentsOf: [bounds, position])
        }
        if fadeIn {
            let fade = CABasicAnimation(keyPath: "opacity")
            fade.fromValue = 0
            fade.toValue = 1
            group.animations?.append(fade)
        }
        Motion.configure(group, duration: duration)
        layer.add(group, forKey: Self.motionKey)
    }

    /// Drops a tile's motion, its corner caps' included.
    private func stopMotion(_ view: NSView) {
        view.layer?.removeAnimation(forKey: Self.motionKey)
        (view as? TileFrame)?.stopCornerMotion()
    }

    private func motionEnded(_ gen: Int, via: String) {
        guard gen == generation else { return }
        if !frozen.isEmpty || linkTicks == ticksAtMotionStart {
            Motion.log("motion \(gen) ended (\(via)): \(frozen.count) frozen terminal(s), "
                + "\(linkTicks - ticksAtMotionStart) display-link tick(s)")
        }
        finishLeaving()
        removeGhosts()
        for view in subviews where view !== ring { view.layer?.zPosition = 0 }
        for aware in frozen.values { aware.motionDidEnd(clock: clock) }
        frozen = [:]
        for (id, check) in resizeCheck {
            guard let aware = views[id] as? MotionAware else { continue }
            MotionStats.shared.resizes(tile: id, expected: check.expected,
                                       actual: aware.surfaceResizes - check.start, kind: resizeKind)
        }
        resizeCheck = [:]
    }

    /// Ends any motion at once (tab switch, motion off).
    private func cancelMotion() {
        generation += 1
        motionEnds = 0
        for view in subviews where view !== ring {
            stopMotion(view)
            view.layer?.zPosition = 0
        }
        ring.layer?.removeAnimation(forKey: Self.motionKey)
        removeGhosts()
        finishLeaving()
        for aware in frozen.values { aware.motionCancelled() }
        frozen = [:]
        resizeCheck = [:]
    }

    private func finishLeaving() {
        for (view, closed) in leaving {
            stopMotion(view)
            view.layer?.zPosition = 0
            if closed {
                view.removeFromSuperview()
            } else if let id = (view as? any TileView)?.tileID, !visibleTiles.contains(id) {
                view.removeFromSuperview()
            }
        }
        leaving = []
    }

    /// Cross-fade or slide between tabs and workspaces. Added before the
    /// subviews change, in the same transaction.
    private func addTabTransition(to window: WindowModel) {
        cancelMotion()
        let transition = CATransition()
        switch Motion.settings.tabSwitch {
        case "none": return
        case "slide":
            let to = Self.position(of: window)
            let forward = (to.workspace, to.tab) > (position.workspace, position.tab)
            transition.type = .push
            transition.subtype = forward ? .fromRight : .fromLeft
        default:
            transition.type = .fade
        }
        Motion.configure(transition, duration: Motion.tabDuration)
        layer?.add(transition, forKey: "azj.tab")
        MotionStats.shared.started(kind: "tab_switch", setup: 0)
        motionEnds = CACurrentMediaTime() + Motion.tabDuration
        MotionStats.shared.movingUntil = max(MotionStats.shared.movingUntil, motionEnds)
        Perf.motionStarted = CACurrentMediaTime()
        wake()
    }

    private func applyDim(animate: Bool) {
        guard let tab else { return }
        let dim = Motion.settings.dimInactive && visibleTiles.count > 1
        for id in visibleTiles {
            guard let view = views[id] else { continue }
            let alpha: CGFloat = dim && id != tab.focused ? Design.Alpha.unfocusedTile : 1
            guard view.alphaValue != alpha else { continue }
            if animate {
                NSAnimationContext.runAnimationGroup { ctx in
                    ctx.duration = Motion.focusDuration
                    view.animator().alphaValue = alpha
                }
            } else {
                view.alphaValue = alpha
            }
        }
    }

    /// Automation: what each visible terminal shows right now (live surface
    /// or motion snapshot) and whether it's current, for captures that would
    /// otherwise pass off an old frame as the app's state.
    func captureFreshness() -> (tiles: [[String: Any]], ticks: Int) {
        let moving = CACurrentMediaTime() < motionEnds
        let shown = Set(visibleTiles)
        let tiles = views.filter { shown.contains($0.key) }.sorted { $0.key < $1.key }.compactMap { _, view in
            ((view as? TileFrame)?.content as? TerminalTileView)?.captureFreshness(moving: moving)
        }
        return (tiles, linkTicks - ticksAtMotionStart)
    }

    /// Automation: each tile's on-screen frame right now (mid-motion: the
    /// presentation), as "x,y,w,h" by tile id suffix.
    var describeVisualFrames: String {
        func text(_ r: CGRect) -> String { "\(Int(r.minX)),\(Int(r.minY)),\(Int(r.width)),\(Int(r.height))" }
        var parts = visualFrames().sorted { $0.key < $1.key }.map { id, r in "\(id.suffix(4))=\(text(r))" }
        if !ring.isHidden { parts.append("ring=\(text(visual(ring)))") }
        return parts.joined(separator: " ")
    }

    /// Where each on-screen tile appears right now: its presentation frame
    /// while it moves, else its frame.
    private func visualFrames() -> [String: CGRect] {
        var out: [String: CGRect] = [:]
        for (id, view) in views where view.superview === self {
            // Outline motion: the tile sits at its target; its outline shows.
            if let ghost = ghostByTile[id], let p = ghost.presentation() {
                out[id] = p.frame
            } else {
                out[id] = visual(view)
            }
        }
        return out
    }

    private func visual(_ view: NSView) -> CGRect {
        guard let layer = view.layer, layer.animation(forKey: Self.motionKey) != nil,
              let presentation = layer.presentation()
        else { return view.frame }
        return presentation.frame
    }

    private static func close(_ a: CGRect, _ b: CGRect) -> Bool {
        abs(a.minX - b.minX) < 0.5 && abs(a.minY - b.minY) < 0.5
            && abs(a.width - b.width) < 0.5 && abs(a.height - b.height) < 0.5
    }

    private static func position(of window: WindowModel) -> (workspace: Int, tab: Int) {
        let ws = window.workspaces.firstIndex { $0.id == window.activeWorkspace } ?? 0
        let tab = window.workspace.flatMap { w in w.tabs.firstIndex { $0.id == w.tab?.id } } ?? 0
        return (ws, tab)
    }

    private static func kind(tilesBefore: Int, tilesAfter: Int, zoomBefore: String?, zoomAfter: String?,
                             focusChanged: Bool) -> String {
        if tilesAfter > tilesBefore { return "split" }
        if tilesAfter < tilesBefore { return "close" }
        if zoomBefore != zoomAfter { return zoomAfter == nil ? "unzoom" : "zoom" }
        return focusChanged ? "focus" : "layout"
    }

    // MARK: Frame clock

    /// Runs the display link while anything moves or waits for a frame.
    private func wake() {
        if link == nil {
            let link = displayLink(target: self, selector: #selector(tick(_:)))
            let rate = Motion.refreshRate
            link.preferredFrameRateRange = CAFrameRateRange(minimum: rate / 2, maximum: rate, preferred: rate)
            link.add(to: .main, forMode: .common)
            self.link = link
        }
        link?.isPaused = false
    }

    @objc private func tick(_ link: CADisplayLink) {
        linkTicks += 1
        let now = CACurrentMediaTime()
        if now <= motionEnds + 0.02 {
            MotionStats.shared.tick(link.timestamp, now: now)
        } else {
            MotionStats.shared.idle()
        }
        Perf.measure("clock.tick") { clock.tick(now) }
        if clock.isEmpty, now > motionEnds + 0.05 {
            link.isPaused = true
            MotionStats.shared.idle()
        }
    }

    // MARK: Input

    /// Clicking a tile focuses it in the core.
    var onClickTile: ((String) -> Void)?

    override func hitTest(_ point: NSPoint) -> NSView? {
        let hit = super.hitTest(point)
        if hit === ring { return nil }
        if let hit, leaving.contains(where: { hit === $0.view || hit.isDescendant(of: $0.view) }) { return self }
        return hit
    }

    /// Tiles that don't handle clicks pass them up the responder chain to here.
    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        let shown = Set(visibleTiles)
        if let id = tileFrames().first(where: { shown.contains($0.key) && $0.value.contains(point) })?.key {
            onClickTile?(id)
        }
        super.mouseDown(with: event)
    }
}

extension LayoutNode {
    /// The axis of the top split; a tall rotate flips it.
    var rootAxis: Axis? {
        if case .split(let axis, _, _, _) = self { return axis }
        return nil
    }
}

/// The border around the focused tile (`Design.Color.focus`, `Design.Border.focus`;
/// pending designer confirmation, see docs/design/README.md). A layer border, so moving and
/// resizing it is cheap and stays crisp.
@MainActor
final class FocusRingView: NSView {
    var zoomed = false { didSet { if zoomed != oldValue { needsDisplay = true } } }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        isHidden = true
        layer?.cornerRadius = Design.Radius.tile + Design.Border.focus
        layer?.borderWidth = Design.Border.focus
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var wantsUpdateLayer: Bool { true }

    override func updateLayer() {
        layer?.borderColor = (zoomed ? Design.Color.zoom : Design.Color.focus).cgColor
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}
