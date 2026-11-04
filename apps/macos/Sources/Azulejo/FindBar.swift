import AppKit
import WebKit

/// A tile that answers ⌘F (the `find` action).
@MainActor
protocol Findable: AnyObject {
    /// Opens the tile's find bar (or its own filter) with the keyboard in it.
    /// Returns a message to flash when the tile has nothing to search.
    func beginFind() -> String?
    /// The tile's find bar, if it has one (files tiles filter instead).
    var findBar: FindBar? { get }
}

extension Findable {
    var findBar: FindBar? { nil }
}

/// The find bar a tile shows under its header (⌘F): a field, "n of m",
/// previous, next and close. The tile's finder does the searching; the bar
/// only reports what's typed and pressed.
///
/// Keys: while the field has the keyboard, the key router hands it enter
/// (next), shift+enter (previous), ⌘G / ⌘⇧G and escape (close) before the
/// keymap sees them, and resolves other chords with no tile context, so ⌘A
/// selects the query rather than toggling the sidebar. Everything else is
/// typing. The field sits inside the tile, so the tile itself keeps refreshes
/// (TileFocus) from taking the keyboard out of it; see `holdsKeyboard`.
/// When something outside the tile takes the keyboard (the ⌘K menu, another
/// tab, the sidebar) the bar remembers it had it (`keepsKeyboard`), and the
/// tile gets it back into the bar when it's focused again (TileFocus).
@MainActor
final class FindBar: NSView {
    static let height = Design.Space.browserChromeHeight

    /// The query changed (every edit; the finder debounces).
    var onChange: ((String) -> Void)?
    /// Enter (forward) or shift+enter (backward).
    var onStep: ((_ forward: Bool) -> Void)?
    /// Escape or the close button.
    var onClose: (() -> Void)?
    /// The field took the keyboard (a click into it, ⌘F).
    var onFocus: (() -> Void)?

    let field = FindField()
    private let pill = NSView()
    private let icon = NSImageView()
    private let count = NSTextField(labelWithString: "")
    private let previous = FindBar.button("chevron.up", "Previous (⇧↩)")
    private let next = FindBar.button("chevron.down", "Next (↩)")
    private let close = FindBar.button("xmark", "Close (esc)")
    private let hairline = NSView()

    init() {
        super.init(frame: .zero)
        wantsLayer = true
        isHidden = true

        pill.wantsLayer = true
        pill.layer?.cornerRadius = Design.Radius.field
        icon.image = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: "Find")
        icon.symbolConfiguration = .init(pointSize: 10, weight: .regular)
        icon.contentTintColor = Design.Color.textMuted

        field.placeholderString = "Find"
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = Design.Font.field
        field.textColor = Design.Color.textPrimary
        field.usesSingleLineMode = true
        field.lineBreakMode = .byClipping
        field.cell?.isScrollable = true
        field.delegate = self
        field.onFocus = { [weak self] in self?.onFocus?() }

        count.font = Design.Font.detail
        count.textColor = Design.Color.textMuted
        count.alignment = .right
        count.lineBreakMode = .byClipping

        previous.target = self; previous.action = #selector(previousClicked)
        next.target = self; next.action = #selector(nextClicked)
        close.target = self; close.action = #selector(closeClicked)

        hairline.wantsLayer = true
        [icon, field, count].forEach(pill.addSubview)
        [pill, previous, next, close, hairline].forEach(addSubview)
        updateColors()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    var isOpen: Bool { !isHidden }
    var query: String { field.stringValue }
    private(set) var status = ""

    /// The field (its field editor) has the keyboard.
    var holdsKeyboard: Bool {
        guard let window, let editor = field.currentEditor() else { return false }
        return window.firstResponder === editor
    }

