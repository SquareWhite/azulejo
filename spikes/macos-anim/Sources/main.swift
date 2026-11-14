import AppKit
import CoreImage
import GhosttyKit

// azj-anim (PLAN 0.5): three libghostty surfaces, each running
// `herdr terminal attach`, in a split layout. Layout changes animate over
// 200 ms; during motion each moving terminal is a scaled CALayer snapshot of
// its last frame, and its surface is resized exactly once when motion ends.
//
// No global input and no activation: the app runs as an accessory, never
// calls activate, and is driven only by --stress, signals or (if a human
// focuses it) ⌘1 / ⌘2 / ⌘3.
//
//   --attach '<cmd>'     x3, command for tiles A, B, C
//   --env K=V            env for every surface
//   --stress N           run N animations back to back, then print a summary
//   --duration MS        animation length (default 200)
//   --mode ca|link       ca: CABasicAnimation (render server); link: per-frame from the display link
//   --gravity resize|topLeft   how a snapshot fills its moving frame
//   --log PATH           JSON lines per animation + summary
//   --exit               quit after --stress finishes
//   --start-delay MS     wait before the stress run (lets herdr paint)
//   --front              order the window in front of others (still never activates)
//   --reveal first|quiet when to swap snapshot → live after the resize:
//                        first = first frame at the new size; quiet (default) =
//                        new size and no new frame for --quiet-ms (60); cap 250 ms
//   --resize-at end|start  resize each surface once, when motion ends (default) or
//                        when it starts (redraw overlaps the motion; snapshot hides it)
//   --gap MS             pause between stress animations (default 0 = back to back)
//   --dump DIR           PNGs per moving tile for the first --dump-count animations:
//                        <i>-<tile>-0-before.png (snapshot), -1-reveal.png (live frame
//                        shown when the snapshot is removed), -2-after.png (300 ms later)
//
// Signals: USR1 = divider move, USR2 = close/split C toggle.

struct Args {
    var attach: [String] = []
    var env: [String: String] = [:]
    var stress = 0
    var duration = 0.2
    var mode = "ca"
    var gravity = CALayerContentsGravity.resize
    var log: String?
    var exitAfter = false
    var startDelay = 1.5
    var front = false
    var dump: String?
    var dumpCount = 4
    var reveal = "quiet"
    var resizeAt = "end"
    var gap = 0.0
    var quietMs = 60.0

    init(_ argv: [String]) {
        var it = argv.dropFirst().makeIterator()
        while let a = it.next() {
            switch a {
            case "--attach": if let v = it.next() { attach.append(v) }
            case "--env":
                if let kv = it.next(), let eq = kv.firstIndex(of: "=") {
                    env[String(kv[..<eq])] = String(kv[kv.index(after: eq)...])
                }
            case "--stress": stress = Int(it.next() ?? "") ?? 0
            case "--duration": duration = (Double(it.next() ?? "") ?? 200) / 1000
            case "--mode": mode = it.next() ?? mode
            case "--gravity": gravity = it.next() == "topLeft" ? .topLeft : .resize
            case "--log": log = it.next()
            case "--exit": exitAfter = true
            case "--front": front = true
            case "--dump": dump = it.next()
            case "--dump-count": dumpCount = Int(it.next() ?? "") ?? 4
            case "--reveal": reveal = it.next() ?? reveal
            case "--resize-at": resizeAt = it.next() ?? resizeAt
            case "--gap": gap = (Double(it.next() ?? "") ?? 0) / 1000
            case "--quiet-ms": quietMs = Double(it.next() ?? "") ?? 60
            case "--start-delay": startDelay = (Double(it.next() ?? "") ?? 1500) / 1000
            default: break
            }
        }
    }
}

// MARK: Layout

/// Fixed spike layout: A | (B / C). `ratio` is A's share of the width;
/// C can be closed (B takes the whole right column) and split back.
struct Layout: Equatable {
    var ratio: CGFloat = 0.5
    var cOpen = true

    static let gap: CGFloat = 2

