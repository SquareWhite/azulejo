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
        stopProbe()
        TerminalRuntime.shared?.unregister(token)
        onExit = nil
        if let surface {
            self.surface = nil
            ghostty_surface_free(surface)
        }
    }

    isolated deinit {
        probe?.cancel()
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
        let changed = px != lastPixelSize
        if changed {
            lastPixelSize = px
            resizeCount += 1
        }
        ghostty_surface_set_size(surface, UInt32(px.width), UInt32(px.height))
        if changed { startDrawProbe() }
    }

    /// Times the surface (and so the PTY) changed pixel size; motion stats.
    private(set) var resizeCount = 0
    private(set) var lastPixelSize = CGSize.zero

    // MARK: Draw probe

    /// Short tile id for logs.
    var logName = ""
    /// Where draw-probe lines go (the app log).
    static var log: (String) -> Void = { _ in }

    /// When the surface last changed pixel size.
    private(set) var resizedAt: CFTimeInterval = 0
    /// When a new presented frame was last seen (the probe only looks after
    /// a resize), and its identity.
    private(set) var lastFrameSeenAt: CFTimeInterval = 0
    private var lastFrameKey: (UnsafeMutableRawPointer, UInt32)?
    /// Whether libghostty has presented a frame at `lastPixelSize` since the
    /// last resize (nil while the probe is still looking).
    private(set) var drewAtSize: Bool?
    private var probe: DispatchSourceTimer?

    /// The presented frame's identity and seed: a new frame changes one.
    private var presentedKey: (UnsafeMutableRawPointer, UInt32)? {
        guard let s = presentedSurface else { return nil }
        return (Unmanaged.passUnretained(s).toOpaque(), IOSurfaceGetSeed(s))
    }

    /// Whether the presented frame has the surface's current pixel size.
    var presentedAtSize: Bool {
        guard let s = presentedSurface else { return false }
        return IOSurfaceGetWidth(s) == Int(lastPixelSize.width) && IOSurfaceGetHeight(s) == Int(lastPixelSize.height)
    }

    /// The presented frame's size in pixels, for logs.
    var presentedSizeText: String {
        guard let s = presentedSurface else { return "none" }
        return "\(IOSurfaceGetWidth(s))x\(IOSurfaceGetHeight(s))"
    }

    /// After a resize, watch (on a plain timer, not the display link, which
    /// stops with the screen locked) for libghostty's first frame at the new
    /// size, and log when it came or that it didn't within 2 s. Tells a
    /// reveal bug from a renderer that isn't drawing.
    private func startDrawProbe() {
        resizedAt = CACurrentMediaTime()
        drewAtSize = nil
        let before = presentedKey
        lastFrameKey = before
        guard probe == nil else { return }
        let timer = DispatchSource.makeTimerSource(queue: .main)
        timer.schedule(deadline: .now() + .milliseconds(5), repeating: .milliseconds(5))
        timer.setEventHandler { [weak self] in
            MainActor.assumeIsolated { self?.probeTick() }
        }
        probe = timer
        timer.resume()
    }

    private func probeTick() {
        let now = CACurrentMediaTime()
        let key = presentedKey
        if let key, lastFrameKey.map({ $0.0 != key.0 || $0.1 != key.1 }) ?? true {
            lastFrameKey = key
            lastFrameSeenAt = now
            if presentedAtSize {
                drewAtSize = true
                stopProbe()
                Self.log("terminal \(logName): drew at \(Int(lastPixelSize.width))x\(Int(lastPixelSize.height)) px "
                    + "\(Int((now - resizedAt) * 1000)) ms after the resize")
                return
            }
        }
        if now - resizedAt > 2 {
            drewAtSize = false
            stopProbe()
            Self.log("terminal \(logName): NO frame at \(Int(lastPixelSize.width))x\(Int(lastPixelSize.height)) px "
                + "2 s after the resize (shows \(presentedSizeText)); libghostty isn't drawing")
        }
    }

    private func stopProbe() {
        probe?.cancel()
        probe = nil
    }

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

    /// Input-method composition (IME, dead keys): shown as libghostty preedit,
    /// never sent to the PTY until the input method commits it.
    private var markedText = NSMutableAttributedString()
    /// Text the input method commits while a keyDown is being interpreted.
    private var keyTextAccumulator: [String]?

    var markedString: String { markedText.string }
    var hasSelection: Bool { surface.map { ghostty_surface_has_selection($0) } ?? false }

    override func keyDown(with event: NSEvent) {
        guard !readOnly else { return }
        let action = event.isARepeat ? GHOSTTY_ACTION_REPEAT : GHOSTTY_ACTION_PRESS
        let markedBefore = markedText.length > 0
        if !markedBefore, Self.isPlainCtrlB(event) {
            // Literal ctrl+b, never the start of herdr's detach chord.
            sendKey(event, action: action)
            sendKey(event, action: GHOSTTY_ACTION_RELEASE)
            sendKey(event, action: action)
            return
        }

        // The input method sees the key first, as in Ghostty.app: with the
        // option-as-alt config translated away so alt chords stay alt chords.
        keyTextAccumulator = []
        defer { keyTextAccumulator = nil }
        interpretKeyEvents([translated(event)])
        syncPreedit(clearIfNeeded: markedBefore)
        let composing = markedText.length > 0 || markedBefore

        if let list = keyTextAccumulator, !list.isEmpty {
            for text in list where !(composing && Self.isControl(text)) {
                // A commit that ends a composition is text, not this key.
                if markedBefore { sendText(text) } else { sendKey(event, action: action, text: text) }
            }
            return
        }
        // The input method kept the key (composing, or it just cancelled).
        if composing { return }
        sendKey(event, action: action)
    }

    /// The event the input method sees: modifiers libghostty translates
    /// away (`macos-option-as-alt`) removed.
    private func translated(_ event: NSEvent) -> NSEvent {
        guard let surface else { return event }
        let kept = Self.flags(ghostty_surface_key_translation_mods(surface, Self.mods(event.modifierFlags)))
        var flags = event.modifierFlags
        for flag in [NSEvent.ModifierFlags.shift, .control, .option, .command] {
            if kept.contains(flag) { flags.insert(flag) } else { flags.remove(flag) }
        }
        // The same event object when nothing changes: some input methods
        // (Korean) depend on it.
        guard flags != event.modifierFlags else { return event }
        return NSEvent.keyEvent(
            with: event.type, location: event.locationInWindow, modifierFlags: flags,
            timestamp: event.timestamp, windowNumber: event.windowNumber, context: nil,
            characters: event.characters(byApplyingModifiers: flags) ?? "",
            charactersIgnoringModifiers: event.charactersIgnoringModifiers ?? "",
            isARepeat: event.isARepeat, keyCode: event.keyCode) ?? event
    }

    private static func isControl(_ text: String) -> Bool {
        text.unicodeScalars.count == 1 && (text.unicodeScalars.first?.value ?? 0x20) < 0x20
    }

    private func syncPreedit(clearIfNeeded: Bool = true) {
        guard let surface else { return }
        if markedText.length > 0 {
            let text = markedText.string
            text.withCString { ghostty_surface_preedit(surface, $0, UInt(text.utf8.count)) }
        } else if clearIfNeeded {
            ghostty_surface_preedit(surface, nil, 0)
        }
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

    /// Committed input-method text, sent as typed input (never as a paste).
    private func sendText(_ text: String) {
        guard let surface, !text.isEmpty else { return }
        var key = ghostty_input_key_s()
        key.action = GHOSTTY_ACTION_PRESS
        key.mods = GHOSTTY_MODS_NONE
        key.consumed_mods = GHOSTTY_MODS_NONE
        text.withCString { ptr in
            key.text = ptr
            _ = ghostty_surface_key(surface, key)
        }
    }

    private func sendKey(_ event: NSEvent, action: ghostty_input_action_e, text override: String? = nil) {
        guard let surface else { return }
        var key = keyEvent(event, action: action)
        let text = action == GHOSTTY_ACTION_RELEASE ? nil : (override ?? Self.text(for: event))
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

    private static func flags(_ mods: ghostty_input_mods_e) -> NSEvent.ModifierFlags {
        var flags: NSEvent.ModifierFlags = []
        if mods.rawValue & GHOSTTY_MODS_SHIFT.rawValue != 0 { flags.insert(.shift) }
        if mods.rawValue & GHOSTTY_MODS_CTRL.rawValue != 0 { flags.insert(.control) }
        if mods.rawValue & GHOSTTY_MODS_ALT.rawValue != 0 { flags.insert(.option) }
        if mods.rawValue & GHOSTTY_MODS_SUPER.rawValue != 0 { flags.insert(.command) }
        return flags
    }

    /// Commands the input method picks for a key (`insertNewline:`,
    /// `cancelOperation:`…): the key itself goes to libghostty from keyDown,
    /// so nothing runs here (and nothing beeps).
    override func doCommand(by selector: Selector) {}

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

    // MARK: Search highlight (find bar)

    /// Runs a libghostty binding action (`search:<text>`, `end_search`).
    @discardableResult
    func bindingAction(_ action: String) -> Bool {
        guard let surface else { return false }
        return ghostty_surface_binding_action(surface, action, UInt(action.utf8.count))
    }

    /// Occurrences libghostty's search marks on this screen (what's in view;
    /// herdr holds the rest), or -1 with no search running.
    private(set) var searchTotal = -1

    func searchChanged(total: Int?) { searchTotal = total ?? -1 }

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

// MARK: - NSTextInputClient

/// Input methods (Japanese, Chinese, Korean, dead keys) talk to the surface
/// through this: marked text is libghostty preedit, committed text is typed
/// input. While text is marked the key router lets every key through to the
/// input method, so app chords can't fire mid-composition.
extension TerminalSurfaceView: @preconcurrency NSTextInputClient {
    func hasMarkedText() -> Bool { markedText.length > 0 }

    func markedRange() -> NSRange {
        markedText.length > 0 ? NSRange(location: 0, length: markedText.length) : NSRange(location: NSNotFound, length: 0)
    }

    func selectedRange() -> NSRange { NSRange(location: NSNotFound, length: 0) }

    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        switch string {
        case let text as NSAttributedString: markedText = NSMutableAttributedString(attributedString: text)
        case let text as String: markedText = NSMutableAttributedString(string: text)
        default: return
        }
        // Outside keyDown (an input-source switch mid-composition), show it now.
        if keyTextAccumulator == nil { syncPreedit() }
    }

    func unmarkText() {
        guard markedText.length > 0 else { return }
        markedText.mutableString.setString("")
        syncPreedit()
    }

    func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }

    func attributedSubstring(forProposedRange range: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? {
        nil
    }

    func characterIndex(for point: NSPoint) -> Int { 0 }

    /// Where the input method's candidate window goes: the cursor.
    func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?) -> NSRect {
        guard let surface, let window else { return .zero }
        var x = 0.0, y = 0.0, width = 0.0, height = 0.0
        ghostty_surface_ime_point(surface, &x, &y, &width, &height)
        let rect = NSRect(x: x, y: bounds.height - y, width: 0, height: max(height, 1))
        return window.convertToScreen(convert(rect, to: nil))
    }

    func insertText(_ string: Any, replacementRange: NSRange) {
        let text: String
        switch string {
        case let s as NSAttributedString: text = s.string
        case let s as String: text = s
        default: return
        }
        // A commit ends the composition.
        unmarkText()
        if keyTextAccumulator != nil {
            keyTextAccumulator?.append(text)
        } else if !readOnly {
            sendText(text)
        }
    }
}
