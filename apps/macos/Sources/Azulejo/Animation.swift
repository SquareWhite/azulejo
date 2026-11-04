import AppKit
import QuartzCore

/// Layout motion settings (`[animation]` in the config, F4) and the helpers
/// the tile container animates with.
///
/// How motion works (ADR 0004): with `reflow` (the default) every moving
/// tile's frame steps along the curve once per display frame on the main
/// thread, like a window being drag-resized, and a terminal's grid follows
/// it, `reflowLead` ahead (`TerminalTileView`'s live resize: whole cells
/// only, at most one PTY resize per `reflowInterval`, the exact final size
/// once the lead reaches the end). The other
/// styles send every tile to its final frame at the start of a motion — so a
/// terminal's PTY is resized exactly once, while a snapshot covers it — and
/// Core Animation carries it visually from where it was to where it is.
///
/// Every layout change with a geometric relation moves: each tile goes from
/// its old frame to its new one (rotate, layout switches, Monocle in and
/// out, masters, promote, swaps, splits, closes, zoom, the sidebar). Only
/// changes with no path between the two pictures fade: tab and workspace
/// switches (another tab's tiles), and a focus switch between Monocle's
/// stacked tiles (the next one dissolves in over the last).
///
/// `style` says what the content does on the way:
/// - "reflow" (default): the content lays out at every step; a terminal's
///   text rewraps and its TUI redraws as the tile grows or shrinks.
/// - "crop": the tile's frame is a clip. Its bounds and position
///   animate; content stays at 1:1, revealed or cut off by the moving edges.
/// - "scale": a transform scales the whole tile (text stretches mid-motion).
/// - "outline": tiles jump to their new place hidden; only outlines move, and
///   content fades in at the end.
@MainActor
enum Motion {
    struct Settings: Equatable {
        var enabled = true
        var durationMs = 200
        var curve = "ease_out"
        var style = "reflow"
        var tabSwitch = "fade"
        var reduceMotion = "system"
        /// Ghostty's `unfocused-split-opacity` and `-fill`, when its config
        /// sets them (the core reads them; nil otherwise).
        var unfocusedOpacity: Double?
        var unfocusedFill: NSColor?
    }

    private(set) static var settings = Settings()
    /// Automation overrides (slow motion for mid-animation screenshots, or off).
    static var durationOverride: CFTimeInterval?
    static var enabledOverride: Bool?
    static var styleOverride: String?
    /// The app log (motion ends, reveals).
    static var log: (String) -> Void = { _ in }

    enum Style { case reflow, crop, scale, outline }

    static var style: Style {
        switch styleOverride ?? settings.style {
        case "crop": .crop
        case "scale": .scale
        case "outline": .outline
        default: .reflow
        }
    }

    /// Reflow: the shortest gap between two PTY resizes of one terminal
    /// mid-motion (each is a SIGWINCH for its TUI, through herdr). libghostty
    /// itself sends at most one per 25 ms. `AZJ_REFLOW_MS` overrides it
    /// (automation: `anim reflow-ms`).
    static var reflowInterval: CFTimeInterval = {
        if let v = ProcessInfo.processInfo.environment["AZJ_REFLOW_MS"].flatMap(Double.init), v >= 0 {
            return v / 1000
        }
        return 0.033
    }()

    /// Reflow: how far ahead of the frame a terminal's grid runs. A live
    /// resize takes the size the tile will have this long from now (the
    /// final size once that's past the end), because the redraw it asks for
    /// lands about that late: libghostty holds a PTY resize 25 ms, herdr's
    /// attach client looks at its size every 100 ms (0–100 ms, a fixed
    /// sleep; SIGWINCH doesn't wake it), and herdr draws the pane's answer
    /// about 19 ms after the pane resized. A grid that runs ahead is
    /// clipped by the tile (growing) or shows the terminal's background
    /// (shrinking), like one that trails, but the last redraw lands with the
    /// motion's end instead of 50–150 ms after it. `AZJ_REFLOW_LEAD_MS`
    /// overrides it (automation: `anim reflow-lead`); 0 follows the frame.
    static var reflowLead: CFTimeInterval = {
        if let v = ProcessInfo.processInfo.environment["AZJ_REFLOW_LEAD_MS"].flatMap(Double.init), v >= 0 {
            return v / 1000
        }
        return 0.1
    }()

