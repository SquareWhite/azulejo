import Foundation

/// fzf-style matching for the palette and picker. Each space-separated term
/// must appear in order (as a subsequence) in the text; matches score higher
/// when they're consecutive, start a word or start the text. Case-insensitive.
enum PaletteFuzzy {
    struct Match: Equatable {
        var score: Int
        /// Matched character offsets in the text (for highlighting).
        var indices: [Int]
    }

    /// Nil when some term doesn't match. An empty query matches everything
    /// with score 0.
    static func match(_ query: String, in text: String) -> Match? {
        let terms = query.lowercased().split(separator: " ").map { Array($0) }
        guard !terms.isEmpty else { return Match(score: 0, indices: []) }
        let chars = Array(text.lowercased())
        let original = Array(text)
        var total = 0
        var indices: [Int] = []
        for term in terms {
            guard let best = bestAlignment(term, in: chars, original: original) else { return nil }
            total += best.score
            indices.append(contentsOf: best.indices)
        }
        return Match(score: total, indices: Array(Set(indices)).sorted())
    }

    private static let scoreMatch = 16
    private static let bonusConsecutive = 12
    private static let bonusWordStart = 10
    private static let bonusFirstChar = 8
    private static let penaltyGap = 2
    private static let maxGapPenalty = 12

    /// Tries every start position for the term's first character and keeps
    /// the best-scoring greedy alignment from there.
    private static func bestAlignment(_ term: [Character], in chars: [Character], original: [Character]) -> Match? {
        guard let first = term.first, chars.count >= term.count else { return nil }
        var best: Match?
        for start in chars.indices where chars[start] == first {
            var indices = [start]
            var pos = start + 1
            var ok = true
            for c in term.dropFirst() {
                // Prefer a word-start occurrence ahead of a plain one when it
                // isn't much further away (so "sr" picks "Split Right").
                guard let plain = chars[pos...].firstIndex(of: c) else { ok = false; break }
                var pick = plain
                if !isWordStart(plain, original), plain > pos,
                   let ws = chars[pos...].indices.first(where: { chars[$0] == c && isWordStart($0, original) }),
                   ws - plain < 12 {
                    pick = ws
                }
                indices.append(pick)
                pos = pick + 1
            }
            guard ok else { continue }
            let s = score(indices, original)
            if best == nil || s > best!.score { best = Match(score: s, indices: indices) }
        }
        return best
    }

    private static func score(_ indices: [Int], _ original: [Character]) -> Int {
        var s = 0
        for (n, i) in indices.enumerated() {
            s += scoreMatch
            if i == 0 { s += bonusFirstChar }
            if isWordStart(i, original) { s += bonusWordStart }
            if n > 0 {
                let gap = i - indices[n - 1] - 1
                if gap == 0 { s += bonusConsecutive } else { s -= min(maxGapPenalty, gap * penaltyGap) }
            }
        }
        return s
    }

    private static func isWordStart(_ i: Int, _ text: [Character]) -> Bool {
        guard i > 0 else { return true }
        let prev = text[i - 1], cur = text[i]
        if " _-./:·—".contains(prev) { return true }
        return prev.isLowercase && cur.isUppercase
    }

    /// Pretty chord for display: `cmd+shift+p` → `⌘⇧P`, `ctrl+alt+h` → `⌃⌥H`.
    static func prettyChord(_ chord: String) -> String {
        chord.split(separator: " ").map { part -> String in
            var mods = ""
            var key = ""
            for bit in part.split(separator: "+") {
                switch bit {
                case "cmd", "super": mods += "⌘"
                case "shift": mods += "⇧"
                case "alt", "opt": mods += "⌥"
                case "ctrl": mods += "⌃"
                default: key = String(bit)
                }
            }
            let names: [String: String] = [
                "enter": "↩", "escape": "⎋", "tab": "⇥", "space": "Space", "backspace": "⌫",
                "left": "←", "right": "→", "up": "↑", "down": "↓", "minus": "-", "equal": "=",
                "bracketleft": "[", "bracketright": "]", "backslash": "\\", "slash": "/",
                "comma": ",", "period": ".", "semicolon": ";", "quote": "'", "grave": "`",
            ]
            return mods + (names[key] ?? key.uppercased())
        }.joined(separator: " ")
    }
}
