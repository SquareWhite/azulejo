import AppKit

// Hand-written helpers on top of the generated Design tokens (Design.swift):
// mappings from app types to tokens. Values themselves live in
// shared/design/tokens.json.

extension Design.Icon {
    /// SF Symbol for a tile kind (tile headers, quick open rows).
    static func `for`(_ kind: TileKind) -> String {
        switch kind {
        case .terminal: terminal
        case .browser: browser
        case .preview: preview
        case .files: files
        }
    }
}

extension Design.Color {
    /// Placeholder body for a tile kind whose view doesn't exist yet.
    static func placeholder(_ kind: TileKind) -> NSColor {
        switch kind {
        case .terminal: kindTerminal
        case .browser: kindBrowser
        case .preview: kindPreview
        case .files: kindFiles
        }
    }
}
