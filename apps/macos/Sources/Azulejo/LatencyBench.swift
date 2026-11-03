import AppKit
import CoreMedia
import GhosttyKit
import ScreenCaptureKit

/// Keystroke → glyph bench on the real app's terminal tile (PLAN 1.13): the
/// 0.4 harness method (`scripts/latency/harness/Sources/Bench.swift`) run
/// against a `TerminalSurfaceView` attached through herdr. Off unless
/// `AZULEJO_LATENCY_BENCH=<samples>` is set; then it waits
/// `AZULEJO_LATENCY_DELAY` seconds (default 3), measures the first terminal
/// tile it finds, writes `AZULEJO_LATENCY_CSV` and quits the app.
///
/// t0 is taken right before `ghostty_surface_key` on the main thread; the end
/// is the `displayTime` of the first window-only capture frame whose pixels
/// changed in the glyph's box. No OS input is posted and no window is raised.
final class LatencyBench: NSObject, SCStreamOutput, @unchecked Sendable {
    private let samples: Int
    private let label: String
    private let csv: String
    private let log: @Sendable (String) -> Void
    private let cond = NSCondition()
    private var frame: Frame?
    private var seq = 0
    private nonisolated(unsafe) weak var window: NSWindow?
    private nonisolated(unsafe) weak var view: TerminalSurfaceView?

    struct Frame { let px: [UInt8]; let w: Int; let h: Int; let stride: Int; let t: UInt64; let seq: Int }
    struct Box { var x0, y0, x1, y1: Int }

    private init(samples: Int, label: String, csv: String, log: @escaping @Sendable (String) -> Void) {
        self.samples = samples; self.label = label; self.csv = csv; self.log = log
    }