    /// The bar had the keyboard when it last left the tile, and nothing in
    /// the tile (its terminal, page or address bar) has taken it since.
    private(set) var keepsKeyboard = false
    private var responderWatch: NSKeyValueObservation?

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // Off screen (another tab) the bar stops watching and remembers.
        responderWatch = window?.observe(\.firstResponder) { [weak self] _, _ in
            MainActor.assumeIsolated { self?.responderChanged() }
        }
    }

    private func responderChanged() {
        guard isOpen, let window else { return }
        guard var view = window.firstResponder as? NSView else { return }
        if let editor = view as? NSTextView, editor.isFieldEditor, let field = editor.delegate as? NSView {
            view = field
        }
        if view.isDescendant(of: self) {
            keepsKeyboard = true
        } else if let tile = enclosingTile, view.isDescendant(of: tile) {
            // The user moved on inside the tile: refocusing leaves that be.
            keepsKeyboard = false
        }
    }

    /// The tile card holding the bar.
    private var enclosingTile: NSView? {
        var view = superview
        while let v = view {
            if v is TileFrame { return v }
            view = v.superview
        }
        return nil
    }

    /// Puts the keyboard back in the field, caret at the end of the query.
    func takeKeyboard() {
        guard let window, isOpen else { return }
        if !holdsKeyboard { window.makeFirstResponder(field) }
        let end = (field.stringValue as NSString).length
        field.currentEditor()?.selectedRange = NSRange(location: end, length: 0)
    }

    /// Shows the bar and puts the keyboard in the field, selecting what's
    /// there so typing replaces it.
    func open() {
        isHidden = false
        needsLayout = true
        guard let window else { return }
        if !holdsKeyboard { window.makeFirstResponder(field) }
        field.currentEditor()?.selectAll(nil)
    }

    /// Hides the bar; the query stays for the next ⌘F.
    func hide() {
        isHidden = true
        keepsKeyboard = false
    }

    /// "3 of 12", "No matches", or "" (nothing searched yet); `note` is
    /// the count's tooltip.
    func setStatus(_ text: String, found: Bool, note: String? = nil) {
        status = text
        count.stringValue = text
        count.toolTip = note
        count.textColor = found || text.isEmpty ? Design.Color.textMuted : Design.Color.error
        previous.isEnabled = found
        next.isEnabled = found
        needsLayout = true
    }

    /// Bar keys while the field has the keyboard; false for anything else.
    func handle(chord: String) -> Bool {
        switch chord {
        case "enter", "cmd+g": onStep?(true)
        case "shift+enter", "cmd+shift+g": onStep?(false)
        case "escape": onClose?()
        default: return false
        }
        return true
    }

    /// The bar whose field has the keyboard in `window`, if any.
    static func focused(in window: NSWindow?) -> FindBar? {
        guard let editor = window?.firstResponder as? NSTextView, editor.isFieldEditor,
              let field = (editor.delegate as? NSView) as? FindField else { return nil }
        var view = field.superview
        while let v = view {
            if let bar = v as? FindBar { return bar }
            view = v.superview
        }
        return nil
    }

    // MARK: Layout

    override func layout() {
        super.layout()
        let w = bounds.width, h = bounds.height
        let button: CGFloat = 22, gap: CGFloat = 2, margin: CGFloat = 10
        let y = (h - button) / 2
        close.frame = NSRect(x: w - margin - button, y: y, width: button, height: button)
        next.frame = NSRect(x: close.frame.minX - gap - button, y: y, width: button, height: button)
        previous.frame = NSRect(x: next.frame.minX - gap - button, y: y, width: button, height: button)
        let fieldH = Design.Space.browserFieldHeight
        pill.frame = NSRect(x: margin, y: (h - fieldH) / 2,
                            width: max(40, previous.frame.minX - 8 - margin), height: fieldH)
        hairline.frame = NSRect(x: 0, y: h - Design.Border.hairline, width: w, height: Design.Border.hairline)

        let pw = pill.bounds.width, ph = pill.bounds.height
        let iconSide: CGFloat = 12, inset: CGFloat = 8, iconGap: CGFloat = 6
        icon.frame = NSRect(x: inset, y: (ph - iconSide) / 2, width: iconSide, height: iconSide)
        let countW = count.stringValue.isEmpty ? 0
            : ceil((count.stringValue as NSString).size(withAttributes: [.font: Design.Font.detail]).width) + 4
        let countH = count.intrinsicContentSize.height
        count.frame = NSRect(x: pw - inset - countW, y: (ph - countH) / 2, width: countW, height: countH)
        let textH = field.intrinsicContentSize.height
        let fieldX = inset + iconSide + iconGap
        field.frame = NSRect(x: fieldX, y: (ph - textH) / 2,
                             width: max(0, count.frame.minX - 6 - fieldX), height: textH)
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        updateColors()
    }

    private func updateColors() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            layer?.backgroundColor = Design.Color.browserChrome.cgColor
            pill.layer?.backgroundColor = Design.Color.browserField.cgColor
            hairline.layer?.backgroundColor = Design.Color.headerHairline.cgColor
        }
    }

    // MARK: Buttons

    @objc private func previousClicked() { onStep?(false) }
    @objc private func nextClicked() { onStep?(true) }
    @objc private func closeClicked() { onClose?() }

    private static func button(_ symbol: String, _ tip: String) -> NSButton {
        let image = NSImage(systemSymbolName: symbol, accessibilityDescription: tip) ?? NSImage()
        let b = NSButton(image: image, target: nil, action: nil)
        b.bezelStyle = .accessoryBarAction
        b.isBordered = false
        b.symbolConfiguration = .init(pointSize: 11, weight: .medium)
        b.contentTintColor = Design.Color.textSecondary
        b.toolTip = tip
        return b
    }
}

