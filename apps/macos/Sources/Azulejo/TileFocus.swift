import AppKit

/// When a tile may take first responder. The container tells the focused tile
/// so after every refresh (agent state, headers, herdr events), and a tile
/// that grabbed first responder each time would pull it out of the ⌘K menu's
/// field mid-typing. While the menu is open it keeps first responder; the
/// menu hands it back to the focused tile when it closes.
@MainActor
enum TileFocus {
    static func canTake(in window: NSWindow) -> Bool {
        guard let responder = window.firstResponder else { return true }
        var view = responder as? NSView
        // A text field's field editor stands in for the field.
        if let editor = responder as? NSTextView, editor.isFieldEditor, let field = editor.delegate as? NSView {
            view = field
        }
        while let v = view {
            if let palette = v as? PaletteView { return !palette.isOpen }
            view = v.superview
        }
        return true
    }
}
