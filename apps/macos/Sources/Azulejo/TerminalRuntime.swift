import AppKit
import GhosttyKit

/// The process-wide libghostty app: the user's Ghostty config, the runtime
/// callbacks, and a registry from surface tokens to views.
///
/// libghostty calls back on its own threads (renderer, IO). Every callback
/// below is a plain global function that copies what it needs and hops to the
/// main queue; views are found by token, never by a raw pointer, so a callback
/// that arrives after its view is gone finds nothing instead of a dangling
/// object.
@MainActor
final class TerminalRuntime {
    static var shared: TerminalRuntime?

    /// Where terminal copy and paste go: the system clipboard, or a private
    /// pasteboard in automation runs, so tests never touch the user's.
    nonisolated(unsafe) static let pasteboard: NSPasteboard =
        ProcessInfo.processInfo.environment["AZULEJO_AUTOMATION"].map { $0.isEmpty } == false
            ? NSPasteboard(name: NSPasteboard.Name("dev.azulejo.automation")) : .general

    private(set) var app: ghostty_app_t?
    private(set) var config: ghostty_config_t?
    private var surfaces: [UInt: WeakSurface] = [:]
    private var nextToken: UInt = 1
    private var observers: [NSObjectProtocol] = []

    private struct WeakSurface { weak var view: TerminalSurfaceView? }

    /// Starts libghostty. Nil (logged) if it can't, so the app still runs with
    /// placeholder terminals.
    static func start(log: (String) -> Void) -> TerminalRuntime? {
        if let shared { return shared }
        guard ghostty_init(UInt(CommandLine.argc), CommandLine.unsafeArgv) == GHOSTTY_SUCCESS else {
            log("terminal: ghostty_init failed")
            return nil
        }
        let runtime = TerminalRuntime()
        guard runtime.boot(log: log) else { return nil }
        shared = runtime
        return runtime
    }

    private func boot(log: (String) -> Void) -> Bool {
        // The user's Ghostty config, loaded the way Ghostty.app does.
        guard let cfg = ghostty_config_new() else {
            log("terminal: ghostty_config_new failed")
            return false
        }
        ghostty_config_load_default_files(cfg)
        ghostty_config_load_recursive_files(cfg)
        ghostty_config_finalize(cfg)
        for i in 0..<ghostty_config_diagnostics_count(cfg) {
            if let message = ghostty_config_get_diagnostic(cfg, i).message {
                log("terminal: Ghostty config: \(String(cString: message))")
            }
        }
        config = cfg

        var rt = ghostty_runtime_config_s()
        rt.userdata = nil
        rt.supports_selection_clipboard = false
        rt.wakeup_cb = ghosttyWakeup
        rt.action_cb = ghosttyAction
        rt.read_clipboard_cb = ghosttyReadClipboard
        rt.confirm_read_clipboard_cb = ghosttyConfirmReadClipboard
        rt.write_clipboard_cb = ghosttyWriteClipboard
        rt.close_surface_cb = ghosttyCloseSurface
        guard let made = ghostty_app_new(&rt, cfg) else {
            log("terminal: ghostty_app_new failed")
            return false
        }
        app = made

        let center = NotificationCenter.default
        observers.append(center.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { _ in MainActor.assumeIsolated { TerminalRuntime.shared?.setAppFocus(true) } })
        observers.append(center.addObserver(
            forName: NSApplication.didResignActiveNotification, object: nil, queue: .main
        ) { _ in MainActor.assumeIsolated { TerminalRuntime.shared?.setAppFocus(false) } })
        setAppFocus(NSApp.isActive)
        return true
    }

    private func setAppFocus(_ focused: Bool) {
        if let app { ghostty_app_set_focus(app, focused) }
    }

    func tick() {
        if let app { Perf.measure("ghostty.tick") { ghostty_app_tick(app) } }
    }

    // MARK: Surface registry

    func register(_ view: TerminalSurfaceView) -> UInt {
        let token = nextToken
        nextToken += 1
        surfaces[token] = WeakSurface(view: view)
        return token
    }

    func unregister(_ token: UInt) {
        surfaces[token] = nil
    }

    func surface(_ token: UInt) -> TerminalSurfaceView? {
        surfaces[token]?.view
    }
}

// MARK: - Callbacks (any thread)

