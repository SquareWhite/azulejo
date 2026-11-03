import AppKit
import IOSurface
import GhosttyKit

/// One libghostty surface: the user's Ghostty config, rendering on its own
/// Metal layer, running one child command (a herdr attach or observe).
///
/// Keys: the window's key router sees every keyDown first and only lets
/// unbound chords through, so `keyDown` here is plain typing. A typed
/// `ctrl+b` goes out twice: herdr's attach client reads `ctrl+b ctrl+b` as a
/// literal `ctrl+b`, so `ctrl+b q` can never detach the tile by accident
/// (docs/keybindings.md). A read-only surface (observe mode) swallows keys.
@MainActor
final class TerminalSurfaceView: NSView {
    private(set) var surface: ghostty_surface_t?
    private var token: UInt = 0
    private var exited = false
    /// Observe mode: input goes nowhere.
    let readOnly: Bool
    /// Runs once when the child process ends.
    var onExit: (() -> Void)?
    var onTitle: ((String) -> Void)?
    /// Focus requests from clicks, so the core's focus follows the mouse.
    var onClickFocus: (() -> Void)?

    /// The surface's grid size, for observe's `--cols/--rows`.
    var gridSize: (cols: Int, rows: Int)? {
        guard let surface else { return nil }
        let size = ghostty_surface_size(surface)
        guard size.columns > 0, size.rows > 0 else { return nil }
        return (Int(size.columns), Int(size.rows))
    }

