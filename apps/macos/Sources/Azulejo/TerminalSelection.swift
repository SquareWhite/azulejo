import CoreGraphics
import Foundation

/// A place in one pane's captured rows: a row and a UTF-16 offset in it.
struct TerminalTextPos: Comparable, Hashable {
    let row: Int
    let offset: Int
    static func < (a: Self, b: Self) -> Bool { a.row != b.row ? a.row < b.row : a.offset < b.offset }
}

/// `start` up to `end` (exclusive), across rows.
struct TerminalTextRange: Hashable {
    var start: TerminalTextPos
    var end: TerminalTextPos

    func contains(_ other: Self) -> Bool { start <= other.start && other.end <= end }
    func union(_ other: Self) -> Self { Self(start: min(start, other.start), end: max(end, other.end)) }
}

/// ⌘S smart select's shapes as the overlay holds them. The logic (labels,
/// paragraphs, grow and shrink, arrows, range text, callout placement)
/// lives in the Rust core; `TerminalHints` calls it.
enum TerminalSelection {
    /// How far a selection reaches: a recognised item, a text paragraph, its
    /// line (soft wraps joined), its block of non-blank rows or the pane's
    /// whole visible text. Arrow keys move at the selection's level. Raw
    /// values are the core's.
    enum Level: UInt8 {
        case item, text, line, block, screen
        var name: String { "\(self)" }
    }

    /// One step of a selection: a range, its level and, for an item, a
    /// paragraph or a line, its index in the pane's items.
    struct Unit: Equatable {
        var range: TerminalTextRange
        var level: Level
        var entry: Int?
    }

    /// Raw values are the core's.
    enum Direction: UInt8 { case left, right, up, down }

    /// The key that picks a kind's category in the ⌘S panel (the core's
    /// `Kind::letter`). A paragraph has none: it has no label.
    static func letter(_ kind: TerminalHintItem.Kind) -> Character? {
        switch kind {
        case .url: "u"
        case .path: "p"
        case .ip: "i"
        case .hash: "h"
        case .branch: "b"
        case .domain: "d"
        case .command: "c"
        case .line: "l"
        case .text: nil
        }
    }

    /// The kind a category key picks.
    static func kind(forLetter letter: String) -> TerminalHintItem.Kind? {
        kindOrder.first { self.letter($0.kind).map(String.init) == letter }?.kind
    }

    /// The categories in panel order: each kind's name for one item and
    /// for several.
    static let kindOrder: [(kind: TerminalHintItem.Kind, one: String, many: String)] = [
        (.url, "URL", "URLs"), (.path, "path", "paths"), (.command, "command", "commands"),
        (.line, "line", "lines"), (.hash, "hash or ID", "hashes & IDs"), (.ip, "IP address", "IP addresses"),
        (.domain, "domain", "domains"), (.branch, "branch", "branches"),
    ]

    /// Whether an item's label shows while choosing in `category` (none
    /// yet: the panel lists the categories and no label shows) with
    /// `typed` typed so far (the core's `select::shown`): it's of that kind
    /// and its label starts with `typed`.
    static func shown(_ kind: TerminalHintItem.Kind, label: String, category: TerminalHintItem.Kind?,
                      typed: String) -> Bool {
        category == kind && !label.isEmpty && label.hasPrefix(typed)
    }

    static func extent(_ item: TerminalHintItem) -> TerminalTextRange {
        let first = item.spans.first!, last = item.spans.last!
        return TerminalTextRange(start: TerminalTextPos(row: first.row, offset: first.range.location),
                                 end: TerminalTextPos(row: last.row, offset: NSMaxRange(last.range)))
    }

    // MARK: Callouts

    /// Which side of the item's first cell a callout sits on; its tail
    /// points back at the cell.
    enum CalloutSide: String {
        /// Over the row below, tail up at the cell's bottom middle.
        case below
        /// Over the row above, tail down at the cell's top middle.
        case above
        /// On the item's own row, in the blank margin before it, tail
        /// pointing right at the cell's left middle.
        case left
        /// Over the item's own first cells, no tail: the last resort.
        case inline
    }

    /// A placed label, in the overlay's flipped coordinates (y grows down).
    struct Callout: Equatable {
        var frame: CGRect
        /// Where the tail points (see `CalloutSide`).
        var tip: CGPoint
        /// Where the tail leaves the pill.
        var base: CGPoint = .zero
        var side: CalloutSide
        /// Slid sideways off the cell's middle (below or above).
        var nudged = false
        /// How many layers of what to keep off it had to give up (0 blank
        /// space, 1 plain text, 2 a paragraph, 3 another item).
        var relaxed = 0
        /// Overlaps another callout or another item's first cell anyway.
        var clash = false
    }

    /// An item as callout placement sees it: its first cell, the rects of
    /// its text, whether it's a paragraph, and its pill's size.
    struct CalloutItem {
        let anchor: CGRect
        let rects: [CGRect]
        let prose: Bool
        let size: CGSize
    }
}