    @MainActor
    static func startIfRequested(log: @escaping @MainActor (String) -> Void) {
        let env = ProcessInfo.processInfo.environment
        guard let n = env["AZULEJO_LATENCY_BENCH"].flatMap(Int.init), n > 0 else { return }
        let delay = env["AZULEJO_LATENCY_DELAY"].flatMap(Double.init) ?? 3
        let sendLog: @Sendable (String) -> Void = { line in
            DispatchQueue.main.async { MainActor.assumeIsolated { log(line) } }
            FileHandle.standardError.write(Data("azulejo: \(line)\n".utf8))
        }
        let bench = LatencyBench(
            samples: n, label: env["AZULEJO_LATENCY_LABEL"] ?? "app",
            csv: env["AZULEJO_LATENCY_CSV"] ?? NSTemporaryDirectory() + "azulejo-latency.csv", log: sendLog)
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
            MainActor.assumeIsolated {
                guard let (window, view) = firstTerminal() else {
                    sendLog("latency: no attached terminal surface found")
                    NSApp.terminate(nil)
                    return
                }
                bench.window = window
                bench.view = view
                // The window can't become key without activating the app
                // (never done in these runs), so give libghostty the focus a
                // key window would, as the 0.4 harness does. Unfocused
                // surfaces render on a slower path.
                if let surface = view.surface { ghostty_surface_set_focus(surface, true) }
                if let app = TerminalRuntime.shared?.app { ghostty_app_set_focus(app, true) }
                Task.detached { await bench.run() }
            }
        }
    }

    @MainActor
    private static func firstTerminal() -> (NSWindow, TerminalSurfaceView)? {
        func find(_ v: NSView) -> TerminalSurfaceView? {
            if let s = v as? TerminalSurfaceView, s.surface != nil, !s.isHiddenOrHasHiddenAncestor { return s }
            for sub in v.subviews { if let s = find(sub) { return s } }
            return nil
        }
        for window in NSApp.windows where window.isVisible {
            if let content = window.contentView, let s = find(content) { return (window, s) }
        }
        return nil
    }

    // MARK: capture

    func stream(_ s: SCStream, didOutputSampleBuffer sb: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen,
              let atts = CMSampleBufferGetSampleAttachmentsArray(sb, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let info = atts.first,
              let raw = info[.status] as? Int, SCFrameStatus(rawValue: raw) == .complete,
              let pb = CMSampleBufferGetImageBuffer(sb) else { return }
        let dt = (info[.displayTime] as? UInt64) ?? mach_absolute_time()
        CVPixelBufferLockBaseAddress(pb, .readOnly)
        let w = CVPixelBufferGetWidth(pb), h = CVPixelBufferGetHeight(pb), st = CVPixelBufferGetBytesPerRow(pb)
        let base = CVPixelBufferGetBaseAddress(pb)!.assumingMemoryBound(to: UInt8.self)
        let copy = Array(UnsafeBufferPointer(start: base, count: st * h))
        CVPixelBufferUnlockBaseAddress(pb, .readOnly)
        cond.lock()
        seq += 1
        frame = Frame(px: copy, w: w, h: h, stride: st, t: dt, seq: seq)
        cond.broadcast()
        cond.unlock()
    }

    private func latest() -> Frame? { cond.lock(); defer { cond.unlock() }; return frame }

    private func next(after: Int, timeout: Double) -> Frame? {
        let deadline = Date(timeIntervalSinceNow: timeout)
        cond.lock(); defer { cond.unlock() }
        while seq <= after { if !cond.wait(until: deadline) { return nil } }
        return frame
    }

    // MARK: keys (in-process only)

    /// Inject on the main thread; returns the mach time taken just before.
    private func inject(_ keycode: UInt32, _ text: String) -> UInt64 {
        DispatchQueue.main.sync {
            MainActor.assumeIsolated {
                let t = mach_absolute_time()
                guard let surface = view?.surface else { return t }
                var k = ghostty_input_key_s()
                k.action = GHOSTTY_ACTION_PRESS
                k.keycode = keycode
                k.mods = GHOSTTY_MODS_NONE
                k.consumed_mods = GHOSTTY_MODS_NONE
                k.composing = false
                k.unshifted_codepoint = text.unicodeScalars.first!.value
                text.withCString { ptr in
                    k.text = ptr
                    _ = ghostty_surface_key(surface, k)
                }
                k.action = GHOSTTY_ACTION_RELEASE
                k.text = nil
                _ = ghostty_surface_key(surface, k)
                return t
            }
        }
    }
    private let kX: UInt32 = 7, kDelete: UInt32 = 51

    // MARK: diff

    private func box(_ a: Frame, _ b: Frame) -> Box? {
        guard a.w == b.w, a.h == b.h else { return nil }
        var r = Box(x0: .max, y0: .max, x1: -1, y1: -1)
        for y in 0..<a.h {
            let row = y * a.stride
            for x in 0..<a.w {
                let i = row + x * 4
                if a.px[i] != b.px[i] || a.px[i+1] != b.px[i+1] || a.px[i+2] != b.px[i+2] {
                    r.x0 = min(r.x0, x); r.y0 = min(r.y0, y); r.x1 = max(r.x1, x); r.y1 = max(r.y1, y)
                }
            }
        }
        return r.x1 < 0 ? nil : r
    }

    private func differs(_ a: Frame, _ b: Frame, _ r: Box) -> Bool {
        guard a.w == b.w, a.h == b.h else { return true }
        for y in r.y0...r.y1 {
            let row = y * a.stride
            for x in r.x0...r.x1 {
                let i = row + x * 4
                if a.px[i] != b.px[i] || a.px[i+1] != b.px[i+1] || a.px[i+2] != b.px[i+2] { return true }
            }
        }
        return false
    }

    // MARK: run

    private func run() async {
        do {
            try await measure()
        } catch {
            log("latency [\(label)] error: \(error)")
        }
        await MainActor.run { NSApp.terminate(nil) }
    }

    private func measure() async throws {
        var tb = mach_timebase_info_data_t()
        mach_timebase_info(&tb)
        func ms(_ ticks: UInt64) -> Double { Double(ticks) * Double(tb.numer) / Double(tb.denom) / 1e6 }

        let (wid, size, scale, fps) = await MainActor.run {
            (CGWindowID(window?.windowNumber ?? 0), window?.frame.size ?? .zero,
             window?.backingScaleFactor ?? 2, NSScreen.main?.maximumFramesPerSecond ?? 60)
        }
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
        guard let scw = content.windows.first(where: { $0.windowID == wid }) else {
            throw NSError(domain: "bench", code: 1, userInfo: [NSLocalizedDescriptionKey: "own window \(wid) not in SCShareableContent"])
        }
        // Window-only capture: sees only our own window, even when occluded.
        let filter = SCContentFilter(desktopIndependentWindow: scw)
        let cfg = SCStreamConfiguration()
        cfg.width = Int(size.width * scale)
        cfg.height = Int(size.height * scale)
        cfg.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(fps))
        cfg.queueDepth = 4
        cfg.showsCursor = false
        cfg.pixelFormat = kCVPixelFormatType_32BGRA
        let s = SCStream(filter: filter, configuration: cfg, delegate: nil)
        try s.addStreamOutput(self, type: .screen, sampleHandlerQueue: DispatchQueue(label: "cap", qos: .userInteractive))
        try await s.startCapture()
        // Not stopped: the app quits right after the run.

        guard next(after: 0, timeout: 3) != nil else {
            throw NSError(domain: "bench", code: 2, userInfo: [NSLocalizedDescriptionKey: "no frames from window capture"])
        }
        try await Task.sleep(nanoseconds: 500_000_000)

        // Idle noise: frames that change with no input (cursor blink etc.).
        let idle0 = latest()!
        var idleChanged = 0
        var last = idle0.seq
        let idleEnd = Date(timeIntervalSinceNow: 1.0)
        while Date() < idleEnd {
            if let f = next(after: last, timeout: 0.2) { last = f.seq; if box(idle0, f) != nil { idleChanged += 1 } }
        }

        // Calibrate: where does the glyph land? Retry while the child starts up.
        var found: Box?
        var after = latest()!
        for attempt in 0..<5 where found == nil {
            let before = latest()!
            _ = inject(kX, "x")
            try await Task.sleep(nanoseconds: 400_000_000)
            after = latest()!
            found = box(before, after)
            if found == nil {
                log("latency [\(label)] calibration attempt \(attempt + 1): no change yet")
                try await Task.sleep(nanoseconds: 1_000_000_000)
            }
        }
        guard var r = found else {
            throw NSError(domain: "bench", code: 3, userInfo: [NSLocalizedDescriptionKey: "typing 'x' changed nothing in the window"])
        }
        _ = inject(kDelete, "\u{7f}")
        try await Task.sleep(nanoseconds: 400_000_000)
        r.x0 = max(0, r.x0 - 2); r.y0 = max(0, r.y0 - 2)
        r.x1 = min(after.w - 1, r.x1 + 2); r.y1 = min(after.h - 1, r.y1 + 2)
        log("latency [\(label)] window \(Int(size.width))x\(Int(size.height))pt, capture \(cfg.width)x\(cfg.height)px, \(fps) Hz, glyph box \(r.x0),\(r.y0)-\(r.x1),\(r.y1), idle frames changed: \(idleChanged)")

        var lat: [Double] = []
        var misses = 0
        var lines = ["label,sample,latency_ms,capture_lag_ms"]
        for i in 0..<samples {
            // Random settle so injections don't phase-lock to vsync.
            try await Task.sleep(nanoseconds: UInt64(Double.random(in: 0.12...0.22) * 1e9))
            let base = latest()!
            let t0 = inject(kX, "x")
            var hit: Frame?
            var seen = base.seq
            let deadline = Date(timeIntervalSinceNow: 1.0)
            while Date() < deadline, hit == nil {
                guard let f = next(after: seen, timeout: 0.5) else { break }
                seen = f.seq
                if f.t > t0, differs(base, f, r) { hit = f }
            }
            let tcb = mach_absolute_time()
            if let f = hit {
                let l = ms(f.t - t0)
                lat.append(l)
                lines.append("\(label),\(i),\(String(format: "%.3f", l)),\(String(format: "%.3f", ms(tcb - f.t)))")
            } else {
                misses += 1
                lines.append("\(label),\(i),,")
            }
            _ = inject(kDelete, "\u{7f}")
            let back = Date(timeIntervalSinceNow: 1.0)
            var s2 = latest()!.seq
            while Date() < back {
                if !differs(base, latest()!, r) { break }
                if let f = next(after: s2, timeout: 0.3) { s2 = f.seq }
            }
        }
        try lines.joined(separator: "\n").appending("\n").write(toFile: csv, atomically: true, encoding: .utf8)

        let v = lat.sorted()
        func p(_ q: Double) -> Double { v.isEmpty ? .nan : v[min(v.count - 1, Int((q * Double(v.count - 1)).rounded()))] }
        log(String(format: "latency [%@] n=%d misses=%d  min %.2f  median %.2f  p95 %.2f  p99 %.2f  max %.2f ms",
                   label, v.count, misses, v.first ?? .nan, p(0.5), p(0.95), p(0.99), v.last ?? .nan))
    }
}
