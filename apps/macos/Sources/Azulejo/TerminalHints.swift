import AzulejoCore
import Foundation

/// Ranges are UTF-16 offsets in the captured rows, not terminal columns.
/// The surface maps them back through Ghostty, which owns Unicode cell widths.
struct TerminalHintSpan: Equatable {
    let row: Int
    let range: NSRange
}

struct TerminalHintItem: Equatable {
    enum Kind: String { case url, path, branch, command, ip, domain, uuid, hash, text }
    let kind: Kind
    let value: String
    let spans: [TerminalHintSpan]
    let openURL: URL?
}

/// ⌘S smart select's detection and selection logic lives in the Rust core
/// (core/crates/hints, docs/adr/0016): this is the glue. `Pane` sends one
/// pane's screen once per ⌘S and keeps the core's handle for the selection
/// keys.
enum TerminalHints {
    /// `[hints] row_classifier`: the row classifier refines commands.
    nonisolated(unsafe) static var rowClassifier = false

    /// `[hints] run_target`: `reuse` (an idle shell, else a new pane) or `new`.
    nonisolated(unsafe) static var runTarget = "reuse"

    /// Reads `azj_core_hints`' JSON. Called at launch and on `config.reloaded`.
    static func apply(_ settings: [String: Any]) {
        rowClassifier = settings["row_classifier"] as? Bool == true
        runTarget = settings["run_target"] as? String ?? "reuse"
    }

    /// One pane's items (paragraphs included, in reading order), each row's
    /// text extent and runs of non-blank characters, and the core's handle.
    final class Pane {
        private let handle: OpaquePointer
        let items: [TerminalHintItem]
        /// Each row without its indent and trailing blanks; nil when blank.
        let extents: [(start: Int, end: Int)?]
        /// Each row's runs of non-blank characters.
        let runs: [[NSRange]]

        /// `joins`: rows whose last cell soft-wraps into the next; `vt`: the
        /// terminal's styled snapshot of the same rows (its colours mark
        /// inline commands). Nil if the core refuses the request.
        init?(rows: [String], joins: Set<Int>, columns: Int, cwd: String?, local: Bool, branch: String?, vt: String?) {
            var request: [String: Any] = ["rows": rows, "joins": joins.sorted(), "columns": columns, "local": local,
                                          "row_classifier": TerminalHints.rowClassifier]
            if let cwd { request["cwd"] = cwd }
            if let branch { request["branch"] = branch }
            if let vt { request["vt"] = vt }
            var pane: OpaquePointer?
            guard let data = try? JSONSerialization.data(withJSONObject: request),
                  let reply = azj_hints_detect(String(decoding: data, as: UTF8.self), &pane) else { return nil }
            defer { azj_string_free(reply) }
            guard let pane else { return nil }
            handle = pane
            guard let object = try? JSONSerialization.jsonObject(with: Data(bytesNoCopy: reply, count: strlen(reply),
                                                                             deallocator: .none)) as? [String: Any],
                  object["ok"] as? Bool == true, let items = object["items"] as? [[String: Any]] else {
                azj_hints_pane_free(pane)
                return nil
            }
            self.items = items.compactMap { item in
                guard let kind = (item["kind"] as? String).flatMap(TerminalHintItem.Kind.init(rawValue:)),
                      let value = item["value"] as? String, let spans = item["spans"] as? [[Int]] else { return nil }
                return TerminalHintItem(
                    kind: kind, value: value,
                    spans: spans.map { TerminalHintSpan(row: $0[0], range: NSRange(location: $0[1], length: $0[2])) },
                    openURL: (item["open"] as? String).flatMap(URL.init(string:)))
            }
            extents = (object["extents"] as? [Any] ?? []).map { ($0 as? [Int]).map { (start: $0[0], end: $0[1]) } }
            runs = (object["runs"] as? [[Int]] ?? []).map { flat in
                stride(from: 0, to: flat.count - 1, by: 2).map { NSRange(location: flat[$0], length: flat[$0 + 1]) }
            }
        }