    /// Frames in the (non-flipped) content view. A closed C gets a zero-height
    /// frame on the B/C boundary so open/close animate from/to a line.
    func frames(in b: CGRect) -> [CGRect] {
        let g = Layout.gap
        let leftW = (b.width * ratio).rounded()
        let a = CGRect(x: 0, y: 0, width: leftW - g / 2, height: b.height)
        let rx = leftW + g / 2, rw = b.width - rx
        if cOpen {
            let halfH = (b.height / 2).rounded()
            let bTile = CGRect(x: rx, y: halfH + g / 2, width: rw, height: b.height - halfH - g / 2)
            let cTile = CGRect(x: rx, y: 0, width: rw, height: halfH - g / 2)
            return [a, bTile, cTile]
        } else {
            return [a, CGRect(x: rx, y: 0, width: rw, height: b.height),
                    CGRect(x: rx, y: 0, width: rw, height: 0)]
        }
    }
}

enum Anim: String { case divider, close, split }

// MARK: Overlay

/// Holds snapshot layers above the live surfaces; never takes mouse events.
final class OverlayView: NSView {
    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.masksToBounds = true
    }
    required init?(coder: NSCoder) { fatalError() }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

final class ContentView: NSView {
    weak var controller: AnimController?
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard event.modifierFlags.contains(.command), let c = controller else {
            return super.performKeyEquivalent(with: event)
        }
        switch event.charactersIgnoringModifiers {
        case "1": c.trigger(.divider); return true
        case "2": c.trigger(c.layout.cOpen ? .close : .split); return true
        case "3": c.trigger(c.layout.cOpen ? .close : .split); return true
        default: return super.performKeyEquivalent(with: event)
        }
    }
}

// MARK: Metrics

struct AnimRecord {
    let index: Int
    let kind: Anim
    var setupMs = 0.0
    var snapMs = 0.0                         // part of setup spent copying frames
    var ticks: [CFTimeInterval] = []        // display-link timestamps during motion
    var resizes: [Int] = [0, 0, 0]          // surface resizes caused by this animation
    var expected: [Int] = [0, 0, 0]
    var settleMs: [Double] = [0, 0, 0]      // resize → snapshot removed
    var revealMs: [Double] = [0, 0, 0]      // motion end → snapshot removed
    var snapshotOK: [Bool] = [true, true, true]
}

private let ciContext = CIContext()
func writePNG(_ surface: IOSurfaceRef?, _ path: String) {
    guard let surface else { return }
    let img = CIImage(ioSurface: surface)
    let cs = CGColorSpace(name: CGColorSpace.displayP3)!
    try? ciContext.writePNGRepresentation(of: img, to: URL(fileURLWithPath: path), format: .RGBA8, colorSpace: cs)
}

func median(_ xs: [Double]) -> Double {
    guard !xs.isEmpty else { return 0 }
    let s = xs.sorted(); return s[s.count / 2]
}

func percentile(_ xs: [Double], _ p: Double) -> Double {
    guard !xs.isEmpty else { return 0 }
    let s = xs.sorted(); return s[min(s.count - 1, Int(Double(s.count - 1) * p))]
}

// MARK: Controller

final class AnimController: NSObject {
    let args: Args
    let window: NSWindow
    let content: ContentView
    let overlay: OverlayView
    let surfaces: [SurfaceView]
    var layout = Layout()

    private var busy = false
    private var queue: [Anim] = []
    private var displayLink: CADisplayLink?
    private var current: AnimRecord?
    private var records: [AnimRecord] = []
    private var snapLayers: [Int: CALayer] = [:]
    private var linkAnim: (from: [CGRect], to: [CGRect], start: CFTimeInterval, done: () -> Void)?
    private struct Settle {
        let resizedAt: CFTimeInterval
        let px: CGSize
        let before: IOSurfaceRef?      // contents at resize time
        var last: IOSurfaceRef?        // contents seen on the previous tick
        var stable = 0                 // ticks since contents last changed
        var lastChange: CFTimeInterval
        var capFrom: CFTimeInterval = 0    // motion end; the 250 ms cap and reveal delay count from here
    }
    private var settling: [Int: Settle] = [:]
    private var pendingSettle: [Int: Settle] = [:]
    /// After a snapshot is removed, keep watching the live surface for 300 ms:
    /// any new frame there means we revealed before herdr's redraw finished.
    private var watching: [Int: (until: CFTimeInterval, last: IOSurfaceRef?, seed: UInt32, record: Int)] = [:]
    private var inMotion = false
    private var settleDone: (() -> Void)?
    private var refresh: Double = 60
    private var logFile: FileHandle?
    private var occlusionSamples: [Bool] = []

