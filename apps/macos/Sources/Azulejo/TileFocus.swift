import AppKit

/// The one rule for when a tile may take first responder.
///
/// The container tells the focused tile it's focused after every refresh
/// (agent state, headers, herdr events), and a terminal takes the keyboard
/// again when its attach finishes. Either would pull the keyboard out of a
/// field being typed in outside the tiles — the ⌘K menu's search field, and
/// later the toolbar search or an inline rename — so the rest of the query
/// would land in the terminal. While such a field holds the keyboard, tiles
/// leave it alone; closing the menu hands it back to the focused tile.
///
/// A real focus change (a different tile is now focused) still moves the
/// keyboard: the container sets `focusMoving` while it applies one.
///
/// A tile's open find bar is part of the tile: when the tile is focused again
/// after something outside it had the keyboard (the ⌘K menu, another tab,
/// the sidebar), the bar gets the keyboard back if it had it then
/// (`returnToFindBar`, from `TileFrame.setFocused`, ahead of the tile's own
/// `setFocused`, which then leaves the bar alone).
@MainActor
enum TileFocus {
    /// Set by TileContainerView while it applies a change of focused tile.
    static var focusMoving = false

    static func canTake(in window: NSWindow) -> Bool {
        focusMoving || !typingOutsideTiles(in: window)
    }

    /// Hands the keyboard back to `tile`'s open find bar if the bar had it
    /// when the keyboard last left the tile.
    static func returnToFindBar(of tile: any TileView) {
        guard let bar = (tile as? Findable)?.findBar, bar.isOpen, bar.keepsKeyboard, !bar.holdsKeyboard,
              let window = bar.window, canTake(in: window)
        else { return }
        bar.takeKeyboard()
    }

    /// A visible text field (or its field editor) outside every tile container
    /// has the keyboard, or a view inside something that keeps the keyboard
    /// (the sidebar's Files tree). Fields inside tiles — the browser's
    /// address bar, the files filter, a find bar — don't count: the focused
    /// tile manages those itself (its `setFocused` leaves them alone).
    static func typingOutsideTiles(in window: NSWindow) -> Bool {
        guard let responder = window.firstResponder as? NSView else { return false }
        var view = responder
        // A text field's field editor stands in for the field.
        if let editor = responder as? NSTextView, editor.isFieldEditor, let field = editor.delegate as? NSView {
            view = field
        }
        // A closing menu lets go of the keyboard (or is hidden) before it
        // hands it back.
        if view.isHiddenOrHasHiddenAncestor { return false }
        var holds = view is NSTextView || view is NSTextField
        var current: NSView? = view
        while let v = current {
            if v is TileContainerView { return false }
            if v is KeepsKeyboard { holds = true }
            current = v.superview
        }
        return holds
    }
}

/// A view outside the tiles whose descendants keep the keyboard until the
/// user moves on (the sidebar): tiles don't take it back on a refresh.
@MainActor
protocol KeepsKeyboard: NSView {}