    /// Reflow: the most PTY resizes one motion of `duration` can cost a
    /// terminal (the live ones, plus the exact one at the end).
    static func maxLiveResizes(_ duration: CFTimeInterval) -> Int {
        guard reflowInterval > 0 else { return Int.max / 4 }
        return Int((duration / reflowInterval).rounded(.up)) + 1
    }

    /// The motion curve at `t` (0…1 of the time): how far along its path a
    /// reflowing tile is. Core Animation's own curve, solved for x.
    static func progress(_ t: Double) -> Double {
        if t <= 0 { return 0 }
        if t >= 1 { return 1 }
        var c1: [Float] = [0, 0], c2: [Float] = [0, 0]
        let f = timing
        f.getControlPoint(at: 1, values: &c1)
        f.getControlPoint(at: 2, values: &c2)
        let (x1, y1, x2, y2) = (Double(c1[0]), Double(c1[1]), Double(c2[0]), Double(c2[1]))
        func bezier(_ s: Double, _ a: Double, _ b: Double) -> Double {
            let u = 1 - s
            return 3 * u * u * s * a + 3 * u * s * s * b + s * s * s
        }
        func slope(_ s: Double, _ a: Double, _ b: Double) -> Double {
            let u = 1 - s
            return 3 * u * u * a + 6 * u * s * (b - a) + 3 * s * s * (1 - b)
        }
        // Newton from t; bisection if it doesn't settle.
        var s = t
        for _ in 0..<8 {
            let dx = bezier(s, x1, x2) - t
            if abs(dx) < 1e-6 { return bezier(s, y1, y2) }
            let d = slope(s, x1, x2)
            if abs(d) < 1e-6 { break }
            s = min(max(s - dx / d, 0), 1)
        }
        var lo = 0.0, hi = 1.0
        s = t
        for _ in 0..<40 {
            let x = bezier(s, x1, x2)
            if abs(x - t) < 1e-6 { break }
            if x < t { lo = s } else { hi = s }
            s = (lo + hi) / 2
        }
        return bezier(s, y1, y2)
    }

    /// How long content takes to fade in after an outline motion.
    static let outlineFade: CFTimeInterval = 0.08

    /// Reads `azj_core_animation`'s JSON. Called at launch and on `config.reloaded`.
    static func load(_ json: [String: Any]) {
        var s = Settings()
        if let v = json["enabled"] as? Bool { s.enabled = v }
        if let v = json["duration_ms"] as? Int { s.durationMs = v }
        if let v = json["curve"] as? String { s.curve = v }
        if let v = json["style"] as? String { s.style = v }
        if let v = json["tab_switch"] as? String { s.tabSwitch = v }
        if let v = json["reduce_motion"] as? String { s.reduceMotion = v }
        if let v = json["unfocused_split_opacity"] as? Double { s.unfocusedOpacity = v }
        if let v = json["unfocused_split_fill"] as? String { s.unfocusedFill = color(hex: v) }
        settings = s
    }

    private static func color(hex: String) -> NSColor? {
        let digits = hex.hasPrefix("#") ? String(hex.dropFirst()) : hex
        guard digits.count == 6, let v = UInt32(digits, radix: 16) else { return nil }
        return NSColor(srgbRed: CGFloat((v >> 16) & 0xff) / 255, green: CGFloat((v >> 8) & 0xff) / 255,
                       blue: CGFloat(v & 0xff) / 255, alpha: 1)
    }

    // MARK: Focus dim

    /// How strongly unfocused tiles dim (the focus indicator): the opacity
    /// of `dimFill` laid over them. Ghostty's `unfocused-split-opacity` when
    /// its config sets one (Ghostty dims by 1 − that), else the token, which
    /// turns the mockup's focused colours into its unfocused ones.
    static var dimStrength: CGFloat {
        settings.unfocusedOpacity.map { CGFloat(1 - $0) } ?? Design.Alpha.unfocusedDim
    }