    init(args: Args, runtime: GhosttyRuntime) {
        self.args = args
        let frame = NSRect(x: 0, y: 0, width: 1200, height: 720)
        window = NSWindow(contentRect: frame, styleMask: [.titled, .closable, .resizable, .miniaturizable],
                          backing: .buffered, defer: false)
        window.title = "azj-anim"
        content = ContentView(frame: frame)
        content.wantsLayer = true
        content.layer?.backgroundColor = NSColor(white: 0.12, alpha: 1).cgColor
        window.contentView = content

        let names = ["A", "B", "C"]
        var made: [SurfaceView] = []
        let rects = layout.frames(in: content.bounds)
        for i in 0..<3 {
            let cmd = i < args.attach.count ? args.attach[i] : "/bin/zsh -l"
            let v = SurfaceView(runtime: runtime, name: names[i], command: cmd, env: args.env, frame: rects[i])
            content.addSubview(v)
            made.append(v)
        }
        surfaces = made
        overlay = OverlayView(frame: content.bounds)
        overlay.autoresizingMask = [.width, .height]
        content.addSubview(overlay)
        super.init()
        content.controller = self
        surfaces.forEach { s in s.onResize = { [weak self] v, px in self?.surfaceResized(v, px) } }

        if let path = args.log {
            FileManager.default.createFile(atPath: path, contents: nil)
            logFile = FileHandle(forWritingAtPath: path)
        }
        refresh = Double(NSScreen.main?.maximumFramesPerSecond ?? 60)
    }

    func show() {
        // Never activate and never order above other apps' windows: put the
        // window at the back of the stack. It still renders and can be
        // captured with `screencapture -l`.
        window.setFrameOrigin(NSPoint(x: 40, y: 40))
        if args.front { window.orderFrontRegardless() } else { window.orderBack(nil) }
        emit(["event": "window", "id": window.windowNumber, "refresh_hz": refresh,
              "screen": window.screen?.localizedName ?? "?",
              "scale": window.backingScaleFactor])
        let link = content.displayLink(target: self, selector: #selector(tick(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: Float(refresh), maximum: Float(refresh),
                                                         preferred: Float(refresh))
        link.add(to: .main, forMode: .common)
        displayLink = link

        installSignals()
        if args.stress > 0 {
            DispatchQueue.main.asyncAfter(deadline: .now() + args.startDelay) { self.runStress() }
        }
    }

    // MARK: Triggers

    private var signalSources: [DispatchSourceSignal] = []
    private func installSignals() {
        for (sig, action) in [(SIGUSR1, 0), (SIGUSR2, 1)] {
            signal(sig, SIG_IGN)
            let src = DispatchSource.makeSignalSource(signal: sig, queue: .main)
            src.setEventHandler { [weak self] in
                guard let self else { return }
                self.trigger(action == 0 ? .divider : (self.layout.cOpen ? .close : .split))
            }
            src.resume()
            signalSources.append(src)
        }
    }

    private func runStress() {
        let cycle: [Anim] = [.divider, .close, .split, .divider]
        for i in 0..<args.stress { queue.append(cycle[i % cycle.count]) }
        // close/split must alternate with the real state; the cycle keeps that invariant.
        pump()
    }

    func trigger(_ a: Anim) {
        queue.append(a)
        pump()
    }

    private func pump() {
        guard !busy, !queue.isEmpty else {
            if !busy, queue.isEmpty, args.stress > 0, records.count >= args.stress { finish() }
            return
        }
        var next = queue.removeFirst()
        if next == .close && !layout.cOpen { next = .split }
        if next == .split && layout.cOpen { next = .close }
        start(next)
    }

    // MARK: Animation

