import AppKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var window: NSWindow?
    private let core = Core()

    func applicationDidFinishLaunching(_ notification: Notification) {
        let label = NSTextField(labelWithString: "Azulejo")
        label.font = .systemFont(ofSize: 24, weight: .medium)
        label.alignment = .center

        core.onEvent = { json in label.stringValue = "Azulejo\n\(json)" }
        core.send(#"{"method":"core.ping"}"#)

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 960, height: 640),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Azulejo"
        window.contentView = label
        window.center()
        window.makeKeyAndOrderFront(nil)
        self.window = window
        NSApp.activate()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}
