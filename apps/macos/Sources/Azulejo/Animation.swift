import AppKit
import QuartzCore

/// Layout motion settings (`[animation]` in the config, F4) and the helpers
/// the tile container animates with.
///
/// How motion works (ADR 0004): every tile goes to its final frame at the
/// start of a motion — so a terminal's PTY is resized exactly once, while a
/// snapshot covers it — and a transform animation carries it visually from
/// where it was to where it is. Nothing re-lays out per frame; Core Animation
/// runs the motion in the render server.
@MainActor
enum Motion {
    struct Settings: Equatable {
        var enabled = true
        var durationMs = 200
        var curve = "ease_out"
        var tabSwitch = "fade"
        var dimInactive = false
        var reduceMotion = "system"
    }

    private(set) static var settings = Settings()
    /// Automation overrides (slow motion for mid-animation screenshots, or off).
    static var durationOverride: CFTimeInterval?
    static var enabledOverride: Bool?

    /// Reads `azj_core_animation`'s JSON. Called at launch and on `config.reloaded`.
    static func load(_ json: [String: Any]) {
        var s = Settings()
        if let v = json["enabled"] as? Bool { s.enabled = v }
        if let v = json["duration_ms"] as? Int { s.durationMs = v }
        if let v = json["curve"] as? String { s.curve = v }
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
    @discardableResult func motionWillStart(freeze: Bool) -> Bool
    /// The motion is over: reveal live content once it's ready, polling on
    /// `clock` (once per frame).
    func motionDidEnd(clock: MotionClock)
    /// Stop now: reveal live content immediately.
    func motionCancelled()
    /// How many times the PTY has been resized (stats: one per motion).
    var surfaceResizes: Int { get }
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
    private var currentKind = ""
    private(set) var droppedByKind: [String: Int] = [:]

    func reset() {
        animations = [:]
        intervals = []
        setupMs = []
        retargets = 0
        expectedResizes = 0
        actualResizes = 0
        mismatches = []
        lastTick = nil
        droppedByKind = [:]
    }

    func started(kind: String, setup: Double) {
        currentKind = kind
        animations[kind, default: 0] += 1
        setupMs.append(setup * 1000)
    }

    func retargeted() { retargets += 1 }

    /// A display-link tick during motion.
    func tick(_ timestamp: CFTimeInterval) {
        if let last = lastTick {
            let ms = (timestamp - last) * 1000
            intervals.append(ms)
            if ms > 1500 / Double(Motion.refreshRate) { droppedByKind[currentKind, default: 0] += 1 }
        }
        lastTick = timestamp
    }

    /// Motion over (or paused): the next tick starts a new interval run.
    func idle() { lastTick = nil }

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
        ]
    }

    private func round2(_ v: Double) -> Double { (v * 100).rounded() / 100 }
}