    private func start(_ kind: Anim) {
        busy = true
        let t0 = CACurrentMediaTime()
        let bounds = content.bounds
        let from = layout.frames(in: bounds)
        switch kind {
        case .divider: layout.ratio = layout.ratio < 0.56 ? 0.62 : 0.5
        case .close: layout.cOpen = false
        case .split: layout.cOpen = true
        }
        let to = layout.frames(in: bounds)
        var rec = AnimRecord(index: records.count, kind: kind)

        // Which tiles move or change size; only those get snapshots.
        var moving: [Int] = []
        for i in 0..<3 where from[i] != to[i] {
            moving.append(i)
            // A tile only resizes if it ends visible with a new size.
            if to[i].height > 0, to[i].size != surfaces[i].frame.size { rec.expected[i] = 1 }
        }
        let resizeBefore = surfaces.map(\.resizeCount)
        for i in moving { watching[i] = nil }

        var snapTime = 0.0
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for i in moving {
            let snap = snapLayers[i] ?? CALayer()
            if snapLayers[i] == nil || from[i].height > 0 {
                // A closed tile keeps the snapshot it had when it closed.
                let ts = CACurrentMediaTime()
                let img = surfaces[i].snapshot()
                snapTime += CACurrentMediaTime() - ts
                if let img {
                    snap.contents = img
                    dumpFrame(img, rec.index, i, "0-before")
                } else {
                    rec.snapshotOK[i] = false
                }
            }
            snap.contentsGravity = args.gravity
            snap.contentsScale = window.backingScaleFactor
            snap.masksToBounds = true
            snap.backgroundColor = NSColor(white: 0.12, alpha: 1).cgColor
            snap.frame = from[i]
            snap.opacity = from[i].height > 0 ? 1 : 0
            if snap.superlayer == nil { overlay.layer?.addSublayer(snap) }
            snapLayers[i] = snap
            surfaces[i].isHidden = true  // freeze: no live frames, no resize during motion
        }
        CATransaction.commit()
        if args.resizeAt == "start" {
            let now = CACurrentMediaTime()
            for i in moving where to[i].height > 0 {
                let sv = surfaces[i]
                let before = sv.resizeCount
                let p = sv.presentedSurface
                sv.frame = to[i]   // the one resize, while hidden under the snapshot
                if sv.resizeCount > before {
                    pendingSettle[i] = Settle(resizedAt: now, px: sv.lastPixelSize, before: p, last: p, lastChange: now)
                    traces[records.count * 3 + i] = (i, now, p, p.map(IOSurfaceGetSeed) ?? 0, records.count, kind.rawValue, [])
                }
            }
        }
        rec.setupMs = (CACurrentMediaTime() - t0) * 1000
        rec.snapMs = snapTime * 1000
        current = rec
        inMotion = true

        let finishMotion = { [weak self] in
            guard let self else { return }
            self.endMotion(moving: moving, to: to, resizeBefore: resizeBefore)
        }

        if args.mode == "link" {
            linkAnim = (from, to, CACurrentMediaTime(), finishMotion)
        } else {
            CATransaction.begin()
            CATransaction.setCompletionBlock(finishMotion)
            for i in moving {
                guard let snap = snapLayers[i] else { continue }
                let timing = CAMediaTimingFunction(controlPoints: 0.2, 0.9, 0.3, 1.0) // ease-out
                let pos = CABasicAnimation(keyPath: "position")
                pos.fromValue = NSValue(point: CGPoint(x: from[i].midX, y: from[i].midY))
                pos.toValue = NSValue(point: CGPoint(x: to[i].midX, y: to[i].midY))
                let bnd = CABasicAnimation(keyPath: "bounds")
                bnd.fromValue = NSValue(rect: CGRect(origin: .zero, size: from[i].size))
                bnd.toValue = NSValue(rect: CGRect(origin: .zero, size: to[i].size))
                let op = CABasicAnimation(keyPath: "opacity")
                op.fromValue = from[i].height > 0 ? 1 : 0
                op.toValue = to[i].height > 0 ? 1 : 0
                let group = CAAnimationGroup()
                group.animations = [pos, bnd, op]
                group.duration = args.duration
                group.timingFunction = timing
                group.preferredFrameRateRange = CAFrameRateRange(
                    minimum: Float(refresh), maximum: Float(refresh), preferred: Float(refresh))
                snap.add(group, forKey: "move")
                // Model values = final state.
                CATransaction.setDisableActions(true)
                snap.frame = to[i]
                snap.opacity = to[i].height > 0 ? 1 : 0
            }
            CATransaction.commit()
        }
    }