    /// What unfocused tiles fade towards: Ghostty's `unfocused-split-fill`,
    /// else the window colour, so they recede into the gutters.
    static var dimFill: NSColor { settings.unfocusedFill ?? Design.Color.dimFill }

    /// The dim's cross-fade when focus moves: quick and fixed, whatever the
    /// layout motion's length (slowed in step with it for automation).
    static var dimDuration: CFTimeInterval {
        durationOverride.map { $0 * 0.6 } ?? Design.Motion.focus
    }

    /// Whether layout changes animate right now.
    static var active: Bool {
        if let forced = enabledOverride { return forced && duration > 0 }
        guard settings.enabled, duration > 0 else { return false }
        if settings.reduceMotion == "system", NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            return false
        }
        return true
    }

    /// Layout motion (split, close, move, resize, zoom, the sidebar).
    static var duration: CFTimeInterval { durationOverride ?? Double(settings.durationMs) / 1000 }
    /// Tab and workspace switches.
    static var tabDuration: CFTimeInterval { duration * 0.75 }
    /// A focus switch between Monocle's stacked tiles: the next one
    /// dissolves in over the last, quicker than a move (nothing travels).
    static var stackDuration: CFTimeInterval { duration * 0.6 }

    static var timing: CAMediaTimingFunction {
        switch settings.curve {
        case "linear": CAMediaTimingFunction(name: .linear)
        case "ease_in_out": CAMediaTimingFunction(name: .easeInEaseOut)
        default: CAMediaTimingFunction(controlPoints: 0.2, 0.9, 0.3, 1.0) // ease-out
        }
    }

    /// The highest refresh rate of the screens we're on (120 on ProMotion).
    static var refreshRate: Float {
        Float(NSScreen.screens.map(\.maximumFramesPerSecond).max() ?? 60)
    }

    /// The transform that makes a layer whose model frame is `to` appear at
    /// `from` (scale + translate, about the layer's anchor point).
    static func transform(from: CGRect, to: CGRect, anchor: CGPoint) -> CATransform3D {
        guard to.width > 0, to.height > 0 else { return CATransform3DIdentity }
        let sx = max(from.width, 0.5) / to.width
        let sy = max(from.height, 0.5) / to.height
        // screen = to.origin + a·size + M·(p − a·size), M = scale then translate.
        let tx = from.minX - to.minX - anchor.x * to.width + sx * anchor.x * to.width
        let ty = from.minY - to.minY - anchor.y * to.height + sy * anchor.y * to.height
        return CATransform3DConcat(CATransform3DMakeScale(sx, sy, 1), CATransform3DMakeTranslation(tx, ty, 0))
    }

    // MARK: Crop geometry

    /// Which side of the clip content sticks to, per axis. In container
    /// coordinates: x `min` = left; y `min` = bottom, `max` = top.
    enum Anchor { case min, mid, max }

    /// Content sticks to the edge that moves least, so it stays still on
    /// screen while the other edge reveals or cuts it. Equal moves (a slide):
    /// top-left. `AZJ_CROP_ANCHOR=topleft` pins top-left always (comparison).
    static func anchors(from: CGRect, to: CGRect) -> (x: Anchor, y: Anchor) {
        if pinTopLeft { return (.min, .max) }
        func pick(_ dMin: CGFloat, _ dMax: CGFloat, _ sameSize: Bool, tie: Anchor) -> Anchor {
            if abs(dMin - dMax) < 0.5 { return sameSize ? tie : .mid }
            return dMin < dMax ? .min : .max
        }
        let x = pick(abs(to.minX - from.minX), abs(to.maxX - from.maxX), abs(to.width - from.width) < 0.5, tie: .min)
        let y = pick(abs(to.minY - from.minY), abs(to.maxY - from.maxY), abs(to.height - from.height) < 0.5, tie: .max)
        return (x, y)
    }

    private static let pinTopLeft = ProcessInfo.processInfo.environment["AZJ_CROP_ANCHOR"] == "topleft"

    /// The layer bounds that show a `clip`-sized window onto content laid out
    /// at `content` size, stuck to `anchor`. `flipped`: the tile's own
    /// coordinates run top-down.
    static func cropBounds(clip: CGSize, content: CGSize, anchor: (x: Anchor, y: Anchor), flipped: Bool) -> CGRect {
        func offset(_ a: Anchor, _ full: CGFloat, _ part: CGFloat) -> CGFloat {
            switch a {
            case .min: 0
            case .mid: (full - part) / 2
            case .max: full - part
            }
        }
        let x = offset(anchor.x, content.width, clip.width)
        // Bottom-up: y min (bottom) is bounds origin 0. Top-down: the top is.
        let yAnchor: Anchor = flipped ? (anchor.y == .min ? .max : anchor.y == .max ? .min : .mid) : anchor.y
        let y = offset(yAnchor, content.height, clip.height)
        return CGRect(x: x, y: y, width: max(clip.width, 0), height: max(clip.height, 0))
    }

    /// The layer position that puts a layer's frame at `frame`.
    static func position(of frame: CGRect, anchorPoint a: CGPoint) -> CGPoint {
        CGPoint(x: frame.minX + a.x * frame.width, y: frame.minY + a.y * frame.height)
    }

    /// A rect in a tile's own coordinates → where it shows in the container,
    /// with the tile's layer at `frame` showing `bounds`.
    static func toContainer(_ r: CGRect, frame: CGRect, bounds: CGRect, flipped: Bool) -> CGRect {
        let x = frame.minX + (r.minX - bounds.minX)
        let y = flipped
            ? frame.maxY - (r.maxY - bounds.minY)
            : frame.minY + (r.minY - bounds.minY)
        return CGRect(x: x, y: y, width: r.width, height: r.height)
    }

    /// The inverse of `toContainer`.
    static func toTile(_ r: CGRect, frame: CGRect, bounds: CGRect, flipped: Bool) -> CGRect {
        let x = r.minX - frame.minX + bounds.minX
        let y = flipped
            ? (frame.maxY - r.maxY + bounds.minY)
            : (r.minY - frame.minY + bounds.minY)
        return CGRect(x: x, y: y, width: r.width, height: r.height)
    }

    /// Animations share these: duration, curve, full frame rate.
    static func configure(_ animation: CAAnimation, duration: CFTimeInterval) {
        animation.duration = duration
        animation.timingFunction = timing
        let rate = refreshRate
        animation.preferredFrameRateRange = CAFrameRateRange(minimum: rate / 2, maximum: rate, preferred: rate)
    }
}

