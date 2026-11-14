import AppKit
import GhosttyKit

/// Owns the ghostty_app_t and the runtime callbacks. One per process.
final class GhosttyRuntime {
    private(set) var app: ghostty_app_t?
    private(set) var config: ghostty_config_t?

    init() {
        if ghostty_init(UInt(CommandLine.argc), CommandLine.unsafeArgv) != GHOSTTY_SUCCESS {
            fatalError("ghostty_init failed")
        }

        // Read the user's Ghostty config, exactly as Ghostty.app does.
        guard let cfg = ghostty_config_new() else { fatalError("ghostty_config_new failed") }
        ghostty_config_load_default_files(cfg)
        ghostty_config_load_recursive_files(cfg)
        ghostty_config_finalize(cfg)
        logConfigDiagnostics(cfg)
        config = cfg

        var rt = ghostty_runtime_config_s()
        rt.userdata = Unmanaged.passUnretained(self).toOpaque()
        rt.supports_selection_clipboard = false
        rt.wakeup_cb = { ud in
            let rt = Unmanaged<GhosttyRuntime>.fromOpaque(ud!).takeUnretainedValue()
            DispatchQueue.main.async { if let app = rt.app { ghostty_app_tick(app) } }
        }
        rt.action_cb = { app, target, action in GhosttyRuntime.handleAction(app!, target, action) }
        rt.read_clipboard_cb = { ud, loc, state, mimes, mimesLen, list in
            GhosttyRuntime.readClipboard(ud, loc, state, mimes, mimesLen, list)
        }
        rt.confirm_read_clipboard_cb = { _, _, _, _ in }
        rt.write_clipboard_cb = { _, loc, content, len, _ in
            GhosttyRuntime.writeClipboard(loc, content, len)
        }
        rt.close_surface_cb = { ud, processAlive in
            let view = SurfaceView.from(userdata: ud)
            NSLog("azj-spike: close_surface process_alive=\(processAlive)")
            DispatchQueue.main.async { view?.window?.close() }
        }

        guard let app = ghostty_app_new(&rt, cfg) else { fatalError("ghostty_app_new failed") }
        self.app = app
    }

    private func logConfigDiagnostics(_ cfg: ghostty_config_t) {
        let n = ghostty_config_diagnostics_count(cfg)
        for i in 0..<n {
            let d = ghostty_config_get_diagnostic(cfg, i)
            if let msg = d.message { NSLog("azj-spike: config diagnostic: \(String(cString: msg))") }
        }
    }

    private static func handleAction(
        _ app: ghostty_app_t, _ target: ghostty_target_s, _ action: ghostty_action_s
    ) -> Bool {
        switch action.tag {
        case GHOSTTY_ACTION_SET_TITLE:
            guard target.tag == GHOSTTY_TARGET_SURFACE,
                  let view = SurfaceView.from(surface: target.target.surface),
                  let cTitle = action.action.set_title.title else { return false }
            let title = String(cString: cTitle)
            DispatchQueue.main.async { view.window?.title = title }
            return true
        case GHOSTTY_ACTION_QUIT:
            DispatchQueue.main.async { NSApp.terminate(nil) }
            return true
        case GHOSTTY_ACTION_SHOW_CHILD_EXITED:
            NSLog("azj-spike: child exited")
            return false
        default:
            return false
        }
    }

    private static func readClipboard(
        _ ud: UnsafeMutableRawPointer?, _ loc: ghostty_clipboard_e, _ state: UnsafeMutableRawPointer?,
        _ mimes: UnsafePointer<UnsafePointer<CChar>?>?, _ mimesLen: Int, _ list: Bool
    ) -> ghostty_clipboard_read_result_e {
        guard loc == GHOSTTY_CLIPBOARD_STANDARD,
              let view = SurfaceView.from(userdata: ud), let surface = view.surface,
              let text = NSPasteboard.general.string(forType: .string) else {
            return GHOSTTY_CLIPBOARD_READ_UNAVAILABLE
        }
        // Spike: serve text/plain only.
        "text/plain".withCString { mime in
            text.withCString { data in
                var content = ghostty_clipboard_content_s(mime: mime, data: data, len: strlen(data))
                withUnsafePointer(to: &content) { ptr in
                    var done = ghostty_clipboard_complete_s()
                    done.contents = ptr
                    done.contents_len = 1
                    done.confirmed = false
                    ghostty_surface_complete_clipboard_request(surface, &done, state)
                }
            }
        }
        return GHOSTTY_CLIPBOARD_READ_STARTED
    }

    private static func writeClipboard(
        _ loc: ghostty_clipboard_e, _ content: UnsafePointer<ghostty_clipboard_content_s>?, _ len: Int
    ) {
        guard loc == GHOSTTY_CLIPBOARD_STANDARD, let content else { return }
        for i in 0..<len where String(cString: content[i].mime) == "text/plain" {
            let data = Data(bytes: content[i].data, count: content[i].len)
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(String(decoding: data, as: UTF8.self), forType: .string)
        }
    }
}