        deinit { azj_hints_pane_free(handle) }

        /// Keeps only these items (ascending indices into `items`), the ones
        /// the overlay could place. Units' `entry` then counts the kept items.
        func keep(_ indices: [Int]) {
            let list = indices.map(UInt32.init)
            list.withUnsafeBufferPointer { azj_hints_pane_keep(handle, $0.baseAddress, UInt($0.count)) }
        }

        /// ⌥↑: the next thing out that holds `range`.
        func grow(_ range: TerminalTextRange) -> TerminalSelection.Unit? {
            var unit = AzjHintUnit()
            return azj_hints_pane_grow(handle, range.c, &unit) ? TerminalSelection.Unit(unit) : nil
        }

        /// ⌥↓ with nothing to go back to: the first item inside `range`.
        func shrink(_ range: TerminalTextRange, from level: TerminalSelection.Level) -> TerminalSelection.Unit? {
            var unit = AzjHintUnit()
            return azj_hints_pane_shrink(handle, range.c, level.rawValue, &unit) ? TerminalSelection.Unit(unit) : nil
        }

        /// An arrow key from `from`, at its level.
        func neighbour(_ from: TerminalSelection.Unit, _ direction: TerminalSelection.Direction) -> TerminalSelection.Unit? {
            var unit = AzjHintUnit()
            let start = AzjHintUnit(range: from.range.c, level: from.level.rawValue, entry: Int32(from.entry ?? -1))
            return azj_hints_pane_neighbour(handle, start, direction.rawValue, &unit) ? TerminalSelection.Unit(unit) : nil
        }

        /// An arrow key while choosing: the item (an index into the kept
        /// items) the focus ring moves to among `shown`, from `focus`, or
        /// with none from the terminal `cursor` (nil: below the last row).
        func focus(among shown: [Int], from focus: Int?, cursor: TerminalTextPos?,
                   _ direction: TerminalSelection.Direction) -> Int? {
            var index: UInt32 = 0
            let list = shown.map(UInt32.init)
            let found = list.withUnsafeBufferPointer {
                azj_hints_pane_focus(handle, $0.baseAddress, UInt($0.count), Int32(focus ?? -1),
                                     Int32(cursor?.row ?? -1), UInt32(cursor?.offset ?? 0), direction.rawValue, &index)
            }
            return found ? Int(index) : nil
        }

        /// The item among `shown` nearest item `to`: where the focus ring
        /// goes when typing filters out the focused item.
        func nearest(among shown: [Int], to: Int) -> Int? {
            var index: UInt32 = 0
            let list = shown.map(UInt32.init)
            let found = list.withUnsafeBufferPointer {
                azj_hints_pane_nearest(handle, $0.baseAddress, UInt($0.count), UInt32(to), &index)
            }
            return found ? Int(index) : nil
        }

        /// Whether arrow keys have anywhere to go at `level`.
        func hasTargets(_ level: TerminalSelection.Level) -> Bool {
            azj_hints_pane_targets(handle, level.rawValue) > 0
        }

        /// The text a terminal selection of `range` would copy.
        func text(_ range: TerminalTextRange) -> String {
            guard let pointer = azj_hints_pane_text(handle, range.c) else { return "" }
            defer { azj_string_free(pointer) }
            return String(cString: pointer)
        }

        /// Labels for every kept item of these panes, in order; "" for an
        /// item repeated in its pane (all but the newest copy).
        static func labels(_ panes: [Pane]) -> [String] {
            let handles: [OpaquePointer?] = panes.map(\.handle)
            guard let reply = handles.withUnsafeBufferPointer({ azj_hints_labels($0.baseAddress, UInt($0.count)) })
            else { return [] }
            defer { azj_string_free(reply) }
            let object = try? JSONSerialization.jsonObject(with: Data(String(cString: reply).utf8)) as? [String: Any]
            return object?["labels"] as? [String] ?? []
        }
    }