/// Tile views that need to know about motion. Terminal tiles freeze behind a
/// snapshot of their last frame while they move (crop, scale, outline),
/// because their PTY is resized at the start of the motion and the redraw
/// (via herdr) takes ~100 ms; with reflow they resize live instead.
@MainActor
protocol MotionAware: AnyObject {
    /// Reflow: the container is about to step this tile's frame towards a
    /// frame of `final` size, once per display frame. Repeated calls (a
    /// retarget) only move the target.
    func liveResizeWillStart(final: CGSize)
    /// Reflow: the tile is at its final frame (or the motion was cut short):
    /// size everything exactly to the bounds now.
    func liveResizeDidEnd()
    /// Reflow: whether moving resizes something (a running terminal).
    var resizesLive: Bool { get }
    /// Reflow: the size the tile will have at `time` (`Motion.reflowLead`
    /// ahead, or the final frame once that's past the end: `last`). Called
    /// each display frame before the tile's frame steps; the content may
    /// take that size early.
    func liveResizeAhead(to size: CGSize, at time: CFTimeInterval, last: Bool)
    /// About to move. `freeze`: cover live content with a snapshot of the last
    /// frame (the tile changes size, or is closing). Returns whether it froze.
    /// `fill`: the snapshot follows the tile's bounds (scale style); otherwise
    /// it stays at 1:1 where `placeSnapshot` puts it.
    @discardableResult func motionWillStart(freeze: Bool, fill: Bool) -> Bool
    /// The snapshot's frame in the tile's coordinates, while frozen.
    var snapshotFrame: CGRect? { get }
    /// Puts the 1:1 snapshot at `frame` (tile coordinates) over a plain
    /// background spanning `cover`, which hides the live surface around it.
    func placeSnapshot(_ frame: CGRect, cover: CGRect)
    /// Reveal live content as soon as it has drawn at its new size, even
    /// mid-motion (a growing tile: live content is never stretched).
    func revealWhenFresh(clock: MotionClock)
    /// The motion is over: reveal live content once it's ready, polling on
    /// `clock` (once per frame).
    func motionDidEnd(clock: MotionClock)
    /// Stop now: reveal live content immediately.
    func motionCancelled()
    /// How many times the PTY has been resized (stats: one per motion).
    var surfaceResizes: Int { get }
    /// Height of a strip above the content (a tile header, F28) that 1:1
    /// snapshots sit below.
    var headerInset: CGFloat { get }
    /// Margin left of the content (a terminal's padding in its card, D.5).
    var contentLeft: CGFloat { get }
}