extension FindBar: NSTextFieldDelegate {
    func controlTextDidChange(_ notification: Notification) {
        onChange?(field.stringValue)
    }

    /// Enter and escape when the key router didn't see them (a key path
    /// that skips the local monitor).
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.insertNewline(_:)): onStep?(true)
        case #selector(NSResponder.cancelOperation(_:)): onClose?()
        default: return false
        }
        return true
    }
}

/// The find bar's text field: tells the bar when it takes the keyboard.
@MainActor
final class FindField: NSTextField {
    var onFocus: () -> Void = {}

    override func becomeFirstResponder() -> Bool {
        let took = super.becomeFirstResponder()
        if took { onFocus() }
        return took
    }
}

// MARK: - Terminal: herdr's history

/// Finds text in a terminal tile's history for its find bar. History lives
/// in herdr (ADR 0005), so the search is the core's `terminal.search` with
/// `locate`, and a match is shown by scrolling herdr's pane to it
/// (`terminal.scroll`). Matches count from the newest: enter steps back in
/// time, shift+enter forward.
///
/// Highlighting: the attach stream is herdr's re-rendered screen, so the
/// local terminal only ever holds what's in view. libghostty's own search
/// runs over that screen and marks every occurrence in view, precisely;
/// the current match is the one mid-view. It can't single the current one
/// out, and it doesn't know about the rest of the history. The bar covers
/// the top two rows rather than resizing the PTY, so a match in the very
/// first rows of the history (where the view can't scroll further) sits
/// under it.
@MainActor
final class TerminalFinder {
    private let tileID: String
    private let core: () -> Core?
    /// The surface showing the pane, for the highlight.
    private let surface: () -> TerminalSurfaceView?
    let bar = FindBar()

    /// Each match's `terminal.scroll` offset (nil when the core couldn't
    /// place it), newest first.
    private var matches: [UInt64?] = []
    /// herdr returned only part of the history (it reads at most 1000 rows).
    private var truncated = false
    private var current = 0
    /// The view's max offset when the matches were found; a later scroll
    /// reply with a larger one means output arrived since.
    private var searchedMax: UInt64 = 0
    /// Where the view was before the first reveal; escape goes back there.
    private var restoreOffset: UInt64?
    private var pending: DispatchWorkItem?
    /// For automation: the last offset herdr reported after a scroll.
    private(set) var lastOffset: UInt64 = 0
    /// For automation: occurrences libghostty marks in view.
    var highlights: Int { surface()?.searchTotal ?? -1 }

