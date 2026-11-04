import AppKit

/// A view with keymap actions of its own: a tile kind's keys (browser chrome,
/// the files list, preview, terminal) or a field's (find bars, the files
/// filter). The key router finds the nearest one around the first responder,
/// resolves the chord in its context, and hands it the actions that only
/// work there (`browser_back`, `files_down`, `find_next`…). Nothing here
/// matches chords itself, so every one of those keys comes from the keymap.
@MainActor
protocol KeyScope: NSView {
    /// The keymap context while this view has the keyboard: `"browser"`,
    /// `"files"`, `"find_bar"`… (`[keys.<platform>.<context>]`).
    var keyScope: String { get }
    /// Runs a context-only keymap action. `event` is the keyDown when it came
    /// from the keyboard (nil from menus and quick open). False when it doesn't
    /// apply right now: the chord then goes on as typed.
    func perform(keyAction name: String, event: NSEvent?) -> Bool
}

@MainActor
enum KeyScopes {
    /// The scope around `window`'s first responder; a field editor counts as
    /// the field it edits.
    static func around(_ window: NSWindow?) -> (any KeyScope)? {
        guard let responder = window?.firstResponder else { return nil }
        var view = responder as? NSView
        if let editor = responder as? NSTextView, editor.isFieldEditor, let field = editor.delegate as? NSView {
            view = field
        }
        while let v = view {
            if let scope = v as? any KeyScope { return scope }
            view = v.superview
        }
        return nil
    }

    /// The focused tile's own scope, for actions run from menus and quick
    /// open, or when the keyboard sits outside any scope.
    static func focusedTile(in controller: WindowController) -> (any KeyScope)? {
        (controller.tiles.focusedView as? TileFrame)?.content as? any KeyScope
    }
}
