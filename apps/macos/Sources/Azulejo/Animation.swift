import AppKit
import QuartzCore

/// Layout motion settings (`[animation]` in the config, F4) and the helpers
/// the tile container animates with.
///
/// How motion works (ADR 0004): every tile goes to its final frame at the
/// start of a motion — so a terminal's PTY is resized exactly once, while a
/// snapshot covers it — and Core Animation carries it visually from where it
/// was to where it is. Nothing re-lays out per frame.
///
/// `style` says what the content does on the way:
/// - "crop" (default): the tile's frame is a clip. Its bounds and position
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
        var style = "crop"
        var tabSwitch = "fade"
        var dimInactive = false
        var reduceMotion = "system"
    }

    private(set) static var settings = Settings()
    /// Automation overrides (slow motion for mid-animation screenshots, or off).
    static var durationOverride: CFTimeInterval?
    static var enabledOverride: Bool?
    static var styleOverride: String?

    enum Style { case crop, scale, outline }

    static var style: Style {
        switch styleOverride ?? settings.style {
        case "scale": .scale
        case "outline": .outline
        default: .crop
        }
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
        if let v = json["dim_inactive"] as? Bool { s.dimInactive = v }
        if let v = json["reduce_motion"] as? String { s.reduceMotion = v }
        settings = s
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

    /// Layout motion (split, close, move, resize, zoom).
    static var duration: CFTimeInterval { durationOverride ?? Double(settings.durationMs) / 1000 }
    /// The focus ring moving on its own (focus changes with no layout change).
    static var focusDuration: CFTimeInterval { duration * 0.6 }
    /// Tab and workspace switches.
    static var tabDuration: CFTimeInterval { duration * 0.75 }

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
/// snapshot of their last frame while they move, because their PTY is resized
/// at the start of the motion and the redraw (via herdr) takes ~100 ms.
@MainActor
protocol MotionAware: AnyObject {
    /// About to move. `freeze`: cover live content with a snapshot of the last
    /// frame (the tile changes size, or is closing). Returns whether it froze.
    /// `fill`: the snapshot follows the tile's bounds (scale style); otherwise
    /// it stays at 1:1 where `placeSnapshot` puts it.
    @discardableResult func motionWillStart(freeze: Bool, fill: Bool) -> Bool
    /// The snapshot's frame in the tile's coordinates, while frozen.
    var snapshotFrame: CGRect? { get }
    /// The frozen frame's contents (an IOSurface), for a cross-fade ghost.
    var snapshotContents: Any? { get }
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
    private(set) var mismatches: [String] = []
    private var lastTick: CFTimeInterval?
    /// When the last tick's callback ran (not its vsync timestamp): a missed
    /// tick blames whatever the main thread did between two callbacks.
    private var lastCallback: CFTimeInterval?
    private var currentKind = ""
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
        mismatches = []
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

    func resizes(tile: String, expected: Int, actual: Int, kind: String) {
        expectedResizes += expected
        actualResizes += actual
        if expected != actual {
            mismatches.append("\(kind) \(Short.id(tile)): expected \(expected) resize(s), saw \(actual)")
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
            "resize_mismatches": mismatches,
            "drop_causes_ms": dropCauses.mapValues(round2),
            "drops": drops,
            "perf": Perf.summary(),
        ]
    }

    private func round2(_ v: Double) -> Double { (v * 100).rounded() / 100 }
}