    init(tileID: String, core: @escaping () -> Core?, surface: @escaping () -> TerminalSurfaceView?) {
        self.tileID = tileID
        self.core = core
        self.surface = surface
        bar.onChange = { [weak self] query in self?.queryChanged(query) }
        bar.onStep = { [weak self] forward in self?.step(forward: forward) }
        bar.onClose = { [weak self] in self?.close() }
    }

    var onClosed: (() -> Void)?

    func open() {
        let reopening = bar.isOpen
        bar.open()
        // Reopened with a query: search again (the history has moved on).
        if !reopening, !bar.query.isEmpty { search(bar.query) }
    }

    func close() {
        pending?.cancel()
        bar.hide()
        surface()?.bindingAction("end_search")
        if let offset = restoreOffset {
            scroll(to: offset)
            restoreOffset = nil
        }
        matches = []
        onClosed?()
    }

    private func queryChanged(_ query: String) {
        pending?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.search(query) }
        pending = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.12, execute: work)
    }

    private func search(_ query: String) {
        pending = nil
        surface()?.bindingAction("search:\(query)")
        guard !query.isEmpty else {
            matches = []
            bar.setStatus("", found: false)
            return
        }
        let reply = core()?.execute(["method": "terminal.search",
                                     "params": ["tile": tileID, "query": query, "locate": true]])
        guard case .success(let body)? = reply else {
            matches = []
            if case .failure(let error)? = reply {
                bar.setStatus(error.code == "unavailable" ? "herdr unavailable" : "can't search", found: false)
            }
            return
        }
        let found = body["matches"] as? [[String: Any]] ?? []
        truncated = body["truncated"] as? Bool == true
        matches = found.reversed().map { ($0["offset_from_bottom"] as? NSNumber)?.uint64Value }
        let view = body["scroll"] as? [String: Any]
        searchedMax = (view?["max_offset_from_bottom"] as? NSNumber)?.uint64Value ?? 0
        if restoreOffset == nil {
            restoreOffset = (view?["offset_from_bottom"] as? NSNumber)?.uint64Value
        }
        current = 0
        reveal()
    }

    private func step(forward: Bool) {
        guard !matches.isEmpty else {
            if !bar.query.isEmpty { search(bar.query) }
            return
        }
        current = (current + (forward ? 1 : -1) + matches.count) % matches.count
        reveal()
    }

    private func reveal() {
        let note = truncated ? "herdr returns its last 1000 rows; older output wasn't searched" : nil
        guard !matches.isEmpty else {
            bar.setStatus("No matches", found: false, note: note)
            return
        }
        // "+": there may be more in the history herdr didn't return.
        bar.setStatus("\(current + 1) of \(matches.count)\(truncated ? "+" : "")", found: true, note: note)
        guard let offset = matches[current] else { return }
        // Output since the search pushed the rows up: follow them. (Once
        // herdr's scrollback is full the max stops growing and this can't
        // tell; typing in the bar searches afresh.)
        if let state = scroll(to: offset), state.max > searchedMax {
            let drift = state.max - searchedMax
            searchedMax = state.max
            matches = matches.map { $0.map { $0 + drift } }
            scroll(to: offset + drift)
        }
    }

    @discardableResult
    private func scroll(to offset: UInt64) -> (offset: UInt64, max: UInt64)? {
        guard case .success(let body)? = core()?.execute([
            "method": "terminal.scroll", "params": ["tile": tileID, "offset_from_bottom": offset],
        ]) else { return nil }
        let now = (body["offset_from_bottom"] as? NSNumber)?.uint64Value ?? offset
        lastOffset = now
        return (now, (body["max_offset_from_bottom"] as? NSNumber)?.uint64Value ?? 0)
    }

    func automationState() -> [String: String] {
        [
            "tile.find": bar.isOpen ? "open" : "closed",
            "tile.find.query": bar.query.isEmpty ? "-" : bar.query,
            "tile.find.status": bar.status.isEmpty ? "-" : bar.status,
            "tile.find.scroll": "\(lastOffset)",
            "tile.find.revealed": lastOffset > 0 ? "yes" : "no",
            "tile.find.highlights": "\(highlights)",
        ]
    }
}

// MARK: - Browser and preview: WebKit's find

