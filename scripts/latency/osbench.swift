// osbench: keystroke → glyph latency through the REAL OS input path.
//
//   osbench --pid <pid> --samples 200 --label <name> --csv <out.csv> --yes-post-real-keys
//
// DANGER: this brings the target app to the front and posts real key events
// through the HID event tap. They go to whatever window is focused. Only run it
// (via run-os-input.sh) when nobody is using the Mac: no meetings, no typing.
//
// Capture is window-only (SCContentFilter(desktopIndependentWindow:)), so only
// the target window's pixels are ever read.
import AppKit
import CoreGraphics
import CoreMedia
import ScreenCaptureKit

setvbuf(stdout, nil, _IOLBF, 0)

var pidArg: pid_t = 0, samples = 200, label = "run", csvPath = "/dev/null", confirmed = false
var argIt = CommandLine.arguments.dropFirst().makeIterator()
while let a = argIt.next() {
    switch a {
    case "--pid": pidArg = pid_t(argIt.next()!)!
    case "--samples": samples = Int(argIt.next()!)!
    case "--label": label = argIt.next()!
    case "--csv": csvPath = argIt.next()!
    case "--yes-post-real-keys": confirmed = true
    default: fatalError("unknown arg \(a)")
    }
}
guard confirmed else {
    print("refusing: osbench posts real key events to the focused window; pass --yes-post-real-keys")
    exit(2)
}

var tb = mach_timebase_info_data_t()
mach_timebase_info(&tb)
func ms(_ t: UInt64) -> Double { Double(t) * Double(tb.numer) / Double(tb.denom) / 1e6 }

struct Frame { let px: [UInt8]; let w: Int; let h: Int; let stride: Int; let t: UInt64; let seq: Int }
struct Box { var x0, y0, x1, y1: Int }

final class Grabber: NSObject, SCStreamOutput {
    let cond = NSCondition()
    var frame: Frame?
    var seq = 0
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
        cond.lock(); seq += 1; frame = Frame(px: copy, w: w, h: h, stride: st, t: dt, seq: seq); cond.broadcast(); cond.unlock()
    }
    func latest() -> Frame? { cond.lock(); defer { cond.unlock() }; return frame }
    func next(after: Int, timeout: Double) -> Frame? {
        let deadline = Date(timeIntervalSinceNow: timeout)
        cond.lock(); defer { cond.unlock() }
        while seq <= after { if !cond.wait(until: deadline) { return nil } }
        return frame
    }
}

func box(_ a: Frame, _ b: Frame) -> Box? {
    guard a.w == b.w, a.h == b.h else { return nil }
    var r = Box(x0: .max, y0: .max, x1: -1, y1: -1)
    for y in 0..<a.h { let row = y * a.stride
        for x in 0..<a.w { let i = row + x * 4
            if a.px[i] != b.px[i] || a.px[i+1] != b.px[i+1] || a.px[i+2] != b.px[i+2] {
                r.x0 = min(r.x0, x); r.y0 = min(r.y0, y); r.x1 = max(r.x1, x); r.y1 = max(r.y1, y) } } }
    return r.x1 < 0 ? nil : r
}
func differs(_ a: Frame, _ b: Frame, _ r: Box) -> Bool {
    guard a.w == b.w, a.h == b.h else { return true }
    for y in r.y0...r.y1 { let row = y * a.stride
        for x in r.x0...r.x1 { let i = row + x * 4
            if a.px[i] != b.px[i] || a.px[i+1] != b.px[i+1] || a.px[i+2] != b.px[i+2] { return true } } }
    return false
}

let src = CGEventSource(stateID: .hidSystemState)
let target = NSRunningApplication(processIdentifier: pidArg)
/// Post a key only while the target app is frontmost; returns mach time before posting.
func tap(_ code: CGKeyCode) -> UInt64? {
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pidArg else { return nil }
    let t = mach_absolute_time()
    CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: true)!.post(tap: .cghidEventTap)
    CGEvent(keyboardEventSource: src, virtualKey: code, keyDown: false)!.post(tap: .cghidEventTap)
    return t
}
let kX: CGKeyCode = 7, kDelete: CGKeyCode = 51

