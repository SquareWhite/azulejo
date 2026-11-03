import AppKit
import GhosttyKit

/// An NSView backed by one libghostty surface. libghostty installs its own
/// Metal layer on the view and renders on its own thread.
final class SurfaceView: NSView {
    private(set) var surface: ghostty_surface_t?
    private var markedText = NSMutableAttributedString()

    /// Spike instrumentation: how many times we told libghostty the pixel
    /// size changed (each one becomes one TIOCSWINSZ on the attach PTY,
    /// which herdr forwards to the pane).
    let name: String
    private(set) var resizeCount = 0
    private(set) var lastPixelSize = CGSize.zero
    var onResize: ((SurfaceView, CGSize) -> Void)?

    init(runtime: GhosttyRuntime, name: String, command: String, env: [String: String], frame: NSRect) {
        self.name = name
        super.init(frame: frame)
        autoresizingMask = []

        var cfg = ghostty_surface_config_new()
        cfg.platform_tag = GHOSTTY_PLATFORM_MACOS
        cfg.platform = ghostty_platform_u(macos: ghostty_platform_macos_s(
            nsview: Unmanaged.passUnretained(self).toOpaque()))
        cfg.userdata = Unmanaged.passUnretained(self).toOpaque()
        cfg.scale_factor = Double(NSScreen.main?.backingScaleFactor ?? 2.0)
        cfg.context = GHOSTTY_SURFACE_CONTEXT_WINDOW

        // Keep C strings alive for the duration of ghostty_surface_new.
        let keys = env.keys.map { strdup($0)! }
        let values = env.keys.map { strdup(env[$0]!)! }
        defer { (keys + values).forEach { free($0) } }
        var vars = zip(keys, values).map { ghostty_env_var_s(key: $0, value: $1) }

        surface = command.withCString { cmd in
            vars.withUnsafeMutableBufferPointer { buf in
                cfg.command = cmd
                cfg.env_vars = buf.baseAddress
                cfg.env_var_count = buf.count
                return ghostty_surface_new(runtime.app, &cfg)
            }
        }
        guard surface != nil else { fatalError("ghostty_surface_new failed") }
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    deinit { if let surface { ghostty_surface_free(surface) } }

    static func from(userdata: UnsafeMutableRawPointer?) -> SurfaceView? {
        guard let userdata else { return nil }
        return Unmanaged<SurfaceView>.fromOpaque(userdata).takeUnretainedValue()
    }

    static func from(surface: ghostty_surface_t?) -> SurfaceView? {
        guard let surface else { return nil }
        return from(userdata: ghostty_surface_userdata(surface))
    }

    // MARK: Size and focus

    override var acceptsFirstResponder: Bool { true }

    override func becomeFirstResponder() -> Bool {
        if let surface { ghostty_surface_set_focus(surface, true) }
        return true
    }

    override func resignFirstResponder() -> Bool {
        if let surface { ghostty_surface_set_focus(surface, false) }
        return true
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        updateSurfaceSize()
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        guard let surface else { return }
        let fb = convertToBacking(bounds)
        let scale = bounds.width > 0 ? fb.width / bounds.width : 2.0
        layer?.contentsScale = scale
        ghostty_surface_set_content_scale(surface, scale, scale)
        updateSurfaceSize()
    }

    private func updateSurfaceSize() {
        guard let surface else { return }
        let px = convertToBacking(NSRect(origin: .zero, size: bounds.size)).size
        guard px.width >= 1, px.height >= 1 else { return } // attach rejects 0x0
        if px != lastPixelSize {
            resizeCount += 1
            lastPixelSize = px
            onResize?(self, px)
        }
        ghostty_surface_set_size(surface, UInt32(px.width), UInt32(px.height))
    }

    // MARK: Snapshot

    /// The IOSurface libghostty last presented (its layer's `contents`).
    var presentedSurface: IOSurfaceRef? {
        guard let c = layer?.contents else { return nil }
        let cf = c as CFTypeRef
        guard CFGetTypeID(cf) == IOSurfaceGetTypeID() else { return nil }
        return (cf as! IOSurfaceRef)
    }

    /// Copy the last presented frame into a fresh IOSurface. We copy rather
    /// than hold libghostty's IOSurface because it keeps rendering into its
    /// swap chain while we animate. An IOSurface (not a CGImage) so Core
    /// Animation can use it as layer contents with no conversion at commit.
    func snapshot() -> IOSurfaceRef? {
        guard let src = presentedSurface else { return nil }
        let w = IOSurfaceGetWidth(src), h = IOSurfaceGetHeight(src), bpr = IOSurfaceGetBytesPerRow(src)
        guard w > 0, h > 0 else { return nil }
        let props: [CFString: Any] = [
            kIOSurfaceWidth: w, kIOSurfaceHeight: h,
            kIOSurfaceBytesPerElement: IOSurfaceGetBytesPerElement(src),
            kIOSurfaceBytesPerRow: bpr,
            kIOSurfacePixelFormat: IOSurfaceGetPixelFormat(src),
        ]
        guard let dst = IOSurfaceCreate(props as CFDictionary) else { return nil }
        if let cs = IOSurfaceCopyValue(src, kIOSurfaceColorSpace) { IOSurfaceSetValue(dst, kIOSurfaceColorSpace, cs) }
        IOSurfaceLock(src, .readOnly, nil)
        IOSurfaceLock(dst, [], nil)
        memcpy(IOSurfaceGetBaseAddress(dst), IOSurfaceGetBaseAddress(src), bpr * h)
        IOSurfaceUnlock(dst, [], nil)
        IOSurfaceUnlock(src, .readOnly, nil)
        return dst
    }
}
