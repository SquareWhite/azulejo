import AppKit
import QuartzCore

/// Lays out the active tab's tiles from the core's tree. Keeps one view per
/// tile for every tab in the window, so switching tabs or rearranging tiles
/// never recreates a tile (a terminal keeps its surface, a browser its page).
///
/// Layout changes animate (F4, `Motion`): each tile jumps to its final frame
/// at the start — one PTY resize per terminal, hidden under its snapshot
/// (ADR 0004) — and Core Animation carries it there visually, from its old
/// frame to its new one (rotates and layout switches included; see
/// `runMotion`). A change that arrives mid-motion starts from where every
/// tile appears on screen, snapshots and fades included, so holding a key
/// never queues up motion. The container's own size changes
/// with the window chrome (the sidebar) the same way (`setChromeFrame`).
///
/// Focus shows by dimming every unfocused tile (`applyDim`); there's no
/// outline round the focused one.
@MainActor
final class TileContainerView: NSView, ThemeAware {
    static let gap = Design.Space.gutter
    static let inset = Design.Space.tileInset
    private static let motionKey = "azj.motion"

    private var views: [String: any TileView] = [:]
    private var tab: TabModel?
    /// Where the active tab sits, for the direction of a slide.
    private var position: (workspace: Int, tab: Int) = (0, 0)

    /// Views fading out: closed tiles (removed for good) and tiles leaving the
    /// screen (zoom). Never hit-tested.
    private var leaving: [(view: NSView, closed: Bool)] = []
    /// Terminal tiles behind a snapshot until the current motion ends.
    private var frozen: [ObjectIdentifier: any MotionAware] = [:]
    /// Bumped by every motion; a finished motion only cleans up if it's still
    /// the latest.
    private var generation = 0
    private var motionEnds: CFTimeInterval = 0
    /// Per terminal over the current motion chain: its resize count and size
    /// at the start, and how many of the chain's motions changed its size.
    /// `live`: how many of those motions reflowed it (each may add up to
    /// `Motion.maxLiveResizes`).
    private var resizeCheck: [String: (start: Int, expected: Int, size: CGSize, live: Int)] = [:]
    private var resizeKind = ""
    /// The last motion `motionEnded` ran for (the reflow clock and its
    /// fallback timer can both get there).
    private var endedGeneration = -1

