import AppKit
import QuartzCore

/// Where the main thread's time goes while tiles move, so a missed
/// display-link tick can be blamed on something (`anim stats` → `perf`).
///
/// Named spans wrap the app's own work (a session fetch, a sidebar rebuild,
/// an event routed); run-loop observers add the gaps between them: a whole
/// busy run-loop pass, and Core Animation's commit (layout and drawing of
/// dirty views) at the end of it. Cheap enough to stay on: a few timestamps
/// per pass into a small ring.
@MainActor
enum Perf {
    struct Span {
        let name: String
        let start: CFTimeInterval
        let end: CFTimeInterval
    }

    private static var ring: [Span] = []
    private static let capacity = 2048
    private static var next = 0

    /// Per span name: how often, total and longest, since `reset`.
    private(set) static var totals: [String: (count: Int, total: Double, max: Double)] = [:]
    /// Events the core sent while something moved, by name.
    private(set) static var motionEvents: [String: Int] = [:]
    /// Session fetches (UI refreshes) while something moved.
    private(set) static var motionRefreshes = 0

    /// Main run-loop passes that ran while something moved, after the pass
    /// that started the motion: each one over a frame is a missed tick when
    /// the display is on. Measurable with the display asleep too.
    private(set) static var motionPasses: [Double] = []
    private(set) static var longPasses: [[String: Any]] = []
    /// When the latest motion started (its first pass is set-up, not drift).
    static var motionStarted: CFTimeInterval = 0

    static func reset() {
        totals = [:]
        motionEvents = [:]
        motionRefreshes = 0
        motionPasses = []
        longPasses = []
    }

    private static func notePass(_ start: CFTimeInterval, _ end: CFTimeInterval) {
        guard MotionStats.shared.moving, start > motionStarted else { return }
        let ms = (end - start) * 1000
        motionPasses.append(ms)
        guard ms > 1000 / Double(Motion.refreshRate) / 2, longPasses.count < 40 else { return }
        let top = overlap(from: start, to: end).filter { $0.key != "runloop" }
            .sorted { $0.value > $1.value }.prefix(8)
            .map { "\($0.key) \((($0.value) * 100).rounded() / 100)" }
        longPasses.append(["ms": (ms * 100).rounded() / 100, "spans": top, "kind": MotionStats.shared.currentKind])
    }

    /// Times `body` as `name`.
    @discardableResult
    static func measure<T>(_ name: String, _ body: () throws -> T) rethrows -> T {
        let start = CACurrentMediaTime()
        defer { record(name, start, CACurrentMediaTime()) }
        return try body()
    }

    static func record(_ name: String, _ start: CFTimeInterval, _ end: CFTimeInterval) {
        let span = Span(name: name, start: start, end: end)
        if ring.count < capacity {
            ring.append(span)
        } else {
            ring[next] = span
        }
        next = (next + 1) % capacity
        let ms = (end - start) * 1000
        var t = totals[name] ?? (0, 0, 0)
        t.count += 1
        t.total += ms
        t.max = Swift.max(t.max, ms)
        totals[name] = t
    }

    /// How long each kind of span overlapped `from`…`to`, in ms.
    static func overlap(from: CFTimeInterval, to: CFTimeInterval) -> [String: Double] {
        var out: [String: Double] = [:]
        for span in ring where span.end > from && span.start < to {
            let ms = (Swift.min(span.end, to) - Swift.max(span.start, from)) * 1000
            if ms > 0.05 { out[span.name, default: 0] += ms }
        }
        return out
    }

    static func noteEvent(_ json: String, moving: Bool) {
        guard moving else { return }
        motionEvents[eventName(json), default: 0] += 1
    }

    static func noteRefresh(moving: Bool) {
        if moving { motionRefreshes += 1 }
    }

    /// `{"event":"tile.updated",…}` → `tile.updated`, without parsing it all.
    static func eventName(_ json: String) -> String {
        guard let r = json.range(of: "\"event\":\"") else { return "?" }
        let rest = json[r.upperBound...]
        return rest.firstIndex(of: "\"").map { String(rest[..<$0]) } ?? "?"
    }

    // MARK: Run loop

    private static var passStart: CFTimeInterval = 0
    private static var commitStart: CFTimeInterval = 0
    private static var observers: [CFRunLoopObserver] = []

    /// Records every busy main run-loop pass ("runloop") and Core Animation's
    /// display cycle and commit at its end ("display.commit").
    static func installRunLoopObservers() {
        guard observers.isEmpty else { return }
        let loop = CFRunLoopGetMain()
        func add(_ activities: CFRunLoopActivity, order: CFIndex, _ handler: @escaping (CFRunLoopActivity) -> Void) {
            guard let observer = CFRunLoopObserverCreateWithHandler(
                kCFAllocatorDefault, activities.rawValue, true, order, { _, activity in handler(activity) })
            else { return }
            CFRunLoopAddObserver(loop, observer, .commonModes)
            observers.append(observer)
        }
        // Earliest: a pass starts when the loop wakes; ends just before it sleeps.
        add([.afterWaiting], order: Int.min) { _ in
            MainActor.assumeIsolated { passStart = CACurrentMediaTime() }
        }
        // Before sleeping, AppKit runs its display cycle (layout, drawing) and
        // Core Animation commits (order 2,000,000): bracket all of it.
        add([.beforeWaiting], order: Int.min) { _ in
            MainActor.assumeIsolated { commitStart = CACurrentMediaTime() }
        }
        add([.beforeWaiting], order: Int.max) { _ in
            MainActor.assumeIsolated {
                let now = CACurrentMediaTime()
                if commitStart > 0 { record("display.commit", commitStart, now) }
                if passStart > 0 {
                    record("runloop", passStart, now)
                    notePass(passStart, now)
                }
                commitStart = 0
                passStart = 0
            }
        }
    }

    /// For `anim stats`: phase timings, worst first.
    static func summary() -> [String: Any] {
        var phases: [String: Any] = [:]
        for (name, t) in totals {
            phases[name] = [
                "count": t.count,
                "total_ms": (t.total * 100).rounded() / 100,
                "mean_ms": t.count > 0 ? ((t.total / Double(t.count)) * 1000).rounded() / 1000 : 0,
                "max_ms": (t.max * 100).rounded() / 100,
            ]
        }
        let frame = 1000 / Double(Motion.refreshRate)
        let sorted = motionPasses.sorted()
        return [
            "motion_passes": motionPasses.count,
            "motion_passes_over_half_frame": motionPasses.filter { $0 > frame / 2 }.count,
            "motion_passes_over_frame": motionPasses.filter { $0 > frame }.count,
            "motion_pass_max_ms": ((sorted.last ?? 0) * 100).rounded() / 100,
            "motion_pass_p99_ms": sorted.isEmpty ? 0
                : ((sorted[min(sorted.count - 1, Int(Double(sorted.count - 1) * 0.99))]) * 100).rounded() / 100,
            "long_passes": longPasses,
            "phases": phases,
            "events_while_moving": motionEvents,
            "refreshes_while_moving": motionRefreshes,
        ]
    }
}
