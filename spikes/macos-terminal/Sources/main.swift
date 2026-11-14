import AppKit
import GhosttyKit

// azj-spike: one window, one libghostty surface running `herdr terminal attach`.
//
// Usage: AzjSpike.app/Contents/MacOS/AzjSpike --command '<shell command>' [--env K=V ...]
// The launcher (run.sh) builds the herdr attach command for us.

struct Args {
    var command = "/bin/zsh -l"
    var env: [String: String] = [:]

    init(_ argv: [String]) {
        var it = argv.dropFirst().makeIterator()
        while let a = it.next() {
            switch a {
            case "--command": command = it.next() ?? command
            case "--env":
                if let kv = it.next(), let eq = kv.firstIndex(of: "=") {
                    env[String(kv[..<eq])] = String(kv[kv.index(after: eq)...])
                }
            default: break
            }
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    let args = Args(CommandLine.arguments)
    var runtime: GhosttyRuntime!
    var window: NSWindow!

    func applicationDidFinishLaunching(_ note: Notification) {
        runtime = GhosttyRuntime()

        let frame = NSRect(x: 0, y: 0, width: 960, height: 600)
        window = NSWindow(
            contentRect: frame,
            styleMask: [.titled, .closable, .resizable, .miniaturizable],
            backing: .buffered, defer: false)
        window.title = "azj-spike"
        window.delegate = self
        window.center()

        let view = SurfaceView(runtime: runtime, command: args.command, env: args.env, frame: frame)
        window.contentView = view
        window.makeKeyAndOrderFront(nil)
        window.makeFirstResponder(view)
        NSApp.activate(ignoringOtherApps: true)
        NSLog("azj-spike: surface up, command=\(args.command)")
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
app.setActivationPolicy(.regular)

let menu = NSMenu()
let appItem = NSMenuItem()
menu.addItem(appItem)
let appMenu = NSMenu()
appMenu.addItem(withTitle: "Quit azj-spike", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
appItem.submenu = appMenu
app.mainMenu = menu

let delegate = AppDelegate()
app.delegate = delegate
app.run()