func run() async throws {
    guard let target else { fatalError("no app with pid \(pidArg)") }
    target.activate()
    try await Task.sleep(nanoseconds: 800_000_000)
    let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as! [[String: Any]]
    guard let w = list.first(where: { ($0[kCGWindowOwnerPID as String] as? pid_t) == pidArg && ($0[kCGWindowLayer as String] as? Int) == 0 }) else {
        fatalError("no on-screen window for pid \(pidArg)")
    }
    let wid = w[kCGWindowNumber as String] as! CGWindowID
    let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
    guard let scw = content.windows.first(where: { $0.windowID == wid }) else { fatalError("window not capturable") }
    let cfg = SCStreamConfiguration()
    let scale = NSScreen.main?.backingScaleFactor ?? 2
    cfg.width = Int(scw.frame.width * scale); cfg.height = Int(scw.frame.height * scale)
    let fps = NSScreen.main?.maximumFramesPerSecond ?? 60
    cfg.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(fps))
    cfg.queueDepth = 4; cfg.showsCursor = false; cfg.pixelFormat = kCVPixelFormatType_32BGRA
    let g = Grabber()
    let s = SCStream(filter: SCContentFilter(desktopIndependentWindow: scw), configuration: cfg, delegate: nil)
    try s.addStreamOutput(g, type: .screen, sampleHandlerQueue: DispatchQueue(label: "cap", qos: .userInteractive))
    try await s.startCapture()
    guard g.next(after: 0, timeout: 3) != nil else { fatalError("no frames") }
    try await Task.sleep(nanoseconds: 500_000_000)

    let before = g.latest()!
    guard tap(kX) != nil else { fatalError("target lost focus; aborting without posting") }
    try await Task.sleep(nanoseconds: 400_000_000)
    guard var r = box(before, g.latest()!) else { fatalError("typing 'x' changed nothing") }
    _ = tap(kDelete)
    try await Task.sleep(nanoseconds: 400_000_000)
    r.x0 = max(0, r.x0 - 2); r.y0 = max(0, r.y0 - 2); r.x1 += 2; r.y1 += 2
    r.x1 = min(before.w - 1, r.x1); r.y1 = min(before.h - 1, r.y1)
    print("[\(label)] capture \(cfg.width)x\(cfg.height)px @\(fps) Hz, glyph box \(r)")

    var lat: [Double] = [], misses = 0
    var lines = ["label,sample,latency_ms,capture_lag_ms"]
    for i in 0..<samples {
        try await Task.sleep(nanoseconds: UInt64(Double.random(in: 0.12...0.22) * 1e9))
        let base = g.latest()!
        guard let t0 = tap(kX) else { print("[\(label)] target lost focus at sample \(i); stopping"); break }
        var hit: Frame?, seen = base.seq
        let deadline = Date(timeIntervalSinceNow: 1.0)
        while Date() < deadline, hit == nil {
            guard let f = g.next(after: seen, timeout: 0.5) else { break }
            seen = f.seq
            if f.t > t0, differs(base, f, r) { hit = f }
        }
        let tcb = mach_absolute_time()
        if let f = hit {
            lat.append(ms(f.t - t0))
            lines.append("\(label),\(i),\(String(format: "%.3f", ms(f.t - t0))),\(String(format: "%.3f", ms(tcb - f.t)))")
        } else { misses += 1; lines.append("\(label),\(i),,") }
        _ = tap(kDelete)
        let back = Date(timeIntervalSinceNow: 1.0)
        var s2 = g.latest()!.seq
        while Date() < back { if !differs(base, g.latest()!, r) { break }; if let f = g.next(after: s2, timeout: 0.3) { s2 = f.seq } }
    }
    try await s.stopCapture()
    try lines.joined(separator: "\n").appending("\n").write(toFile: csvPath, atomically: true, encoding: .utf8)
    let v = lat.sorted()
    func p(_ q: Double) -> Double { v.isEmpty ? .nan : v[min(v.count - 1, Int((q * Double(v.count - 1)).rounded()))] }
    print(String(format: "[%@] n=%d misses=%d  median %.2f  p95 %.2f  p99 %.2f ms", label, v.count, misses, p(0.5), p(0.95), p(0.99)))
}

let done = DispatchSemaphore(value: 0)
Task { do { try await run() } catch { print("error: \(error)"); exit(1) }; done.signal() }
while done.wait(timeout: .now()) == .timedOut { RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.05)) }