    /// Reflow motion: a tile whose frame the display link steps along the
    /// curve, from where it showed to its final frame (`driveReflow`).
    private struct Reflow {
        let view: NSView
        let from: CGRect
        let to: CGRect
        let start: CFTimeInterval
        let duration: CFTimeInterval
    }
    private var reflowing: [String: Reflow] = [:]

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
        clock.onChange = { [weak self] in self?.wake() }
        Theme.track(self)
    }

    func applyTheme() {
        layer?.backgroundColor = Design.Color.window.cgColor
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        Theme.reapply(self)
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
        // And where frozen snapshots show and how faded tiles caught
        // mid-fade are: a change that brings one back carries on from there
        // (Monocle in and straight out again).
        let shownBefore = animate && sameTab ? Perf.measure("tiles.snapshots") { snapshotsOnScreen() } : [:]
        let opacityBefore = animate && sameTab ? fadingOpacities() : [:]
        let previousFocus = tab?.focused
        let tilesBefore = tab?.tiles.count ?? 0
        let zoomBefore = tab?.zoomed
        let axisBefore = tab?.root.rootAxis
        let layoutBefore = tab?.kind
        // Tiles on their way off the screen keep going (a refresh mid-motion
        // doesn't cut their fade short); the motion that ends last drops them.
        let fadingOut = Set(leaving.filter { !$0.closed }.compactMap { ($0.view as? any TileView)?.tileID })

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
                    Perf.measure("tiles.attach") { addSubview(view) }
                    appearing.insert(id)
                } else if fadingOut.contains(id) {
                    // Wanted back mid-fade: it turns round where it shows.
                    leaving.removeAll { $0.view === view }
                    appearing.insert(id)
                }
            } else if view.superview === self {
                if fadingOut.contains(id), animate, sameTab {
                    continue  // already on its way out
                } else if animate, sameTab, let from = before[id] {
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
            if kind == "layout" || kind == "focus", zoomBefore == nil, tab?.zoomed == nil,
               let monocle = Self.monocleKind(before: layoutBefore, after: tab?.kind, focus: kind) {
                kind = monocle
            } else if kind == "layout", let axisBefore, let after = tab?.root.rootAxis, after != axisBefore {
                kind = "rotate"
            }
            Perf.measure("tiles.runMotion") {
                runMotion(kind: kind, before: before, appearing: appearing, created: created,
                          closing: closing, hiding: hiding, shownBefore: shownBefore,
                          opacityBefore: opacityBefore, t0: t0)
            }
        } else {
            cancelMotion()
            needsLayout = true
            layoutSubtreeIfNeeded()
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
        // A tab switch cross-fades the whole container; its tiles take their
        // dim at once under it.
        Perf.measure("tiles.dim") { applyDim(animate: animate && sameTab) }
    }

    /// The tile area's new frame as the chrome around it changes (the
    /// sidebar opening or closing). `animated`: a layout motion like any
    /// other. Every tile starts from where it shows on screen now, takes its
    /// final frame at once (one PTY resize per terminal, under its snapshot,
    /// ADR 0004) and moves there with the motion's style, curve and length,
    /// in step with the chrome's own motion. Otherwise, or with motion off,
    /// the tiles jump.
    func setChromeFrame(_ new: NSRect, animated: Bool) {
        let old = frame
        guard animated, Motion.active, window != nil, tab != nil, !Self.close(old, new),
              old.width > 1, old.height > 1, new.width > 1, new.height > 1
        else {
            if frame != new { frame = new }
            return
        }
        let t0 = CACurrentMediaTime()
        // Tiles sit in this view's coordinates, and this view moves: where
        // they show now, in its new coordinates.
        let shift = CGPoint(x: old.minX - new.minX, y: old.minY - new.minY)
        // A closing tile's fade lives in the old coordinates: finish it.
        finishLeaving()
        let before = visualFrames().mapValues { $0.offsetBy(dx: shift.x, dy: shift.y) }
        let shown = snapshotsOnScreen().mapValues { $0.offsetBy(dx: shift.x, dy: shift.y) }
        frame = new
        runMotion(kind: "sidebar", before: before, appearing: [], created: [], closing: [], hiding: [],
                  shownBefore: shown, opacityBefore: [:], shift: shift, t0: t0)
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
        // Reflowing tiles are on their way there, a step per display frame.
        for (id, frame) in frames where reflowing[id] == nil { views[id]?.frame = frame }
    }

    // MARK: Motion

    /// One tile in a motion: where it shows now (`from`) and where it goes.
    /// `fadeFrom`: it comes onto the screen, fading in from that opacity.
    private struct Mover {
        let id: String
        let view: NSView
        let from: CGRect
        let to: CGRect
        let fadeFrom: Float?
    }

    /// `shift`: how far this view itself moved since `before` was taken
    /// (a chrome change), so where a snapshot shows can be found.
    /// `shownBefore`: where each frozen terminal's snapshot showed when
    /// `before` was taken; `opacityBefore`: tiles caught mid-fade.
    ///
    /// Every change of frames is a move, on the motion's curve and length:
    /// a rotate, a layout switch, masters, promote and the rest carry each
    /// tile from its old frame to its new one, position and size together.
    /// Monocle (ADR 0010): going in, the focused tile grows to fill the tab
    /// over the others, which fade and shrink a little where they are;
    /// coming out, it shrinks to its slot while they grow back in. A focus
    /// switch inside Monocle dissolves the next tile in over the last one,
    /// quickly (`Motion.stackDuration`): the tiles are stacked in one place,
    /// so there's no path to move along, and no terminal resizes.
    private func runMotion(kind: String, before: [String: CGRect], appearing: Set<String>, created: Set<String>,
                           closing: [(NSView, CGRect)], hiding: [(NSView, CGRect)],
                           shownBefore: [String: CGRect], opacityBefore: [String: Float],
                           shift: CGPoint = .zero, t0: CFTimeInterval) {
        let shown = Set(visibleTiles)
        // Every tile's final frame, hidden ones too (Monocle sizes them all).
        let finals = tileFrames()
        let frames = finals.filter { shown.contains($0.key) }
        let style = Motion.style
        let reflow = style == .reflow
        let stacked = kind == "monocle_focus"

        // Who moves, from where. A tile already on its way to the same frame
        // keeps its animation.
        var movers: [Mover] = []
        for (id, to) in frames {
            guard let view = views[id] else { continue }
            if let from = before[id], !appearing.contains(id) {
                // The frame is the target of any motion in flight (reflow:
                // the one it's stepping towards): unchanged means it's there
                // already, or on its way.
                let target = reflowing[id]?.to ?? view.frame
                if !Self.close(target, to) {
                    movers.append(Mover(id: id, view: view, from: from, to: to, fadeFrom: nil))
                }
            } else if created.contains(id) {
                let from = style == .scale
                    ? growFrom(to: to, before: before, frames: frames)
                    : sweepFrom(to: to, before: before, frames: frames)
                movers.append(Mover(id: id, view: view, from: from, to: to, fadeFrom: 0))
            } else if let from = before[id] {
                // Turned round while fading out: back from where it shows.
                movers.append(Mover(id: id, view: view, from: from, to: to, fadeFrom: opacityBefore[id] ?? 0))
            } else {
                // Back on screen (out of Monocle or zoom): it fades and grows
                // in at its place. Switching tiles in Monocle: in place.
                let from = stacked || style == .outline ? to : Self.tucked(to)
                movers.append(Mover(id: id, view: view, from: from, to: to, fadeFrom: 0))
            }
        }
        let layoutMoves = !movers.isEmpty || !closing.isEmpty || !hiding.isEmpty
        guard layoutMoves else {
            // Nothing moves (a title, a URL, focus alone: the dim cross-fades
            // on its own): don't disturb motion in flight.
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
        let duration = stacked ? Motion.stackDuration : Motion.duration

        // 1. Snapshot every terminal that ends at a new size, before it
        //    resizes: tiles that move, come back or go (Monocle sizes hidden
        //    tiles to the whole tab). Crop and outline keep it at 1:1, so
        //    note where it shows now; one already up (mid-motion) stays.
        var snapshotAt: [String: CGRect] = [:]
        // Reflow: nothing freezes on its way; a terminal whose path changes
        // its size resizes a few times (live) and exactly once at the end.
        func countLive(_ m: Mover) {
            guard let aware = m.view as? MotionAware, aware.resizesLive else { return }
            if resizeCheck[m.id] == nil { resizeCheck[m.id] = (aware.surfaceResizes, 0, m.view.frame.size, 0) }
            guard !Self.close(CGRect(origin: .zero, size: m.from.size), CGRect(origin: .zero, size: m.to.size))
            else { return }
            resizeCheck[m.id]?.expected += 1
            resizeCheck[m.id]?.live += 1
        }
        func freeze(_ id: String, _ view: NSView, to: CGRect?, onScreen: Bool) {
            guard let aware = view as? MotionAware else { return }
            if resizeCheck[id] == nil { resizeCheck[id] = (aware.surfaceResizes, 0, view.frame.size, 0) }
            if let to, view.frame.size != to.size {
                guard Perf.measure("motion.snapshot", { aware.motionWillStart(freeze: true, fill: style == .scale) })
                else { return }
                frozen[ObjectIdentifier(aware)] = aware
                // One resize per motion that changes its size, retargets too.
                resizeCheck[id]?.expected += 1
            }
            guard onScreen else { return }
            if let at = shownBefore[id] {
                snapshotAt[id] = at
            } else if let rest = aware.snapshotFrame {
                // Freezing put it where the live content was (below a tile's
                // header strip, F28); that, where the tile shows now.
                let (frame, bounds) = geometry(view)
                snapshotAt[id] = Motion.toContainer(rest, frame: frame.offsetBy(dx: shift.x, dy: shift.y),
                                                    bounds: bounds, flipped: view.isFlipped)
            }
        }
        Perf.measure("motion.freeze") {
            for m in movers where !created.contains(m.id) {
                if reflow {
                    countLive(m)
                } else {
                    freeze(m.id, m.view, to: m.to, onScreen: before[m.id] != nil)
                }
            }
            for (view, _) in hiding {
                guard let id = (view as? any TileView)?.tileID else { continue }
                freeze(id, view, to: finals[id], onScreen: true)
            }
            for (view, _) in closing {
                guard let id = (view as? any TileView)?.tileID, let at = shownBefore[id] else { continue }
                snapshotAt[id] = at
            }
        }

        // Reflow: tiles that were on their way and now leave the screen (a
        // close, Monocle) stop reflowing; the movers start, or turn round
        // from where they are.
        if reflow {
            let shownIDs = Set(frames.keys)
            for (id, r) in reflowing where !shownIDs.contains(id) {
                reflowing[id] = nil
                (r.view as? MotionAware)?.liveResizeDidEnd()
            }
            let now = CACurrentMediaTime()
            for m in movers {
                (m.view as? MotionAware)?.liveResizeWillStart(final: m.to.size)
                reflowing[m.id] = Reflow(view: m.view, from: m.from, to: m.to, start: now, duration: duration)
            }
        }

        // 2. Final frames now: the one resize, under the snapshots. Reflow:
        //    the movers start where they show instead (layout skips them).
        for m in movers { stopMotion(m.view) }
        Perf.measure("motion.layout") {
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            needsLayout = true
            layoutSubtreeIfNeeded()
            if reflow {
                for m in movers where !Self.close(m.view.frame, m.from) { m.view.frame = m.from }
            }
            CATransaction.commit()
        }

        // The z positions set below (leaving tiles under the rest, the
        // zoomed or focused tile over its neighbours) in the subview order
        // too: anything that draws the layer tree in list order, as the app's
        // own captures do (`render(in:)` ignores zPosition), would otherwise
        // show a fading tile over the one growing into its space (V20).
        let front = (tab?.zoomed ?? tab?.focused).flatMap { views[$0] }
        restack(back: closing.map(\.0) + hiding.map(\.0), front: front)

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
        if reflow {
            // Nothing Core Animation runs says when a reflow is over (a
            // transaction with no animations completes at once): the clock
            // does, and a timer if the display link isn't ticking (screen
            // locked, display asleep).
            clock.add { [weak self] now in
                guard let self, gen == self.generation else { return false }
                guard now >= self.motionEnds else { return true }
                Perf.measure("motion.end") { self.motionEnded(gen, via: "clock") }
                return false
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + duration + 0.1) { [weak self] in
                guard let self, gen == self.generation, CACurrentMediaTime() >= self.motionEnds else { return }
                self.motionEnded(gen, via: "timer, no display-link tick")
            }
        } else {
            CATransaction.setCompletionBlock { [weak self] in
                Perf.measure("motion.end") { self?.motionEnded(gen, via: "completion") }
            }
        }
        for m in movers {
            guard let layer = m.view.layer else { continue }
            switch style {
            case .reflow:
                if let fadeFrom = m.fadeFrom {
                    let fade = fadeAnimation(from: fadeFrom, to: layer.opacity)
                    Motion.configure(fade, duration: duration)
                    layer.add(fade, forKey: Self.motionKey)
                }
            case .scale:
                scaleMotion(layer, from: m.from, to: m.to, fadeFrom: m.fadeFrom, duration: duration)
            case .crop:
                cropMotion(m.view, from: m.from, to: m.to, fadeFrom: m.fadeFrom,
                           snapshotAt: snapshotAt[m.id], duration: duration)
            case .outline:
                outlineMotion(m.view, id: m.id, from: m.from, to: m.to, duration: duration)
            }
            // The zoomed or focused tile travels over its neighbours.
            layer.zPosition = m.view === front ? 1 : 0
        }
        for (view, from) in closing {
            let id = (view as? any TileView)?.tileID ?? ""
            leave(view, from: from, to: collapse(from, frames: frames), closed: true, fades: true,
                  snapshotAt: snapshotAt[id], opacity: nil, duration: duration)
        }
        for (view, from) in hiding {
            // Monocle's next tile dissolves in over this one, which stays
            // put until it's covered; anything else fades and shrinks a
            // little where it is, under the tile taking the space.
            let id = (view as? any TileView)?.tileID ?? ""
            leave(view, from: from, to: stacked ? from : Self.tucked(from), closed: false, fades: !stacked,
                  snapshotAt: snapshotAt[id], opacity: opacityBefore[id], duration: duration)
        }
        CATransaction.commit()

        MotionStats.shared.started(kind: kind, setup: CACurrentMediaTime() - t0)
        wake()
    }

    /// A little inside `frame`: where a tile leaving the screen shrinks to,
    /// and where one coming back grows from.
    private static func tucked(_ frame: CGRect) -> CGRect {
        frame.insetBy(dx: frame.width * 0.03, dy: frame.height * 0.03)
    }

    /// Reorders the subviews: `back` first, `front` last. Setting `subviews`
    /// to the same views reorders them without removing any, so first
    /// responder and surfaces stay put.
    private func restack(back: [NSView], front: NSView?) {
        let backIDs = Set(back.map(ObjectIdentifier.init))
        let current = subviews
        var ordered = current.filter { backIDs.contains(ObjectIdentifier($0)) }
        ordered += current.filter { !backIDs.contains(ObjectIdentifier($0)) && $0 !== front }
        if let front, front.superview === self, !backIDs.contains(ObjectIdentifier(front)) { ordered.append(front) }
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
    private func scaleMotion(_ layer: CALayer, from: CGRect, to: CGRect, fadeFrom: Float?, duration: CFTimeInterval) {
        let group = CAAnimationGroup()
        let move = CABasicAnimation(keyPath: "transform")
        move.fromValue = NSValue(caTransform3D: Motion.transform(from: from, to: to, anchor: layer.anchorPoint))
        move.toValue = NSValue(caTransform3D: CATransform3DIdentity)
        group.animations = [move]
        if let fadeFrom { group.animations?.append(fadeAnimation(from: fadeFrom, to: layer.opacity)) }
        Motion.configure(group, duration: duration)
        layer.add(group, forKey: Self.motionKey)
    }

    /// Crop: the tile's bounds and position animate, so its frame is a clip
    /// over content at 1:1. Content sticks to the edge that moves least. A
    /// frozen terminal's snapshot stays exactly where it shows now; one
    /// coming back onto the screen (out of Monocle) shows its last frame at
    /// its content's top left until live content has drawn at its new size.
    private func cropMotion(_ view: NSView, from: CGRect, to: CGRect, fadeFrom: Float?,
                            snapshotAt: CGRect?, duration: CFTimeInterval) {
        guard let layer = view.layer else { return }
        layer.masksToBounds = true
        let anchor = Motion.anchors(from: from, to: to)
        let flipped = view.isFlipped
        let start = Motion.cropBounds(clip: from.size, content: to.size, anchor: anchor, flipped: flipped)
        let end = CGRect(origin: .zero, size: to.size)
        let group = CAAnimationGroup()
        group.animations = clipAnimations(layer, from: from, fromBounds: start, to: to, toBounds: end)
        if let fadeFrom { group.animations?.append(fadeAnimation(from: fadeFrom, to: layer.opacity)) }
        Motion.configure(group, duration: duration)
        layer.add(group, forKey: Self.motionKey)
        // The card's rounded corners follow the moving clip (D.5).
        (view as? TileFrame)?.animateCorners(from: start, to: end, duration: duration, holdAtEnd: false)

        guard let aware = view as? MotionAware, let rest = aware.snapshotFrame else { return }
        let picture: CGRect
        if let shown = snapshotAt {
            picture = Motion.toTile(shown, frame: from, bounds: start, flipped: flipped)
        } else {
            let top = flipped ? aware.headerInset : to.height - aware.headerInset - rest.height
            picture = CGRect(x: aware.contentLeft, y: top, width: rest.width, height: rest.height)
        }
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
        // theme: made fresh for each outline motion, gone when it ends.
        ghost.borderColor = Design.Color.textMuted.cgColor
        ghost.backgroundColor = Design.Color.ghostFill.cgColor
        ghost.zPosition = 0.5
        return ghost
    }

    /// Takes a tile off the screen under the rest; closed tiles go for good
    /// afterwards. `fades`: it fades out (from `opacity`, when it was caught
    /// mid-fade); otherwise it stays put until the motion ends, covered by
    /// the tile dissolving in over it (Monocle). `snapshotAt`: where its
    /// frozen terminal's snapshot shows now.
    /// Crop: its clip shrinks to `target` (away from the neighbours taking a
    /// closed tile's space; a little, where it is, for a hidden one), content
    /// at 1:1. Scale: it shrinks slightly. Outline: its outline does,
    /// content hidden.
    private func leave(_ view: NSView, from: CGRect, to target: CGRect, closed: Bool, fades: Bool,
                       snapshotAt: CGRect?, opacity: Float?, duration: CFTimeInterval) {
        guard let layer = view.layer else {
            view.removeFromSuperview()
            return
        }
        stopMotion(view)
        let model = view.frame
        let startOpacity = opacity ?? layer.opacity
        // Staying put, one caught mid-fade holds where it was, so what shows
        // through it doesn't change at once.
        let fade: [CAAnimation] = fades ? [fadeAnimation(from: startOpacity, to: 0)]
            : opacity.map { [fadeAnimation(from: $0, to: $0)] } ?? []
        let group = CAAnimationGroup()
        switch Motion.style {
        case .scale:
            let shrunk = fades ? from.insetBy(dx: from.width * 0.04, dy: from.height * 0.04) : from
            let move = CABasicAnimation(keyPath: "transform")
            move.fromValue = NSValue(caTransform3D: Motion.transform(from: from, to: model, anchor: layer.anchorPoint))
            move.toValue = NSValue(caTransform3D: Motion.transform(from: shrunk, to: model, anchor: layer.anchorPoint))
            group.animations = [move] + fade
        case .crop, .reflow:
            // Reflow too: a closed terminal has no surface left to reflow,
            // and one Monocle hides is already at the whole tab's size.
            layer.masksToBounds = true
            let anchor = Motion.anchors(from: from, to: target)
            let flipped = view.isFlipped
            let start = Motion.cropBounds(clip: from.size, content: model.size, anchor: anchor, flipped: flipped)
            let end = Motion.cropBounds(clip: target.size, content: model.size, anchor: anchor, flipped: flipped)
            group.animations = clipAnimations(layer, from: from, fromBounds: start, to: target, toBounds: end) + fade
            (view as? TileFrame)?.animateCorners(from: start, to: end, duration: duration, holdAtEnd: true)
            // A snapshot stays where it shows now: Monocle has already sized
            // a hidden terminal to the whole tab, under its last frame.
            if let aware = view as? MotionAware, let shown = snapshotAt {
                let picture = Motion.toTile(shown, frame: from, bounds: start, flipped: flipped)
                aware.placeSnapshot(picture, cover: start.union(end).union(picture))
            }
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
        leaving.removeAll { $0.view === view }
        leaving.append((view, closed))
    }

    /// Tiles caught mid-fade (in or out) and their opacity on screen now.
    private func fadingOpacities() -> [String: Float] {
        var out: [String: Float] = [:]
        for (id, view) in views where view.superview === self {
            guard let layer = view.layer, layer.animation(forKey: Self.motionKey) != nil,
                  let p = layer.presentation(), p.opacity < 0.999
            else { continue }
            out[id] = p.opacity
        }
        return out
    }

    /// Where each frozen terminal's snapshot shows right now, in this
    /// view's coordinates.
    private func snapshotsOnScreen() -> [String: CGRect] {
        var out: [String: CGRect] = [:]
        for (id, view) in views where view.superview === self {
            guard let aware = view as? MotionAware, let picture = aware.snapshotFrame else { continue }
            let (frame, bounds) = geometry(view)
            out[id] = Motion.toContainer(picture, frame: frame, bounds: bounds, flipped: view.isFlipped)
        }
        return out
    }

    // MARK: Ghosts (outline motion)

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

    /// Drops a tile's motion, its corner caps' included.
    private func stopMotion(_ view: NSView) {
        view.layer?.removeAnimation(forKey: Self.motionKey)
        (view as? TileFrame)?.stopCornerMotion()
    }

    private func motionEnded(_ gen: Int, via: String) {
        guard gen == generation, gen != endedGeneration else { return }
        endedGeneration = gen
        finishReflow()
        if !frozen.isEmpty || linkTicks == ticksAtMotionStart {
            Motion.log("motion \(gen) ended (\(via)): \(frozen.count) frozen terminal(s), "
                + "\(linkTicks - ticksAtMotionStart) display-link tick(s)")
        }
        finishLeaving()
        removeGhosts()
        for view in subviews { view.layer?.zPosition = 0 }
        for aware in frozen.values {
            // Off the screen now (hidden by Monocle): nothing shows it, so
            // the snapshot goes at once.
            if (aware as? NSView)?.superview === self {
                aware.motionDidEnd(clock: clock)
            } else {
                aware.motionCancelled()
            }
        }
        frozen = [:]
        for (id, check) in resizeCheck {
            guard let view = views[id], let aware = view as? MotionAware else { continue }
            // A chain that ends where it began (a double toggle) may have
            // had its resizes merged away.
            let resized = abs(view.frame.width - check.size.width) >= 0.5
                || abs(view.frame.height - check.size.height) >= 0.5
            let maximum = check.live > 0 ? check.expected + check.live * Motion.maxLiveResizes(Motion.duration) : nil
            MotionStats.shared.resizes(tile: id, expected: check.expected, minimum: resized ? 1 : 0,
                                       maximum: maximum, actual: aware.surfaceResizes - check.start, kind: resizeKind)
        }
        resizeCheck = [:]
    }

    /// Ends any motion at once (tab switch, motion off).
    private func cancelMotion() {
        finishReflow()
        generation += 1
        motionEnds = 0
        for view in subviews {
            stopMotion(view)
            view.layer?.zPosition = 0
        }
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

    /// The focus indicator: every unfocused tile on screen dims
    /// (`Motion.dimStrength` of `Motion.dimFill` over it, header strip
    /// included); the focused one stays at full brightness. Nothing dims
    /// when one tile is on screen (a lone tile, Monocle, zoom). It follows
    /// the core's focused tile, not the keyboard, so ⌘K, the sidebar's
    /// Files panel or the search field leave the last focused tile bright,
    /// and it doesn't change when the window isn't key (as in Ghostty).
    /// Focus moving cross-fades it (`Motion.dimDuration`).
    private func applyDim(animate: Bool) {
        guard let tab else { return }
        let shown = visibleTiles
        let strength = shown.count > 1 ? Motion.dimStrength : 0
        for id in shown {
            guard let frame = views[id] as? TileFrame else { continue }
            frame.setDim(id == tab.focused ? 0 : strength, duration: animate ? Motion.dimDuration : 0)
        }
    }

    /// Automation: which tiles on screen are dimmed, by their place in the
    /// tab's order ("2,3,4"), or "none".
    var dimmedTiles: String {
        guard let tab else { return "none" }
        let shown = Set(visibleTiles)
        let dimmed = tab.order.enumerated().filter { _, id in
            shown.contains(id) && ((views[id] as? TileFrame)?.dimLevel ?? 0) > 0
        }.map { "\($0.offset + 1)" }
        return dimmed.isEmpty ? "none" : dimmed.joined(separator: ",")
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
    /// presentation), as "x,y,w,h" by tile id suffix, then the focused
    /// tile's suffix and the dimmed ones ("focus=ab12 dim=cd34,ef56").
    var describeVisualFrames: String {
        func text(_ r: CGRect) -> String { "\(Int(r.minX)),\(Int(r.minY)),\(Int(r.width)),\(Int(r.height))" }
        var parts = visualFrames().sorted { $0.key < $1.key }.map { id, r in "\(id.suffix(4))=\(text(r))" }
        if let tab {
            parts.append("focus=\(tab.focused.suffix(4))")
            let dimmed = views.filter { $0.value.superview === self && (($0.value as? TileFrame)?.dimLevel ?? 0) > 0 }
                .keys.sorted().map { String($0.suffix(4)) }
            parts.append("dim=\(dimmed.isEmpty ? "-" : dimmed.joined(separator: ","))")
        }
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

    /// Into or out of Monocle, or between its stacked tiles (`focus` is the
    /// kind so far); nil when Monocle isn't involved.
    private static func monocleKind(before: LayoutKind?, after: LayoutKind?, focus kind: String) -> String? {
        switch (before == .monocle, after == .monocle) {
        case (false, true): "monocle_in"
        case (true, false): "monocle_out"
        case (true, true): kind == "focus" ? "monocle_focus" : nil
        case (false, false): nil
        }
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
        // Where the reflowing tiles are when this frame shows.
        if !reflowing.isEmpty { Perf.measure("motion.reflow") { driveReflow(at: link.targetTimestamp) } }
        Perf.measure("clock.tick") { clock.tick(now) }
        if clock.isEmpty, reflowing.isEmpty, now > motionEnds + 0.05 {
            link.isPaused = true
            MotionStats.shared.idle()
        }
    }

    // MARK: Reflow

    /// Steps every reflowing tile to where the curve puts it at `time`,
    /// whole device pixels, so a terminal's surface never sits between
    /// them. Its content lays out at that size (`liveResizeWillStart`).
    private func driveReflow(at time: CFTimeInterval) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        var done = false
        for r in reflowing.values {
            let t = r.duration > 0 ? (time - r.start) / r.duration : 1
            if t >= 1 {
                done = true
                continue
            }
            let e = CGFloat(Motion.progress(t))
            func mix(_ a: CGFloat, _ b: CGFloat) -> CGFloat { a + (b - a) * e }
            let step = backingAlignedRect(
                CGRect(x: mix(r.from.minX, r.to.minX), y: mix(r.from.minY, r.to.minY),
                       width: mix(r.from.width, r.to.width), height: mix(r.from.height, r.to.height)),
                options: .alignAllEdgesNearest)
            if r.view.frame != step { r.view.frame = step }
        }
        if done { finishReflow(at: time) }
        CATransaction.commit()
    }

    /// Puts reflowing tiles at their final frame and has their content take
    /// its exact size: those whose time is up at `time`, or all of them.
    private func finishReflow(at time: CFTimeInterval = .infinity) {
        guard !reflowing.isEmpty else { return }
        let finals = tileFrames()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for (id, r) in reflowing where time >= r.start + r.duration {
            reflowing[id] = nil
            // A window resize mid-motion moved the target: the latest one.
            let to = finals[id] ?? r.to
            if r.view.frame != to { r.view.frame = to }
            (r.view as? MotionAware)?.liveResizeDidEnd()
        }
        CATransaction.commit()
    }

    // MARK: Input

    /// Clicking a tile focuses it in the core.
    var onClickTile: ((String) -> Void)?

    override func hitTest(_ point: NSPoint) -> NSView? {
        let hit = super.hitTest(point)
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