/// A surface's userdata is its registry token, not a pointer.
private func token(of surface: ghostty_surface_t?) -> UInt? {
    guard let surface, let userdata = ghostty_surface_userdata(surface) else { return nil }
    return UInt(bitPattern: userdata)
}

private func onMain(_ token: UInt?, _ body: @escaping @MainActor (TerminalSurfaceView) -> Void) {
    guard let token else { return }
    DispatchQueue.main.async {
        MainActor.assumeIsolated {
            if let view = TerminalRuntime.shared?.surface(token) { Perf.measure("ghostty.action") { body(view) } }
        }
    }
}

private func ghosttyWakeup(_: UnsafeMutableRawPointer?) {
    DispatchQueue.main.async { MainActor.assumeIsolated { TerminalRuntime.shared?.tick() } }
}

private func ghosttyAction(
    _: ghostty_app_t?, _ target: ghostty_target_s, _ action: ghostty_action_s
) -> Bool {
    let surface = target.tag == GHOSTTY_TARGET_SURFACE ? target.target.surface : nil
    switch action.tag {
    case GHOSTTY_ACTION_SET_TITLE:
        guard let cTitle = action.action.set_title.title else { return false }
        let title = String(cString: cTitle)
        onMain(token(of: surface)) { $0.titleChanged(title) }
        return true
    case GHOSTTY_ACTION_SHOW_CHILD_EXITED:
        // The attach (or observe) process ended. Handling it here stops
        // libghostty printing "Process exited" into the tile; the tile reads
        // the real reason from the status files the command wrote.
        onMain(token(of: surface)) { $0.childExited() }
        return true
    case GHOSTTY_ACTION_RING_BELL:
        return true
    case GHOSTTY_ACTION_QUIT, GHOSTTY_ACTION_CLOSE_WINDOW, GHOSTTY_ACTION_NEW_WINDOW,
         GHOSTTY_ACTION_NEW_TAB, GHOSTTY_ACTION_NEW_SPLIT, GHOSTTY_ACTION_CLOSE_TAB:
        // Ghostty's own window keybindings: Azulejo's keymap owns these.
        return true
    case GHOSTTY_ACTION_MOUSE_SHAPE:
        let shape = action.action.mouse_shape
        onMain(token(of: surface)) { $0.setMouseShape(shape) }
        return true
    default:
        return false
    }
}

private func ghosttyCloseSurface(_ userdata: UnsafeMutableRawPointer?, _: Bool) {
    // Backup for childExited: libghostty asks to close the surface when its
    // process is gone and it isn't waiting after the command.
    let token = UInt(bitPattern: userdata)
    onMain(token) { $0.childExited() }
}

private func ghosttyReadClipboard(
    _ userdata: UnsafeMutableRawPointer?, _ location: ghostty_clipboard_e,
    _ state: UnsafeMutableRawPointer?, _: UnsafePointer<UnsafePointer<CChar>?>?, _: Int, _: Bool
) -> ghostty_clipboard_read_result_e {
    guard location == GHOSTTY_CLIPBOARD_STANDARD else { return GHOSTTY_CLIPBOARD_READ_UNAVAILABLE }
    let token = UInt(bitPattern: userdata)
    let address = UInt(bitPattern: state)
    onMain(token) { view in
        view.completeClipboardRead(text: TerminalRuntime.pasteboard.string(forType: .string) ?? "",
                                   state: UnsafeMutableRawPointer(bitPattern: address))
    }
    return GHOSTTY_CLIPBOARD_READ_STARTED
}

private func ghosttyConfirmReadClipboard(
    _: UnsafeMutableRawPointer?, _: UnsafePointer<ghostty_clipboard_confirm_s>?,
    _: UnsafeMutableRawPointer?, _: ghostty_clipboard_request_e
) {}

private func ghosttyWriteClipboard(
    _: UnsafeMutableRawPointer?, _ location: ghostty_clipboard_e,
    _ content: UnsafePointer<ghostty_clipboard_content_s>?, _ count: Int, _: Bool
) {
    guard location == GHOSTTY_CLIPBOARD_STANDARD, let content else { return }
    for i in 0..<count {
        guard let mime = content[i].mime, String(cString: mime) == "text/plain",
              let data = content[i].data else { continue }
        let text = String(decoding: Data(bytes: data, count: content[i].len), as: UTF8.self)
        DispatchQueue.main.async {
            TerminalRuntime.pasteboard.clearContents()
            TerminalRuntime.pasteboard.setString(text, forType: .string)
        }
        return
    }
}