extension MotionAware {
    func liveResizeWillStart(final: CGSize) {}
    func liveResizeDidEnd() {}
    var resizesLive: Bool { false }
    func liveResizeAhead(to size: CGSize, at time: CFTimeInterval, last: Bool) {}
    var headerInset: CGFloat { 0 }
    var contentLeft: CGFloat { 0 }
}

/// One per-frame callback list, driven by the container's display link.
@MainActor
final class MotionClock {
    typealias Handler = (CFTimeInterval) -> Bool
    private var handlers: [Handler] = []
    var onChange: (() -> Void)?

    var isEmpty: Bool { handlers.isEmpty }

    /// `handler` runs every frame until it returns false.
    func add(_ handler: @escaping Handler) {
        handlers.append(handler)
        onChange?()
    }

    func tick(_ now: CFTimeInterval) {
        handlers = handlers.filter { $0(now) }
    }
}

/// Frame pacing and resize counts while motions run, for automation
/// (`anim stats`) and `docs/measurements.md`.
@MainActor
final class MotionStats {
    static let shared = MotionStats()

    private(set) var animations: [String: Int] = [:]
    private(set) var intervals: [Double] = []
    private(set) var setupMs: [Double] = []
    private(set) var retargets = 0
    private(set) var expectedResizes = 0
    private(set) var actualResizes = 0
    /// Reflow: resizes on top of one per motion (the live ones).
    private(set) var liveResizes = 0
    private(set) var mismatches: [String] = []
    /// Reflow: every live and final terminal resize (herdr pane, when asked,
    /// when the tile's frame has that size, the grid), and when motions
    /// ended, for `automation/reflow-lag.py` (text-to-frame lag against
    /// SIGWINCH logs in the panes).
    private var reflowResizes: [[Any]] = []
    private var reflowEnds: [Double] = []
    private var lastTick: CFTimeInterval?
    /// When the last tick's callback ran (not its vsync timestamp): a missed
    /// tick blames whatever the main thread did between two callbacks.
    private var lastCallback: CFTimeInterval?
    private(set) var currentKind = ""
    private(set) var droppedByKind: [String: Int] = [:]
    /// Per dropped tick: what overlapped it (`Perf` spans), summed by name.
    private(set) var dropCauses: [String: Double] = [:]
    private(set) var drops: [[String: Any]] = []
    /// Until when something is moving (any window), for `Perf`'s counts.
    var movingUntil: CFTimeInterval = 0
    var moving: Bool { CACurrentMediaTime() < movingUntil + 0.02 }

    func reset() {
        animations = [:]
        intervals = []
        setupMs = []
        retargets = 0
        expectedResizes = 0
        actualResizes = 0
        liveResizes = 0
        mismatches = []
        reflowResizes = []
        reflowEnds = []
        lastTick = nil
        lastCallback = nil
        droppedByKind = [:]
        dropCauses = [:]
        drops = []
        Perf.reset()
    }

    func started(kind: String, setup: Double) {
        currentKind = kind
        animations[kind, default: 0] += 1
        setupMs.append(setup * 1000)
    }

    func retargeted() { retargets += 1 }

    /// Reflow: a terminal's surface took a new size at `now`, the size its
    /// tile has at `frameTime` (later, with a lead).
    func reflowResized(pane: String, now: CFTimeInterval, frameTime: CFTimeInterval, grid: (Int, Int)) {
        guard reflowResizes.count < 20_000 else { return }
        reflowResizes.append([pane, now, frameTime, grid.0, grid.1])
    }

