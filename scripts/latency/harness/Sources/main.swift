import AppKit
import GhosttyKit

// Latency harness: a copy of spikes/macos-terminal with an in-process bench.
//
// Usage: LatHarness --command '<cmd>' [--env K=V ...] --bench <samples> --label <name> --csv <path>
//
// It never activates, never takes key focus from other apps and never posts
// OS input events: the window is ordered to the back, keys go straight into
// the libghostty surface (ghostty_surface_key), and frames come from a
// window-only ScreenCaptureKit stream that works while the window is occluded.

struct Args {
    var command = "/bin/zsh -l"
    var env: [String: String] = [:]
    var benchSamples = 0
    var label = "run"
    var csv = "/dev/null"

    init(_ argv: [String]) {
        var it = argv.dropFirst().makeIterator()
        while let a = it.next() {
            switch a {
            case "--command": command = it.next() ?? command
            case "--env":
                if let kv = it.next(), let eq = kv.firstIndex(of: "=") {
                    env[String(kv[..<eq])] = String(kv[kv.index(after: eq)...])
                }
            case "--bench": benchSamples = Int(it.next() ?? "0") ?? 0
            case "--label": label = it.next() ?? label
            case "--csv": csv = it.next() ?? csv
            default: break
            }
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    let args = Args(CommandLine.arguments)
    var runtime: GhosttyRuntime!
    var window: NSWindow!
    var bench: Bench?

    func applicationDidFinishLaunching(_ note: Notification) {
        runtime = GhosttyRuntime()

        let frame = NSRect(x: 0, y: 0, width: 960, height: 600)
        window = NSWindow(
            contentRect: frame,
            styleMask: [.titled, .closable, .resizable, .miniaturizable],
            backing: .buffered, defer: false)
        window.title = "azj-latency"
        window.delegate = self
        window.setFrameOrigin(NSPoint(x: 40, y: 40))

        let view = SurfaceView(runtime: runtime, command: args.command, env: args.env, frame: frame)
        window.contentView = view
        if CommandLine.arguments.contains("--foreground") {
            // Only for run-os-input.sh (real OS key events need a focused window).
            window.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
        } else {
            // Behind every other window, no activation, no key focus stolen.
            window.orderBack(nil)
        }
        window.makeFirstResponder(view)
        if let surface = view.surface { ghostty_surface_set_focus(surface, true) }
        if let app = runtime.app { ghostty_app_set_focus(app, true) }
        NSLog("azj-latency: surface up, command=\(args.command)")
        if args.benchSamples > 0 {
            bench = Bench(window: window, view: view, samples: args.benchSamples, label: args.label, csv: args.csv)
            bench?.start(after: 3.0)
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ app: NSApplication) -> Bool { true }

    func applicationDidBecomeActive(_ note: Notification) {
        if let app = runtime?.app { ghostty_app_set_focus(app, true) }
    }

    func applicationDidResignActive(_ note: Notification) {
        if let app = runtime?.app { ghostty_app_set_focus(app, false) }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(CommandLine.arguments.contains("--foreground") ? .regular : .accessory)

let menu = NSMenu()
let appItem = NSMenuItem()
menu.addItem(appItem)
let appMenu = NSMenu()
appMenu.addItem(withTitle: "Quit azj-latency", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
appItem.submenu = appMenu
app.mainMenu = menu

let delegate = AppDelegate()
app.delegate = delegate
app.run()