/// Finds text in a web view for its tile's find bar, with
/// `WKWebView.find(_:configuration:)`: WebKit selects the match and scrolls
/// to it. WebKit doesn't count matches, so "m" comes from the page's
/// visible text (`innerText`), counted the same case-insensitive way; "n"
/// follows the steps from the first match.
@MainActor
final class WebFinder {
    private let webView: () -> WKWebView?
    let bar = FindBar()
    private var total = 0
    private var current = 0
    private var pending: DispatchWorkItem?
    /// Bumped per search, so a late answer for an old query is dropped.
    private var generation = 0

    init(webView: @escaping () -> WKWebView?) {
        self.webView = webView
        bar.onChange = { [weak self] query in self?.queryChanged(query) }
        bar.onStep = { [weak self] forward in self?.step(forward: forward) }
        bar.onClose = { [weak self] in self?.close() }
    }

    var onClosed: (() -> Void)?

    func open() {
        let reopening = bar.isOpen
        bar.open()
        if !reopening, !bar.query.isEmpty { search(bar.query) }
    }

    func close() {
        pending?.cancel()
        generation += 1
        bar.hide()
        clearSelection()
        onClosed?()
    }

    private func queryChanged(_ query: String) {
        pending?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.search(query) }
        pending = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.12, execute: work)
    }

    private func search(_ query: String) {
        pending = nil
        generation += 1
        let mine = generation
        guard let webView = webView(), !query.isEmpty else {
            total = 0
            bar.setStatus("", found: false)
            clearSelection()
            return
        }
        // Start from the top of the page: drop the selection a previous find
        // left, then count.
        let script = """
            (function (q) {
              const s = window.getSelection(); if (s) s.removeAllRanges();
              const t = (document.body ? document.body.innerText : '').toLowerCase();
              let n = 0, i = 0;
              while ((i = t.indexOf(q, i)) !== -1) { n++; i += q.length; }
              return n;
            })(\(Self.jsString(query.lowercased())))
            """
        webView.evaluateJavaScript(script) { [weak self] value, _ in
            MainActor.assumeIsolated {
                guard let self, mine == self.generation else { return }
                self.total = (value as? NSNumber)?.intValue ?? 0
                self.current = 0
                self.find(forward: true, first: true)
            }
        }
    }

    private func step(forward: Bool) {
        guard total > 0 else { return }
        find(forward: forward)
    }

    /// WebKit finds from the selection; `first` is the find right after a
    /// new query, from the top.
    private func find(forward: Bool, first: Bool = false) {
        guard let webView = webView() else { return }
        let config = WKFindConfiguration()
        config.backwards = !forward
        config.caseSensitive = false
        config.wraps = true
        let mine = generation
        webView.find(bar.query, configuration: config) { [weak self] result in
            MainActor.assumeIsolated {
                guard let self, mine == self.generation else { return }
                guard result.matchFound else {
                    self.bar.setStatus("No matches", found: false)
                    return
                }
                // Found, but not in the visible text we counted (rare).
                guard self.total > 0 else { return self.bar.setStatus("Found", found: true) }
                let n = self.total
                if first {
                    self.current = 1
                } else if forward {
                    self.current = self.current % n + 1
                } else {
                    self.current = (self.current + n - 2) % n + 1
                }
                self.bar.setStatus("\(self.current) of \(n)", found: true)
            }
        }
    }

    private func clearSelection() {
        webView()?.evaluateJavaScript("window.getSelection() && window.getSelection().removeAllRanges()")
    }

    private static func jsString(_ text: String) -> String {
        let data = (try? JSONSerialization.data(withJSONObject: [text])) ?? Data("[\"\"]".utf8)
        return String(decoding: data.dropFirst().dropLast(), as: UTF8.self)
    }

    func automationState() -> [String: String] {
        [
            "tile.find": bar.isOpen ? "open" : "closed",
            "tile.find.query": bar.query.isEmpty ? "-" : bar.query,
            "tile.find.status": bar.status.isEmpty ? "-" : bar.status,
        ]
    }
}