    /// Creates the surface and starts `command`. Call only once the view has a
    /// non-zero frame: the PTY takes the surface's size before the child runs
    /// (herdr's attach rejects a zero-sized PTY; ADR 0006).
    init?(runtime: TerminalRuntime, frame: NSRect, command: String, readOnly: Bool) {
        self.readOnly = readOnly
        super.init(frame: frame)
        token = runtime.register(self)

        var cfg = ghostty_surface_config_new()
        cfg.platform_tag = GHOSTTY_PLATFORM_MACOS
        cfg.platform = ghostty_platform_u(macos: ghostty_platform_macos_s(
            nsview: Unmanaged.passUnretained(self).toOpaque()))
        cfg.userdata = UnsafeMutableRawPointer(bitPattern: token)
        cfg.scale_factor = Double(NSScreen.main?.backingScaleFactor ?? 2)
        cfg.context = GHOSTTY_SURFACE_CONTEXT_SPLIT
        cfg.wait_after_command = true
        surface = command.withCString { cmd in
            cfg.command = cmd
            return ghostty_surface_new(runtime.app, &cfg)
        }
        guard surface != nil else {
            runtime.unregister(token)
            return nil
        }
        updateSurfaceSize()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Stops the child (SIGHUP via the PTY closing) and frees the surface.
    func close() {
        TerminalRuntime.shared?.unregister(token)
        onExit = nil
        if let surface {
            self.surface = nil
            ghostty_surface_free(surface)
        }
    }

    isolated deinit {
        if let surface { ghostty_surface_free(surface) }
        TerminalRuntime.shared?.unregister(token)
    }

    func childExited() {
        guard !exited else { return }
        exited = true
        onExit?()
    }

    func titleChanged(_ title: String) { onTitle?(title) }

    func setMouseShape(_ shape: ghostty_action_mouse_shape_e) {
        let cursor: NSCursor = switch shape {
        case GHOSTTY_MOUSE_SHAPE_TEXT: .iBeam
        case GHOSTTY_MOUSE_SHAPE_POINTER: .pointingHand
        default: .arrow
        }
        cursor.set()
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
        let backing = convertToBacking(bounds)
        let scale = bounds.width > 0 ? backing.width / bounds.width : 2
        layer?.contentsScale = scale
        ghostty_surface_set_content_scale(surface, scale, scale)
        updateSurfaceSize()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard let surface, let screen = window?.screen,
              let id = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? UInt32
        else { return }
        ghostty_surface_set_display_id(surface, id)
        viewDidChangeBackingProperties()
    }

    /// Every size change reaches the PTY (and so the herdr pane) at once.
    /// During layout motion the tile sets this view's frame once, at the
    /// start, while a snapshot covers it (ADR 0004).
    private func updateSurfaceSize() {
        guard let surface, bounds.width > 0, bounds.height > 0 else { return }
        let px = convertToBacking(NSRect(origin: .zero, size: bounds.size)).size
        if px != lastPixelSize {
            lastPixelSize = px
            resizeCount += 1
        }
        ghostty_surface_set_size(surface, UInt32(px.width), UInt32(px.height))
    }

    /// Times the surface (and so the PTY) changed pixel size; motion stats.
    private(set) var resizeCount = 0
    private(set) var lastPixelSize = CGSize.zero

    // MARK: Snapshot

    /// The IOSurface libghostty last presented (its layer's `contents`).
    var presentedSurface: IOSurfaceRef? {
        guard let c = layer?.contents else { return nil }
        let cf = c as CFTypeRef
        guard CFGetTypeID(cf) == IOSurfaceGetTypeID() else { return nil }
        return (cf as! IOSurfaceRef)
    }

    /// A copy of the last presented frame. A copy because libghostty keeps
    /// rendering into its swap chain; an IOSurface (not a CGImage) so Core
    /// Animation shows it with no conversion at commit (1–2 ms, spike 0.5).
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

    // MARK: Keyboard

    override func keyDown(with event: NSEvent) {
        guard !readOnly else { return }
        let action = event.isARepeat ? GHOSTTY_ACTION_REPEAT : GHOSTTY_ACTION_PRESS
        if Self.isPlainCtrlB(event) {
            // Literal ctrl+b, never the start of herdr's detach chord.
            sendKey(event, action: action)
            sendKey(event, action: GHOSTTY_ACTION_RELEASE)
        }
        sendKey(event, action: action)
    }

    override func keyUp(with event: NSEvent) {
        guard !readOnly else { return }
        sendKey(event, action: GHOSTTY_ACTION_RELEASE)
    }

    override func flagsChanged(with event: NSEvent) {
        guard !readOnly, let surface else { return }
        var key = keyEvent(event, action: GHOSTTY_ACTION_PRESS)
        key.text = nil
        _ = ghostty_surface_key(surface, key)
    }

    static func isPlainCtrlB(_ event: NSEvent) -> Bool {
        let flags = event.modifierFlags.intersection([.control, .option, .command, .shift])
        return flags == .control && event.keyCode == 11 // kVK_ANSI_B
    }

    private func sendKey(_ event: NSEvent, action: ghostty_input_action_e) {
        guard let surface else { return }
        var key = keyEvent(event, action: action)
        let text = action == GHOSTTY_ACTION_RELEASE ? nil : Self.text(for: event)
        if let text, !text.isEmpty {
            text.withCString { ptr in
                key.text = ptr
                _ = ghostty_surface_key(surface, key)
            }
        } else {
            _ = ghostty_surface_key(surface, key)
        }
    }

    private func keyEvent(_ event: NSEvent, action: ghostty_input_action_e) -> ghostty_input_key_s {
        var key = ghostty_input_key_s()
        key.action = action
        key.keycode = UInt32(event.keyCode)
        key.mods = Self.mods(event.modifierFlags)
        key.consumed_mods = Self.mods(event.modifierFlags.subtracting([.control, .command]))
        key.composing = false
        if event.type == .keyDown || event.type == .keyUp,
           let chars = event.characters(byApplyingModifiers: []),
           let scalar = chars.unicodeScalars.first {
            key.unshifted_codepoint = scalar.value
        }
        return key
    }

    /// The text a key produces, by Ghostty's rule for control characters
    /// (send the un-ctrl'd character; libghostty encodes the control).
    private static func text(for event: NSEvent) -> String? {
        guard let chars = event.characters else { return nil }
        if chars.count == 1, let scalar = chars.unicodeScalars.first {
            if scalar.value < 0x20 {
                return event.characters(byApplyingModifiers: event.modifierFlags.subtracting(.control))
            }
            if scalar.value >= 0xF700 && scalar.value <= 0xF8FF { return nil }
        }
        return chars
    }

    private static func mods(_ flags: NSEvent.ModifierFlags) -> ghostty_input_mods_e {
        var m = GHOSTTY_MODS_NONE.rawValue
        if flags.contains(.shift) { m |= GHOSTTY_MODS_SHIFT.rawValue }
        if flags.contains(.control) { m |= GHOSTTY_MODS_CTRL.rawValue }
        if flags.contains(.option) { m |= GHOSTTY_MODS_ALT.rawValue }
        if flags.contains(.command) { m |= GHOSTTY_MODS_SUPER.rawValue }
        if flags.contains(.capsLock) { m |= GHOSTTY_MODS_CAPS.rawValue }
        return ghostty_input_mods_e(m)
    }

    // MARK: Edit menu

    @objc func copy(_ sender: Any?) {
        guard let surface else { return }
        let action = "copy_to_clipboard"
        _ = ghostty_surface_binding_action(surface, action, UInt(action.utf8.count))
    }

    @objc func paste(_ sender: Any?) {
        guard !readOnly, let surface else { return }
        let action = "paste_from_clipboard"
        _ = ghostty_surface_binding_action(surface, action, UInt(action.utf8.count))
    }

    @objc override func selectAll(_ sender: Any?) {
        guard let surface else { return }
        let action = "select_all"
        _ = ghostty_surface_binding_action(surface, action, UInt(action.utf8.count))
    }

    func completeClipboardRead(text: String, state: UnsafeMutableRawPointer?) {
        guard let surface else { return }
        "text/plain".withCString { mime in
            text.withCString { data in
                var content = ghostty_clipboard_content_s(mime: mime, data: data, len: strlen(data))
                withUnsafePointer(to: &content) { pointer in
                    var done = ghostty_clipboard_complete_s()
                    done.contents = pointer
                    done.contents_len = 1
                    done.confirmed = false
                    ghostty_surface_complete_clipboard_request(surface, &done, state)
                }
            }
        }
    }

    // MARK: Mouse

    override func updateTrackingAreas() {
        trackingAreas.forEach { removeTrackingArea($0) }
        addTrackingArea(NSTrackingArea(
            rect: bounds,
            options: [.mouseMoved, .mouseEnteredAndExited, .inVisibleRect, .activeInKeyWindow],
            owner: self))
        super.updateTrackingAreas()
    }

    override func mouseDown(with event: NSEvent) {
        onClickFocus?()
        window?.makeFirstResponder(self)
        mouseButton(event, GHOSTTY_MOUSE_PRESS, GHOSTTY_MOUSE_LEFT)
    }

    override func mouseUp(with event: NSEvent) { mouseButton(event, GHOSTTY_MOUSE_RELEASE, GHOSTTY_MOUSE_LEFT) }
    override func rightMouseDown(with event: NSEvent) { mouseButton(event, GHOSTTY_MOUSE_PRESS, GHOSTTY_MOUSE_RIGHT) }
    override func rightMouseUp(with event: NSEvent) { mouseButton(event, GHOSTTY_MOUSE_RELEASE, GHOSTTY_MOUSE_RIGHT) }
    override func otherMouseDown(with event: NSEvent) { mouseButton(event, GHOSTTY_MOUSE_PRESS, GHOSTTY_MOUSE_MIDDLE) }
    override func otherMouseUp(with event: NSEvent) { mouseButton(event, GHOSTTY_MOUSE_RELEASE, GHOSTTY_MOUSE_MIDDLE) }
    override func mouseMoved(with event: NSEvent) { mousePos(event) }
    override func mouseDragged(with event: NSEvent) { mousePos(event) }
    override func rightMouseDragged(with event: NSEvent) { mousePos(event) }

    /// The attach client turns on mouse reporting, so wheel events go to herdr,
    /// which scrolls its own history (ADR 0005).
    override func scrollWheel(with event: NSEvent) {
        guard !readOnly, let surface else { return }
        var x = event.scrollingDeltaX
        var y = event.scrollingDeltaY
        var mods: Int32 = 0
        if event.hasPreciseScrollingDeltas {
            x *= 2
            y *= 2
            mods |= 1
        }
        ghostty_surface_mouse_scroll(surface, x, y, ghostty_input_scroll_mods_t(mods))
    }

    private func mouseButton(
        _ event: NSEvent, _ state: ghostty_input_mouse_state_e, _ button: ghostty_input_mouse_button_e
    ) {
        guard let surface else { return }
        mousePos(event)
        _ = ghostty_surface_mouse_button(surface, state, button, Self.mods(event.modifierFlags))
    }

    private func mousePos(_ event: NSEvent) {
        guard let surface else { return }
        let p = convert(event.locationInWindow, from: nil)
        ghostty_surface_mouse_pos(surface, p.x, bounds.height - p.y, Self.mods(event.modifierFlags))
    }
}