    @objc private func tick(_ link: CADisplayLink) {
        occlusionSamples.append(window.occlusionState.contains(.visible))
        if inMotion {
            current?.ticks.append(link.timestamp)
        }
        if let la = linkAnim {
            var p = (link.targetTimestamp - la.start) / args.duration
            p = max(0, min(1, p))
            let e = 1 - pow(1 - p, 3) // ease-out cubic
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            for (i, snap) in snapLayers {
                let f = la.from[i], t = la.to[i]
                snap.frame = CGRect(x: f.minX + (t.minX - f.minX) * e, y: f.minY + (t.minY - f.minY) * e,
                                    width: f.width + (t.width - f.width) * e,
                                    height: f.height + (t.height - f.height) * e)
                let o0: Double = f.height > 0 ? 1 : 0, o1: Double = t.height > 0 ? 1 : 0
                snap.opacity = Float(o0 + (o1 - o0) * e)
            }
            CATransaction.commit()
            if p >= 1 { linkAnim = nil; la.done() }
        }
        for (i, var st) in pendingSettle {
            let cur = surfaces[i].presentedSurface
            let same = cur.map { c in st.last.map { CFEqual(c, $0) && IOSurfaceGetSeed(c) == IOSurfaceGetSeed($0) } ?? false } ?? (st.last == nil)
            if same { st.stable += 1 } else { st.stable = 0; st.last = cur; st.lastChange = link.timestamp }
            pendingSettle[i] = st
        }
        pollSettle(link.timestamp)
        pollWatch(link.timestamp)
        pollTrace(link.timestamp)
    }

    private func pollWatch(_ now: CFTimeInterval) {
        for (i, w) in watching {
            let cur = surfaces[i].presentedSurface
            let seed = cur.map(IOSurfaceGetSeed) ?? 0
            let changed: Bool = {
                switch (cur, w.last) {
                case let (x?, y?): return !CFEqual(x, y) || seed != w.seed
                case (nil, nil): return false
                default: return true
                }
            }()
            if changed {
                lateByRecord[w.record, default: [0, 0, 0]][i] += 1
                watching[i] = (w.until, cur, seed, w.record)
            }
            if now > w.until { watching[i] = nil }
        }
    }
    private var lateByRecord: [Int: [Int]] = [:]
    /// Present timeline per tile: ms after the resize at which a new frame appeared.
    private var traces: [Int: (tile: Int, start: CFTimeInterval, last: IOSurfaceRef?, seed: UInt32, record: Int, kind: String, ms: [Double])] = [:]

    private func pollTrace(_ now: CFTimeInterval) {
        for (key, t) in traces {
            let i = t.tile
            let cur = surfaces[i].presentedSurface
            let seed = cur.map(IOSurfaceGetSeed) ?? 0
            var t = t
            let changed: Bool = {
                switch (cur, t.last) {
                case let (x?, y?): return !CFEqual(x, y) || seed != t.seed
                case (nil, nil): return false
                default: return true
                }
            }()
            if changed { t.ms.append(((now - t.start) * 1000).rounded()); t.last = cur; t.seed = seed }
            if now - t.start > 0.5 {
                emit(["event": "present_trace", "i": t.record, "kind": t.kind, "tile": ["A", "B", "C"][i], "ms": t.ms])
                traces[key] = nil
            } else { traces[key] = t }
        }
    }

    private func endMotion(moving: [Int], to: [CGRect], resizeBefore: [Int]) {
        // One resize per tile that ends visible with a new size.
        inMotion = false
        let now = CACurrentMediaTime()
        for i in moving {
            let s = surfaces[i]
            if args.resizeAt == "start", to[i].height > 0 {
                s.isHidden = false
                if var st = pendingSettle.removeValue(forKey: i) {
                    // Contents may have changed (and settled) during motion; keep
                    // the quiet clock but judge "fresh" against the pre-resize frame.
                    st.capFrom = now
                    settling[i] = st
                } else {
                    removeSnapshot(i)
                }
                continue
            }
            if to[i].height > 0 {
                let before = s.resizeCount
                s.frame = to[i]           // → setFrameSize → ghostty_surface_set_size (once)
                s.isHidden = false        // still covered by its snapshot
                if s.resizeCount > before {
                    let p = s.presentedSurface
                    traces[records.count * 3 + i] = (i, now, p, p.map(IOSurfaceGetSeed) ?? 0, records.count, current?.kind.rawValue ?? "?", [])
                    settling[i] = Settle(resizedAt: now, px: s.lastPixelSize,
                                         before: s.presentedSurface, last: s.presentedSurface, lastChange: now,
                                         capFrom: now)
                } else {
                    removeSnapshot(i)     // moved but same size: live frame is already right
                }
            } else {
                s.frame = CGRect(origin: to[i].origin, size: s.frame.size) // stays hidden, no resize
            }
        }
        settleDone = { [weak self] in
            guard let self, var rec = self.current else { return }
            for i in 0..<3 { rec.resizes[i] = self.surfaces[i].resizeCount - resizeBefore[i] }
            self.records.append(rec)
            self.emitRecord(rec)
            self.current = nil
            self.settleDone = nil
            if self.args.gap > 0 {
                DispatchQueue.main.asyncAfter(deadline: .now() + self.args.gap) { self.busy = false; self.pump() }
            } else {
                self.busy = false
                self.pump()
            }
        }
        if settling.isEmpty { settleDone?() }
    }

