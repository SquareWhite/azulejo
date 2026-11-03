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
    static let gap: CGFloat = 4
    static let inset: CGFloat = 4
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

    override var isFlipped: Bool { false }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        addSubview(ring)
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
        let animate = Motion.active && self.window != nil && bounds.width > 1 && bounds.height > 1
        let sameTab = tab != nil && newTab?.id == tab?.id
        let switching = tab != nil && newTab != nil && !sameTab

        // Where everything appears right now (mid-motion: the presentation).
        let before = animate && sameTab ? visualFrames() : [:]
        let ringBefore = ring.isHidden ? nil : visual(ring)
        let previousFocus = tab?.focused
        let tilesBefore = tab?.tiles.count ?? 0
        let zoomBefore = tab?.zoomed
        finishLeaving()

        if switching, animate { addTabTransition(to: window) }

        let all = window.workspaces.flatMap { $0.tabs }.flatMap { $0.tiles }
        let alive = Set(all.map(\.id))
        var closing: [(NSView, CGRect)] = []
        for (id, view) in views where !alive.contains(id) {
            if animate, sameTab, view.superview === self, let from = before[id] {
                // A closing terminal keeps a snapshot of its last frame to fade.
                (view as? MotionAware)?.motionWillStart(freeze: true)
                closing.append((view, from))
            } else {
                view.removeFromSuperview()
            }
            view.tileWillClose()
            views[id] = nil
        }
        var created: Set<String> = []
        for tile in all {
            if let view = views[tile.id] {
                view.update(tile)
            } else {
                views[tile.id] = TileViews.make(tile)
                created.insert(tile.id)
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
            let kind = Self.kind(tilesBefore: tilesBefore, tilesAfter: tab?.tiles.count ?? 0,
                                 zoomBefore: zoomBefore, zoomAfter: tab?.zoomed,
                                 focusChanged: previousFocus != tab?.focused)
            runMotion(kind: kind, before: before, appearing: appearing, created: created,
                      closing: closing, hiding: hiding,
                      ringBefore: ringBefore, focusChanged: previousFocus != tab?.focused, t0: t0)
        } else {
            cancelMotion()
            needsLayout = true
            layoutSubtreeIfNeeded()
            if !animate, previousFocus != tab?.focused { ring.layer?.removeAllAnimations() }
        }

        if let tab {
            for (id, view) in views where visible.contains(id) {
                view.setFocused(id == tab.focused)
            }
        }
        applyDim(animate: animate)
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
        // insetBy on a rect smaller than the inset gives CGRect.null, whose
        // origin is infinite; AppKit clamps that to 2^44 and logs "constant
        // exceeds internal limits". The first layout pass runs at zero size.
        let dx = min(Self.inset, self.bounds.width / 2), dy = min(Self.inset, self.bounds.height / 2)
        let bounds = self.bounds.insetBy(dx: dx, dy: dy)
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

    // MARK: Motion

    private func runMotion(kind: String, before: [String: CGRect], appearing: Set<String>, created: Set<String>,
                           closing: [(NSView, CGRect)], hiding: [(NSView, CGRect)],
                           ringBefore: CGRect?, focusChanged: Bool, t0: CFTimeInterval) {
        let frames = tileFrames()

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
                movers.append((id, view, growFrom(to: to, before: before, frames: frames), to, true))
            } else {
                // Back on screen (unzoom): fade in where it belongs.
                movers.append((id, view, to.insetBy(dx: to.width * 0.03, dy: to.height * 0.03), to, true))
            }
        }
        let layoutMoves = !movers.isEmpty || !closing.isEmpty || !hiding.isEmpty
        guard layoutMoves || focusChanged else {
            // Nothing moves (a title, a URL): don't disturb motion in flight.
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            needsLayout = true
            layoutSubtreeIfNeeded()
            CATransaction.commit()
            return
        }
        let retarget = CACurrentMediaTime() < motionEnds
        if retarget { MotionStats.shared.retargeted() } else { resizeCheck = [:]; resizeKind = kind }
        let duration = layoutMoves ? Motion.duration : Motion.focusDuration

        // 1. Snapshot every terminal that ends at a new size, before it resizes.
        for m in movers where !m.new && m.view.frame.size != m.to.size {
            guard let aware = m.view as? MotionAware, aware.motionWillStart(freeze: true) else { continue }
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
        for m in movers { m.view.layer?.removeAnimation(forKey: Self.motionKey) }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        needsLayout = true
        layoutSubtreeIfNeeded()
        CATransaction.commit()

        // 3. Carry everything there visually.
        generation += 1
        let gen = generation
        motionEnds = CACurrentMediaTime() + duration
        CATransaction.begin()
        CATransaction.setCompletionBlock { [weak self] in self?.motionEnded(gen) }
        for m in movers {
            guard let layer = m.view.layer else { continue }
            let group = CAAnimationGroup()
            let move = CABasicAnimation(keyPath: "transform")
            move.fromValue = NSValue(caTransform3D: Motion.transform(from: m.from, to: m.to, anchor: layer.anchorPoint))
            move.toValue = NSValue(caTransform3D: CATransform3DIdentity)
            group.animations = [move]
            if m.new {
                let fade = CABasicAnimation(keyPath: "opacity")
                fade.fromValue = 0
                fade.toValue = layer.opacity
                group.animations?.append(fade)
            }
            Motion.configure(group, duration: duration)
            layer.add(group, forKey: Self.motionKey)
            // The zoomed tile travels over its neighbours.
            layer.zPosition = tab?.zoomed == m.id ? 1 : 0
        }
        for (view, from) in closing { leave(view, from: from, closed: true, duration: duration) }
        for (view, from) in hiding { leave(view, from: from, closed: false, duration: duration) }
        animateRing(from: ringBefore, duration: focusChanged || layoutMoves ? duration : 0)
        CATransaction.commit()

        MotionStats.shared.started(kind: kind, setup: CACurrentMediaTime() - t0)
        wake()
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

    /// Fades a tile out where it appears now, shrinking slightly, under the
    /// rest; closed tiles go for good afterwards.
    private func leave(_ view: NSView, from: CGRect, closed: Bool, duration: CFTimeInterval) {
        guard let layer = view.layer else {
            view.removeFromSuperview()
            return
        }
        layer.removeAnimation(forKey: Self.motionKey)
        let model = view.frame
        let shrunk = from.insetBy(dx: from.width * 0.04, dy: from.height * 0.04)
        let move = CABasicAnimation(keyPath: "transform")
        move.fromValue = NSValue(caTransform3D: Motion.transform(from: from, to: model, anchor: layer.anchorPoint))
        move.toValue = NSValue(caTransform3D: Motion.transform(from: shrunk, to: model, anchor: layer.anchorPoint))
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = layer.opacity
        fade.toValue = 0
        let group = CAAnimationGroup()
        group.animations = [move, fade]
        group.fillMode = .forwards
        group.isRemovedOnCompletion = false
        Motion.configure(group, duration: duration)
        layer.zPosition = -1
        layer.add(group, forKey: Self.motionKey)
        leaving.append((view, closed))
    }

    private func animateRing(from: CGRect?, duration: CFTimeInterval) {
        guard let layer = ring.layer else { return }
        layer.removeAnimation(forKey: Self.motionKey)
        guard !ring.isHidden, duration > 0 else { return }
        let to = ring.frame
        let a = layer.anchorPoint
        let group = CAAnimationGroup()
        if let from {
            let bounds = CABasicAnimation(keyPath: "bounds.size")
            bounds.fromValue = NSValue(size: from.size)
            bounds.toValue = NSValue(size: to.size)
            let position = CABasicAnimation(keyPath: "position")
            position.fromValue = NSValue(point: CGPoint(x: from.minX + a.x * from.width, y: from.minY + a.y * from.height))
            position.toValue = NSValue(point: CGPoint(x: to.minX + a.x * to.width, y: to.minY + a.y * to.height))
            group.animations = [bounds, position]
        } else {
            let fade = CABasicAnimation(keyPath: "opacity")
            fade.fromValue = 0
            fade.toValue = 1
            group.animations = [fade]
        }
        Motion.configure(group, duration: duration)
        layer.add(group, forKey: Self.motionKey)
    }

    private func motionEnded(_ gen: Int) {
        guard gen == generation else { return }
        finishLeaving()
        for view in subviews { view.layer?.zPosition = 0 }
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
        for view in subviews {
            view.layer?.removeAnimation(forKey: Self.motionKey)
            view.layer?.zPosition = 0
        }
        finishLeaving()
        for aware in frozen.values { aware.motionCancelled() }
        frozen = [:]
        resizeCheck = [:]
    }

    private func finishLeaving() {
        for (view, closed) in leaving {
            view.layer?.removeAnimation(forKey: Self.motionKey)
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
        wake()
    }

    private func applyDim(animate: Bool) {
        guard let tab else { return }
        let dim = Motion.settings.dimInactive && visibleTiles.count > 1
        for id in visibleTiles {
            guard let view = views[id] else { continue }
            let alpha: CGFloat = dim && id != tab.focused ? 0.72 : 1
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

    /// Where each on-screen tile appears right now: its presentation frame
    /// while it moves, else its frame.
    private func visualFrames() -> [String: CGRect] {
        var out: [String: CGRect] = [:]
        for (id, view) in views where view.superview === self {
            out[id] = visual(view)
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
        let now = CACurrentMediaTime()
        if now <= motionEnds + 0.02 {
            MotionStats.shared.tick(link.timestamp)
        } else {
            MotionStats.shared.idle()
        }
        clock.tick(now)
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
        if let id = tileFrames().first(where: { $0.value.contains(point) })?.key {
            onClickTile?(id)
        }
        super.mouseDown(with: event)
    }
}

/// The accent border around the focused tile. A layer border, so moving and
/// resizing it is cheap and stays crisp.
@MainActor
final class FocusRingView: NSView {
    var zoomed = false { didSet { if zoomed != oldValue { needsDisplay = true } } }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        isHidden = true
        layer?.cornerRadius = 7
        layer?.borderWidth = 2
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var wantsUpdateLayer: Bool { true }

    override func updateLayer() {
        layer?.borderColor = (zoomed ? NSColor.systemOrange : NSColor.controlAccentColor).cgColor
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}
