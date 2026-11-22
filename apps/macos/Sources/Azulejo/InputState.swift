import AppKit

/// The one place the app reads global input state: the modifier keys held
/// right now, where the pointer is, the cursor showing, the key event being
/// handled.
///
/// A normal run asks AppKit, which asks the hardware. An automation run
/// (`AZULEJO_AUTOMATION`) never does: the `.azs` runner drives the app with
/// synthetic events while the user keeps working, so a key they hold or
/// where their pointer sits must not reach the app. There, the answers are
/// what the runner's own input says: the modifiers a simulated drop or
/// mouse gesture holds (`holding`, `hold`), its in-app pointer
/// (`DividerHoverAutomation`), and the cursor the app set last.
///
/// `scripts/check-input-state` (in `just lint`) fails on any of AppKit's
/// global input reads outside this file.
@MainActor
enum InputState {
    /// An automation run: everything here comes from the runner.
    nonisolated static let simulated =
        ProcessInfo.processInfo.environment["AZULEJO_AUTOMATION"].map { !$0.isEmpty } ?? false

    // MARK: Modifiers

    private static var heldModifiers: NSEvent.ModifierFlags = []

    /// The modifier keys held now.
    static var modifiers: NSEvent.ModifierFlags {
        simulated ? heldModifiers : NSEvent.modifierFlags
    }

    /// Automation: from now on `flags` are held (none: everything let go).
    static func hold(_ flags: NSEvent.ModifierFlags) {
        heldModifiers = flags
    }

    /// Automation: `flags` held while `body` runs (a drop or click made with
    /// them), then what was held before.
    static func holding<T>(_ flags: NSEvent.ModifierFlags, _ body: () throws -> T) rethrows -> T {
        let before = heldModifiers
        heldModifiers = flags
        defer { heldModifiers = before }
        return try body()
    }

    // MARK: Pointer

    private static var pointer: (window: ObjectIdentifier, at: CGPoint)?

    /// Where the pointer is, in `window`'s coordinates. Automation: its
    /// in-app pointer, nil while that's off this window (or was never put
    /// anywhere).
    static func pointer(in window: NSWindow) -> CGPoint? {
        guard simulated else { return window.mouseLocationOutsideOfEventStream }
        guard let pointer, pointer.window == ObjectIdentifier(window) else { return nil }
        return pointer.at
    }

    /// Automation: the in-app pointer is at `point` in `window` (window
    /// coordinates); nil takes it off every window.
    static func movePointer(to point: CGPoint?, in window: NSWindow) {
        pointer = point.map { (ObjectIdentifier(window), $0) }
    }

    // MARK: Cursor

    private static var lastCursor: NSCursor = .arrow

    /// Shows `cursor`. Automation records it and leaves the screen alone:
    /// a background run has no business changing the user's cursor.
    static func setCursor(_ cursor: NSCursor) {
        lastCursor = cursor
        if !simulated { cursor.set() }
    }

    /// The cursor showing. Automation: the one the app set last
    /// (`setCursor`), not whatever AppKit or the window server put up for
    /// the real pointer.
    static var cursor: NSCursor {
        simulated ? lastCursor : NSCursor.current
    }

    // MARK: Events

    /// The key event being handled now: the one automation delivers by hand
    /// (`KeyDelivery`), else the one AppKit is dispatching.
    static var keyEvent: NSEvent? {
        KeyDelivery.delivering ?? NSApp.currentEvent
    }
}