    /// Swap snapshot → live once libghostty has presented a new frame at the
    /// new size and the contents have then stayed put for 3 frames (herdr's
    /// redraw after the resize has landed), or after 250 ms whatever happens.
    private func pollSettle(_ now: CFTimeInterval) {
        guard !settling.isEmpty else { return }
        func same(_ a: IOSurfaceRef?, _ b: IOSurfaceRef?) -> Bool {
            switch (a, b) {
            case (nil, nil): return true
            case let (x?, y?): return CFEqual(x, y) && IOSurfaceGetSeed(x) == IOSurfaceGetSeed(y)
            default: return false
            }
        }
        for (i, var st) in settling {
            let cur = surfaces[i].presentedSurface
            let sized = cur.map { IOSurfaceGetWidth($0) == Int(st.px.width)
                && IOSurfaceGetHeight($0) == Int(st.px.height) } ?? false
            if same(cur, st.last) { st.stable += 1 } else { st.stable = 0; st.last = cur; st.lastChange = now }
            let fresh = !same(cur, st.before)
            let elapsed = (now - st.resizedAt) * 1000
            let sinceEnd = (now - st.capFrom) * 1000
            let quiet = (now - st.lastChange) * 1000 >= args.quietMs
            let ready = args.reveal == "first" ? (sized && fresh) : (sized && fresh && quiet)
            if ready || sinceEnd > 250 {
                current?.settleMs[i] = elapsed
                current?.revealMs[i] = sinceEnd
                if !sized { current?.snapshotOK[i] = false }
                settling[i] = nil
                removeSnapshot(i)
                watching[i] = (now + 0.3, cur, cur.map(IOSurfaceGetSeed) ?? 0, records.count)
                dumpFrame(surfaces[i].snapshot(), records.count, i, "1-reveal")
                let idx = records.count, kind = current?.kind.rawValue
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { [weak self] in
                    guard let self else { return }
                    self.dumpFrame(self.surfaces[i].snapshot(), idx, i, "2-after", kind: kind)
                }
            } else {
                settling[i] = st
            }
        }
        if settling.isEmpty { settleDone?() }
    }

    private func dumpFrame(_ surface: IOSurfaceRef?, _ index: Int, _ tile: Int, _ stage: String, kind: String? = nil) {
        guard let dir = args.dump, index < args.dumpCount else { return }
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        let kind = kind ?? (current?.kind.rawValue) ?? "anim"
        writePNG(surface, "\(dir)/\(index)-\(kind)-\(["A", "B", "C"][tile])-\(stage).png")
    }

