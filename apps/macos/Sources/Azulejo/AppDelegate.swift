import AppKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var window: NSWindow?
    private var keyMonitor: Any?
    private let core = Core()

    func applicationDidFinishLaunching(_ notification: Notification) {
        let treeView = TileTreeView()
        core.onEvent = { json in
            if let event = LayoutChanged.decode(json) { treeView.apply(event) }
        }

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 960, height: 640),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Azulejo"
        window.contentView = treeView
        window.center()
        window.makeKeyAndOrderFront(nil)
        self.window = window

        // Spike (step 0.6): seed a few splits so there's something to look at.
        core.send(#"{"method":"layout.get"}"#)
        split("right")
        split("down")
        split("right")

        // ⌘D splits right, ⌘⇧D splits down. A placeholder for the real key router.
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            guard event.charactersIgnoringModifiers?.lowercased() == "d" else { return event }
            switch flags {
            case [.command]: self?.split("right")
            case [.command, .shift]: self?.split("down")
            default: return event
            }
            return nil
        }
        NSApp.activate()
    }

    private func split(_ direction: String) {
        core.send(#"{"method":"layout.split","params":{"direction":"\#(direction)"}}"#)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}