    func reflowEnded(at time: CFTimeInterval) {
        guard reflowEnds.count < 5_000 else { return }
        reflowEnds.append(time)
    }

    /// A display-link tick during motion; `now` is when its callback ran.
    func tick(_ timestamp: CFTimeInterval, now: CFTimeInterval) {
        if let last = lastTick {
            let ms = (timestamp - last) * 1000
            intervals.append(ms)
            if ms > 1500 / Double(Motion.refreshRate) {
                droppedByKind[currentKind, default: 0] += 1
                blame(interval: ms, from: lastCallback ?? last, to: now)
            }
        }
        lastTick = timestamp
        lastCallback = now
    }

    private func blame(interval: Double, from: CFTimeInterval, to: CFTimeInterval) {
        let causes = Perf.overlap(from: from, to: to)
        for (name, ms) in causes { dropCauses[name, default: 0] += ms }
        guard drops.count < 40 else { return }
        let top = causes.sorted { $0.value > $1.value }.prefix(8)
            .map { "\($0.key) \(round2($0.value))" }
        drops.append(["kind": currentKind, "interval_ms": round2(interval), "causes": top])
    }

    /// Motion over (or paused): the next tick starts a new interval run.
    func idle() {
        lastTick = nil
        lastCallback = nil
    }

    /// One terminal's resizes over a motion chain: one per motion that
    /// changed its size (`expected`). Motions that retarget each other
    /// faster than the terminal's 50 ms resize coalescing can merge, so a
    /// chain needs at least `minimum` (1 when it ends at a new size) and at
    /// most `expected`; a chain of one motion is exact.
    /// Reflow: `maximum` allows the live resizes (a few per motion, see
    /// `Motion.maxLiveResizes`) on top.
    func resizes(tile: String, expected: Int, minimum: Int, maximum: Int? = nil, actual: Int, kind: String) {
        let maximum = maximum ?? expected
        expectedResizes += expected
        actualResizes += actual
        if maximum > expected { liveResizes += max(actual - expected, 0) }
        if actual > maximum || actual < min(minimum, expected) {
            let low = min(minimum, expected)
            let want = low < maximum ? "\(low) to \(maximum)" : "\(expected)"
            mismatches.append("\(kind) \(Short.id(tile)): expected \(want) resize(s), saw \(actual)")
        }
    }

    func summary() -> [String: Any] {
        let refresh = Double(Motion.refreshRate)
        let frame = 1000 / refresh
        let dropped = intervals.filter { $0 > frame * 1.5 }.count
        let sorted = intervals.sorted()
        func pct(_ p: Double) -> Double {
            sorted.isEmpty ? 0 : sorted[min(sorted.count - 1, Int(Double(sorted.count - 1) * p))]
        }
        let setups = setupMs.sorted()
        return [
            "refresh_hz": refresh,
            "animations": animations,
            "animation_count": animations.values.reduce(0, +),
            "retargets": retargets,
            "ticks": intervals.count,
            "dropped_frames": dropped,
            "dropped_by_kind": droppedByKind,
            "interval_median_ms": round2(pct(0.5)),
            "interval_p99_ms": round2(pct(0.99)),
            "interval_max_ms": round2(sorted.last ?? 0),
            "setup_median_ms": round2(setups.isEmpty ? 0 : setups[setups.count / 2]),
            "setup_max_ms": round2(setups.last ?? 0),
            "pty_resizes_expected": expectedResizes,
            "pty_resizes_actual": actualResizes,
            "pty_resizes_live": liveResizes,
            "style": "\(Motion.style)",
            "reflow_interval_ms": round2(Motion.reflowInterval * 1000),
            "reflow_lead_ms": round2(Motion.reflowLead * 1000),
            "reflow_resizes": reflowResizes,
            "reflow_ends": reflowEnds,
            "resize_mismatches": mismatches,
            "drop_causes_ms": dropCauses.mapValues(round2),
            "drops": drops,
            "perf": Perf.summary(),
        ]
    }

    private func round2(_ v: Double) -> Double { (v * 100).rounded() / 100 }
}