    /// The line Run types into a new pane's login shell (a multi-line
    /// command goes through `/bin/sh`, base64 encoded).
    static func runLine(_ value: String) -> String {
        guard let pointer = azj_hints_run_line(value) else { return value }
        defer { azj_string_free(pointer) }
        return String(cString: pointer)
    }

    /// Where Run sends a command and the line it types there
    /// (`azj_hints_run_plan`; the request's fields are documented there).
    /// Nil if the core refuses the request.
    static func runPlan(_ request: [String: Any]) -> (target: String, tile: String?, line: String)? {
        guard let data = try? JSONSerialization.data(withJSONObject: request),
              let reply = azj_hints_run_plan(String(decoding: data, as: UTF8.self)) else { return nil }
        defer { azj_string_free(reply) }
        guard let object = try? JSONSerialization.jsonObject(with: Data(String(cString: reply).utf8)) as? [String: Any],
              object["ok"] as? Bool == true, let target = object["target"] as? String,
              let line = object["line"] as? String else { return nil }
        return (target, object["tile"] as? String, line)
    }

    /// Homerow-style callouts for one pane's labelled items
    /// (`azj_hints_place_callouts`): `text` is every run of non-blank cells,
    /// `bounds` where pills may go. Empty if the core refuses.
    static func placeCallouts(_ items: [TerminalSelection.CalloutItem], text: [CGRect], bounds: CGRect, tail: CGFloat,
                              spacing: CGFloat, inset: CGFloat, drift: CGFloat) -> [TerminalSelection.Callout] {
        func rect(_ r: CGRect) -> [CGFloat] { [r.minX, r.minY, r.width, r.height] }
        let request: [String: Any] = [
            "items": items.map { ["anchor": rect($0.anchor), "rects": $0.rects.map(rect), "prose": $0.prose,
                                  "size": [$0.size.width, $0.size.height]] },
            "text": text.map(rect), "bounds": rect(bounds), "tail": tail, "spacing": spacing, "inset": inset, "drift": drift,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: request),
              let reply = azj_hints_place_callouts(String(decoding: data, as: UTF8.self)) else { return [] }
        defer { azj_string_free(reply) }
        guard let object = try? JSONSerialization.jsonObject(with: Data(String(cString: reply).utf8)) as? [String: Any],
              let callouts = object["callouts"] as? [[String: Any]] else { return [] }
        func cgRect(_ v: Any?) -> CGRect {
            guard let v = v as? [CGFloat], v.count == 4 else { return .zero }
            return CGRect(x: v[0], y: v[1], width: v[2], height: v[3])
        }
        func point(_ v: Any?) -> CGPoint {
            guard let v = v as? [CGFloat], v.count == 2 else { return .zero }
            return CGPoint(x: v[0], y: v[1])
        }
        return callouts.map { c in
            TerminalSelection.Callout(
                frame: cgRect(c["frame"]), tip: point(c["tip"]), base: point(c["base"]),
                side: (c["side"] as? String).flatMap(TerminalSelection.CalloutSide.init(rawValue:)) ?? .below,
                nudged: c["nudged"] as? Bool ?? false, relaxed: c["relaxed"] as? Int ?? 0, clash: c["clash"] as? Bool ?? false)
        }
    }
}

private extension TerminalTextRange {
    var c: AzjHintRange {
        AzjHintRange(start_row: UInt32(start.row), start_offset: UInt32(start.offset),
                     end_row: UInt32(end.row), end_offset: UInt32(end.offset))
    }
}

private extension TerminalSelection.Unit {
    init?(_ unit: AzjHintUnit) {
        guard let level = TerminalSelection.Level(rawValue: unit.level) else { return nil }
        self.init(range: TerminalTextRange(
                      start: TerminalTextPos(row: Int(unit.range.start_row), offset: Int(unit.range.start_offset)),
                      end: TerminalTextPos(row: Int(unit.range.end_row), offset: Int(unit.range.end_offset))),
                  level: level, entry: unit.entry >= 0 ? Int(unit.entry) : nil)
    }
}
