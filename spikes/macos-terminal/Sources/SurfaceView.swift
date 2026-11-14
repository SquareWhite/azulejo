import AppKit
import GhosttyKit

/// An NSView backed by one libghostty surface. libghostty installs its own
/// Metal layer on the view and renders on its own thread.
final class SurfaceView: NSView {
    private(set) var surface: ghostty_surface_t?
    private var markedText = NSMutableAttributedString()

    init(runtime: GhosttyRuntime, command: String, env: [String: String], frame: NSRect) {
        super.init(frame: frame)

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
        ghostty_surface_set_size(surface, UInt32(px.width), UInt32(px.height))
    }

    // MARK: Keyboard

    override func keyDown(with event: NSEvent) {
        sendKey(event, action: event.isARepeat ? GHOSTTY_ACTION_REPEAT : GHOSTTY_ACTION_PRESS)
    }

    override func keyUp(with event: NSEvent) {
        sendKey(event, action: GHOSTTY_ACTION_RELEASE)
    }

    override func flagsChanged(with event: NSEvent) {
        guard let surface else { return }
        var key = keyEvent(event, action: GHOSTTY_ACTION_PRESS)
        key.text = nil
        _ = ghostty_surface_key(surface, key)
    }

    // Let ⌘-shortcuts reach libghostty's keybindings (copy, paste, etc.)
    // before AppKit swallows them; ⌘Q still goes to the menu.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard window?.firstResponder === self, event.type == .keyDown else { return false }
        if event.modifierFlags.contains(.command), event.charactersIgnoringModifiers == "q" { return false }
        guard let surface else { return false }
        var flags = ghostty_binding_flags_e(0)
        let key = keyEvent(event, action: GHOSTTY_ACTION_PRESS)
        guard ghostty_surface_key_is_binding(surface, key, &flags) else { return false }
        sendKey(event, action: GHOSTTY_ACTION_PRESS)
        return true
    }

    private func sendKey(_ event: NSEvent, action: ghostty_input_action_e) {
        guard let surface else { return }
        var key = keyEvent(event, action: action)
        // Spike: no IME. Text comes straight from the event (Ghostty's own
        // control-character rule: send the un-ctrl'd character).
        let text = action == GHOSTTY_ACTION_RELEASE ? nil : ghosttyCharacters(event)
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
        key.mods = mods(event.modifierFlags)
        key.consumed_mods = mods(event.modifierFlags.subtracting([.control, .command]))
        key.composing = false
        if event.type == .keyDown || event.type == .keyUp,
           let chars = event.characters(byApplyingModifiers: []),
           let cp = chars.unicodeScalars.first {
            key.unshifted_codepoint = cp.value
        }
        return key
    }

    private func ghosttyCharacters(_ event: NSEvent) -> String? {
        guard let chars = event.characters else { return nil }
        if chars.count == 1, let scalar = chars.unicodeScalars.first {
            if scalar.value < 0x20 {
                return event.characters(byApplyingModifiers: event.modifierFlags.subtracting(.control))
            }
            if scalar.value >= 0xF700 && scalar.value <= 0xF8FF { return nil }
        }
        return chars
    }

    private func mods(_ flags: NSEvent.ModifierFlags) -> ghostty_input_mods_e {
        var m = GHOSTTY_MODS_NONE.rawValue
        if flags.contains(.shift) { m |= GHOSTTY_MODS_SHIFT.rawValue }
        if flags.contains(.control) { m |= GHOSTTY_MODS_CTRL.rawValue }
        if flags.contains(.option) { m |= GHOSTTY_MODS_ALT.rawValue }
        if flags.contains(.command) { m |= GHOSTTY_MODS_SUPER.rawValue }
        if flags.contains(.capsLock) { m |= GHOSTTY_MODS_CAPS.rawValue }
        return ghostty_input_mods_e(m)
    }

    // MARK: Mouse

    override func updateTrackingAreas() {
        trackingAreas.forEach { removeTrackingArea($0) }
        addTrackingArea(NSTrackingArea(
            rect: bounds,
            options: [.mouseMoved, .mouseEnteredAndExited, .inVisibleRect, .activeAlways],
            owner: self))
        super.updateTrackingAreas()
    }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        mouseButton(event, GHOSTTY_MOUSE_PRESS, GHOSTTY_MOUSE_LEFT)
    }
    override func mouseUp(with event: NSEvent) { mouseButton(event, GHOSTTY_MOUSE_RELEASE, GHOSTTY_MOUSE_LEFT) }
    override func rightMouseDown(with event: NSEvent) { mouseButton(event, GHOSTTY_MOUSE_PRESS, GHOSTTY_MOUSE_RIGHT) }
    override func rightMouseUp(with event: NSEvent) { mouseButton(event, GHOSTTY_MOUSE_RELEASE, GHOSTTY_MOUSE_RIGHT) }
    override func mouseMoved(with event: NSEvent) { mousePos(event) }
    override func mouseDragged(with event: NSEvent) { mousePos(event) }

    override func scrollWheel(with event: NSEvent) {
        guard let surface else { return }
        var x = event.scrollingDeltaX
        var y = event.scrollingDeltaY
        var scrollMods: Int32 = 0
        if event.hasPreciseScrollingDeltas {
            x *= 2; y *= 2
            scrollMods |= 1 // precision bit
        }
        ghostty_surface_mouse_scroll(surface, x, y, ghostty_input_scroll_mods_t(scrollMods))
    }

    private func mouseButton(_ event: NSEvent, _ state: ghostty_input_mouse_state_e, _ button: ghostty_input_mouse_button_e) {
        guard let surface else { return }
        mousePos(event)
        _ = ghostty_surface_mouse_button(surface, state, button, mods(event.modifierFlags))
    }

    private func mousePos(_ event: NSEvent) {
        guard let surface else { return }
        let p = convert(event.locationInWindow, from: nil)
        ghostty_surface_mouse_pos(surface, p.x, bounds.height - p.y, mods(event.modifierFlags))
    }
}