    private func removeSnapshot(_ i: Int) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        snapLayers[i]?.removeFromSuperlayer()
        snapLayers[i] = nil
        CATransaction.commit()
    }

    private func surfaceResized(_ v: SurfaceView, _ px: CGSize) {
        emit(["event": "resize", "tile": v.name, "wall": Date().timeIntervalSince1970, "px": "\(Int(px.width))x\(Int(px.height))",
              "during_motion": inMotion])
    }

    // MARK: Reporting

    private func intervals(_ r: AnimRecord) -> [Double] {
        zip(r.ticks.dropFirst(), r.ticks).map { ($0 - $1) * 1000 }
    }

    private func emitRecord(_ r: AnimRecord) {
        let iv = intervals(r)
        let budget = 1000 / refresh
        emit(["event": "anim", "i": r.index, "kind": r.kind.rawValue, "setup_ms": r.setupMs,
              "snap_ms": r.snapMs,
              "frames": r.ticks.count, "median_ms": median(iv), "max_ms": iv.max() ?? 0,
              "dropped": iv.filter { $0 > budget * 1.5 }.count,
              "resizes": r.resizes, "expected": r.expected,
              "settle_ms": r.settleMs.map { ($0 * 10).rounded() / 10 },
              "reveal_after_end_ms": r.revealMs.map { ($0 * 10).rounded() / 10 },
              "snapshot_ok": r.snapshotOK])
    }

    private func finish() {
        if !watching.isEmpty || !traces.isEmpty {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) { self.finish() }
            return
        }
        let budget = 1000 / refresh
        let all = records.flatMap(intervals)
        let wrong = records.filter { $0.resizes != $0.expected }
        let settles = records.flatMap { r in (0..<3).filter { r.expected[$0] == 1 }.map { r.settleMs[$0] } }
        let visible = occlusionSamples.filter { $0 }.count
        let summary: [String: Any] = [
            "event": "summary", "mode": args.mode, "animations": records.count,
            "duration_ms": args.duration * 1000, "refresh_hz": refresh,
            "frames": all.count + records.count,
            "interval_median_ms": median(all), "interval_p99_ms": percentile(all, 0.99),
            "interval_max_ms": all.max() ?? 0,
            "dropped_frames": all.filter { $0 > budget * 1.5 }.count,
            "animations_with_drops": records.filter { r in intervals(r).contains { $0 > budget * 1.5 } }.count,
            "setup_median_ms": median(records.map(\.setupMs)), "setup_max_ms": records.map(\.setupMs).max() ?? 0,
            "resize_mismatches": wrong.count,
            "total_resizes": records.reduce(0) { $0 + $1.resizes.reduce(0, +) },
            "expected_resizes": records.reduce(0) { $0 + $1.expected.reduce(0, +) },
            "settle_median_ms": median(settles), "settle_max_ms": settles.max() ?? 0,
            "settle_timeouts": records.reduce(0) { $0 + $1.revealMs.filter { $0 > 250 }.count },
            "reveal_after_end_median_ms": median(records.flatMap { r in (0..<3).filter { r.expected[$0] == 1 }.map { r.revealMs[$0] } }),
            "reveal_after_end_max_ms": records.flatMap { r in (0..<3).filter { r.expected[$0] == 1 }.map { r.revealMs[$0] } }.max() ?? 0,
            "resize_at": args.resizeAt, "reveal": args.reveal,
            "snapshot_failures": records.reduce(0) { $0 + $1.snapshotOK.filter { !$0 }.count },
            "snap_median_ms": median(records.map(\.snapMs)),
            "late_frames_after_reveal": lateByRecord.values.reduce(0) { $0 + $1.reduce(0, +) },
            "late_frames_by_tile_ABC": (0..<3).map { t in lateByRecord.values.reduce(0) { $0 + $1[t] } },
            "reveals_with_late_frames": lateByRecord.values.reduce(0) { $0 + $1.filter { $0 > 0 }.count },
            "window_visible_fraction": occlusionSamples.isEmpty ? 0 : Double(visible) / Double(occlusionSamples.count),
        ]
        emit(summary)
        if args.exitAfter {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { NSApp.terminate(nil) }
        }
    }

    func emit(_ obj: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys]),
              var line = String(data: data, encoding: .utf8) else { return }
        line += "\n"
        FileHandle.standardOutput.write(line.data(using: .utf8)!)
        logFile?.write(line.data(using: .utf8)!)
    }
}

// MARK: App

final class AppDelegate: NSObject, NSApplicationDelegate {
    let args = Args(CommandLine.arguments)
    var runtime: GhosttyRuntime!
    var controller: AnimController!

    func applicationDidFinishLaunching(_ note: Notification) {
        runtime = GhosttyRuntime()
        controller = AnimController(args: args, runtime: runtime)
        controller.show()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ app: NSApplication) -> Bool { true }
}

let app = NSApplication.shared
// Accessory: no Dock icon, never becomes the active app on its own.
app.setActivationPolicy(.accessory)
let delegate = AppDelegate()
app.delegate = delegate
app.run()
