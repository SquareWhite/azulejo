import AppKit
import CoreMedia
import GhosttyKit
import ScreenCaptureKit

/// In-process keystroke → glyph bench.
///
/// t0 is taken right before `ghostty_surface_key` on the main thread; the end
/// is the `displayTime` of the first captured window frame where the glyph's
/// pixels changed. The OS input path (HID → WindowServer → NSEvent → keyDown),
/// the same for every config, is not included.
final class Bench: NSObject, SCStreamOutput {
    let window: NSWindow
    let view: SurfaceView
    let samples: Int
    let label: String
    let csv: String
    private var stream: SCStream?
    private let cond = NSCondition()
    private var frame: Frame?
    private var seq = 0

    struct Frame { let px: [UInt8]; let w: Int; let h: Int; let stride: Int; let t: UInt64; let seq: Int }
    struct Box { var x0, y0, x1, y1: Int }

    init(window: NSWindow, view: SurfaceView, samples: Int, label: String, csv: String) {
        self.window = window; self.view = view; self.samples = samples; self.label = label; self.csv = csv
    }

    func start(after delay: Double) {
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
            Task { await self.run() }
        }
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

    private func key(_ keycode: UInt32, _ text: String) {
        guard let surface = view.surface else { return }
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
    }

    /// Inject on the main thread; returns the mach time taken just before.
    private func inject(_ keycode: UInt32, _ text: String) -> UInt64 {
        DispatchQueue.main.sync {
            let t = mach_absolute_time()
            key(keycode, text)
            return t
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

    private func dump(_ f: Frame, _ path: String) {
        var px = f.px
        let ctx = CGContext(data: &px, width: f.w, height: f.h, bitsPerComponent: 8, bytesPerRow: f.stride,
                            space: CGColorSpaceCreateDeviceRGB(),
                            bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)!
        let rep = NSBitmapImageRep(cgImage: ctx.makeImage()!)
        try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
    }

    // MARK: run

    private func run() async {
        do {
            try await measure()
        } catch {
            print("[\(label)] error: \(error)")
        }
        await MainActor.run { NSApp.terminate(nil) }
    }

    private func measure() async throws {
        var tb = mach_timebase_info_data_t()
        mach_timebase_info(&tb)
        func ms(_ ticks: UInt64) -> Double { Double(ticks) * Double(tb.numer) / Double(tb.denom) / 1e6 }

        let wid = await MainActor.run { CGWindowID(window.windowNumber) }
        let (size, scale) = await MainActor.run { (window.frame.size, window.backingScaleFactor) }
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
        guard let scw = content.windows.first(where: { $0.windowID == wid }) else {
            throw NSError(domain: "bench", code: 1, userInfo: [NSLocalizedDescriptionKey: "own window \(wid) not in SCShareableContent"])
        }
        // Window-only capture: sees only our own window, even when occluded.
        let filter = SCContentFilter(desktopIndependentWindow: scw)
        let cfg = SCStreamConfiguration()
        cfg.width = Int(size.width * scale)
        cfg.height = Int(size.height * scale)
        let fps = NSScreen.main?.maximumFramesPerSecond ?? 60
        cfg.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(fps))
        cfg.queueDepth = 4
        cfg.showsCursor = false
        cfg.pixelFormat = kCVPixelFormatType_32BGRA
        let s = SCStream(filter: filter, configuration: cfg, delegate: nil)
        try s.addStreamOutput(self, type: .screen, sampleHandlerQueue: DispatchQueue(label: "cap", qos: .userInteractive))
        try await s.startCapture()
        stream = s
        defer { Task { try? await s.stopCapture() } }

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
        let dumpDir = ProcessInfo.processInfo.environment["LATBENCH_DUMP"]
        var found: Box?
        var after = latest()!
        for attempt in 0..<5 where found == nil {
            let before = latest()!
            _ = inject(kX, "x")
            try await Task.sleep(nanoseconds: 400_000_000)
            after = latest()!
            if let d = dumpDir { dump(before, "\(d)/\(label)-before.png"); dump(after, "\(d)/\(label)-after.png") }
            found = box(before, after)
            if found == nil { print("[\(label)] calibration attempt \(attempt + 1): no change yet") ; try await Task.sleep(nanoseconds: 1_000_000_000) }
        }
        guard var r = found else {
            throw NSError(domain: "bench", code: 3, userInfo: [NSLocalizedDescriptionKey: "typing 'x' changed nothing in the window"])
        }
        _ = inject(kDelete, "\u{7f}")
        try await Task.sleep(nanoseconds: 400_000_000)
        r.x0 = max(0, r.x0 - 2); r.y0 = max(0, r.y0 - 2)
        r.x1 = min(after.w - 1, r.x1 + 2); r.y1 = min(after.h - 1, r.y1 + 2)
        print("[\(label)] window \(Int(size.width))x\(Int(size.height))pt, capture \(cfg.width)x\(cfg.height)px, frame interval \(String(format: "%.2f", 1000.0 / Double(fps))) ms (\(fps) Hz), glyph box \(r.x0),\(r.y0)-\(r.x1),\(r.y1), idle frames changed: \(idleChanged)")

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
        print(String(format: "[%@] n=%d misses=%d  min %.2f  median %.2f  p95 %.2f  p99 %.2f  max %.2f ms",
                     label, v.count, misses, v.first ?? .nan, p(0.5), p(0.95), p(0.99), v.last ?? .nan))
    }
}
