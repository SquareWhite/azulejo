import AppKit
import Darwin

/// Cold-start timing (PLAN 1.13). Marks are wall-clock seconds; the process
/// start comes from the kernel, so dyld and Swift runtime start-up count too.
/// With `AZULEJO_TIMING_OUT=<file>` set, one JSON line per launch is appended
/// there once the first window has drawn; otherwise nothing is recorded
/// beyond a log line.
@MainActor
enum Startup {
    private static var marks: [(String, Double)] = []
    private static var reported = false
    private static var link: CADisplayLink?
    private static var tiles = 0

    static func now() -> Double { Date().timeIntervalSince1970 }

    /// When the kernel started this process.
    static func processStart() -> Double? {
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.stride
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()]
        guard sysctl(&mib, 4, &info, &size, nil, 0) == 0 else { return nil }
        let tv = info.kp_proc.p_un.__p_starttime
        return Double(tv.tv_sec) + Double(tv.tv_usec) / 1e6
    }

    static func mark(_ name: String) { marks.append((name, now())) }

    /// Called at the end of launch: draws the focused window now and records
    /// the first display-link tick after that commit as "first frame".
    static func launched(window: NSWindow?, tileCount: Int, log: @escaping (String) -> Void) {
        mark("layout")
        tiles = tileCount
        guard let window, let view = window.contentView else { return report(log: log) }
        window.displayIfNeeded()
        CATransaction.flush()
        mark("committed")
        let target = Tick { report(log: log) }
        let link = view.displayLink(target: target, selector: #selector(Tick.fire(_:)))
        link.add(to: .main, forMode: .common)
        self.link = link
        // A window that never presents (fully hidden) still reports.
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { report(log: log) }
    }

    private static func report(log: (String) -> Void) {
        guard !reported else { return }
        reported = true
        mark("first_frame")
        link?.invalidate()
        link = nil
        let start = processStart() ?? marks.first?.1 ?? now()
        var fields: [String: Any] = ["process_start": start, "tiles": tiles, "pid": Int(getpid())]
        var parts: [String] = []
        for (name, t) in marks {
            let ms = (t - start) * 1000
            fields[name + "_ms"] = (ms * 10).rounded() / 10
            parts.append("\(name) \(Int(ms.rounded())) ms")
        }
        log("startup: " + parts.joined(separator: ", ") + " (\(tiles) tiles)")
        guard let path = ProcessInfo.processInfo.environment["AZULEJO_TIMING_OUT"], !path.isEmpty,
              let data = try? JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys])
        else { return }
        let line = data + Data("\n".utf8)
        if let handle = FileHandle(forWritingAtPath: path) {
            handle.seekToEndOfFile()
            handle.write(line)
            try? handle.close()
        } else {
            FileManager.default.createFile(atPath: path, contents: line)
        }
    }

    @MainActor
    private final class Tick: NSObject {
        let action: () -> Void
        init(_ action: @escaping () -> Void) { self.action = action }
        @objc func fire(_ link: CADisplayLink) { action() }
    }
}
