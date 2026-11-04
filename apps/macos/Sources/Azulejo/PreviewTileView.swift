import AppKit
import Quartz
import WebKit

/// A preview tile (F9): markdown, code and text rendered by the core
/// (`core/crates/preview`) into a WKWebView; images, video, audio and PDF shown
/// natively with QuickLook.
///
/// Content arrives as `preview.updated` events (on open, on restore and after
/// every change on disk) and from `preview.get` when the view is made. When
/// the tile already shows the same file, new HTML goes through
/// `azulejoPreview.swap(html)` (shared/webview/preview.js) so the reader keeps
/// their place; a different file, or a page that isn't ours yet, gets a full
/// load.
///
/// Webview safety: the only script on the page is the bundled preview.js. The
/// core drops raw HTML from markdown and every document carries a CSP that
/// allows inline scripts only, and this view refuses every navigation except
/// its own loads (links open in the default browser, or in this tile for local
/// files). Each page is written to `~/Library/Caches/Azulejo/preview/` and
/// loaded with read access to the folder holding both it and the previewed
/// file, so local images resolve (the core makes their URLs absolute); data
/// stores are non-persistent.
@MainActor
final class PreviewTileView: NSView, TileView {
    let tileID: String
    let kind: TileKind = .preview

    private var model: TileModel
    private let header = PreviewHeader()
    private let body = NSView()
    private var webView: WKWebView?
    private var nativeView: QLPreviewView?
    private let placeholder = NSTextField(wrappingLabelWithString: "")
    /// ⌘F: the bar sits under the header and pushes the page down.
    private lazy var finder: WebFinder = {
        let finder = WebFinder(webView: { [weak self] in self?.webView })
        finder.bar.onFocus = { [weak self] in self?.findFocused() }
        finder.onClosed = { [weak self] in self?.findClosed() }
        return finder
    }()
    private var findHeight: NSLayoutConstraint!

    /// What's on screen: the file and how it's shown.
    private var shownPath: String?
    /// The content on screen is the file's source (F29).
    private var shownSource = false
    private var shownMode: Mode = .none
    /// The newest revision applied (or being applied); older events are dropped.
    private var revision: UInt64 = 0
    /// Set while a full load is in flight: the path and revision it shows.
    private var loading: (path: String, revision: UInt64)?

    private enum Mode { case none, html, native, empty }

    // MARK: Setup

    /// Registers the tile kind and remembers how to reach the core. Called once
    /// at launch; `route` must then see every core event.
    static func install(core: Core, log: @escaping (String) -> Void) {
        Self.core = core
        Self.log = log
        TileViews.register(.preview) { tile in PreviewTileView(tile) }
    }

    /// Hands a core event to the preview tile it's for. Cheap for every other
    /// event: a substring check before any parsing.
    static func route(_ json: String) {
        guard json.contains("\"preview.updated\""),
              let data = json.data(using: .utf8),
              let event = try? JSONDecoder().decode(PreviewEvent.self, from: data),
              event.event == "preview.updated"
        else { return }
        live[event.data.tile]?.view?.apply(event.data)
    }

    private static var core: Core?
    private static var log: (String) -> Void = { _ in }
    private static var live: [String: WeakPreview] = [:]

    init(_ tile: TileModel) {
        tileID = tile.id
        model = tile
        super.init(frame: .zero)
        wantsLayer = true
        // Rectangular clip: the card (TileFrame) rounds the corners.
        layer?.masksToBounds = true
        layer?.backgroundColor = Design.Color.tile.cgColor

        // The controls live in the tile's header strip (TileFrame, F28/F29).
        header.onOpen = { [weak self] in self?.openInDefaultApp() }
        header.onReveal = { [weak self] in self?.revealInFinder() }
        header.onMode = { [weak self] source in self?.setSource(source) }
        header.setSource(tile.showsSource)
        body.translatesAutoresizingMaskIntoConstraints = false
        addSubview(body)
        let bar = finder.bar
        bar.translatesAutoresizingMaskIntoConstraints = false
        addSubview(bar)
        findHeight = bar.heightAnchor.constraint(equalToConstant: 0)
        NSLayoutConstraint.activate([
            bar.topAnchor.constraint(equalTo: topAnchor),
            bar.leadingAnchor.constraint(equalTo: leadingAnchor),
            bar.trailingAnchor.constraint(equalTo: trailingAnchor),
            findHeight,
            body.topAnchor.constraint(equalTo: bar.bottomAnchor),
            body.leadingAnchor.constraint(equalTo: leadingAnchor),
            body.trailingAnchor.constraint(equalTo: trailingAnchor),
            body.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        placeholder.alignment = .center
        placeholder.textColor = Design.Color.textSecondary
        placeholder.font = Design.Font.body
        placeholder.translatesAutoresizingMaskIntoConstraints = false

        Self.live[tileID] = WeakPreview(self)
        header.setPath(tile.path)
        if tile.path == nil {
            showEmpty()
        } else {
            fetch()
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        effectiveAppearance.performAsCurrentDrawingAppearance {
            layer?.backgroundColor = Design.Color.tile.cgColor
        }
    }

    // MARK: TileView

    func update(_ tile: TileModel) {
        guard tile != model else { return }
        let pathChanged = tile.path != model.path
        model = tile
        header.setPath(tile.path)
        header.setSource(tile.showsSource)
        guard pathChanged else { return }
        if tile.path == nil {
            showEmpty()
        } else {
            // The core renders on open and sends `preview.updated`; asking too
            // covers a path set while this view was being made.
            fetch()
        }
    }

    func setFocused(_ focused: Bool) {
        self.focused = focused
        guard focused, let window, TileFocus.canTake(in: window) else { return }
        // Typing in this tile's find bar: refreshes leave it there.
        if finder.bar.holdsKeyboard { return }
        let target: NSView = webView ?? nativeView ?? self
        if window.firstResponder !== target { window.makeFirstResponder(target) }
    }

    func tileWillClose() {
        Self.live[tileID] = nil
        webView?.stopLoading()
        webView?.navigationDelegate = nil
        nativeView?.close()
        if let page = try? Self.pageFile(for: tileID) { try? FileManager.default.removeItem(at: page) }
    }

    override var acceptsFirstResponder: Bool { true }

    // MARK: Content

    /// Asks the core for the latest render of this tile.
    private func fetch() {
        guard let core = Self.core else { return }
        switch core.execute(["method": "preview.get", "params": ["tile": tileID]]) {
        case .success(let reply):
            guard let data = try? JSONSerialization.data(withJSONObject: reply),
                  let content = try? JSONDecoder().decode(PreviewContent.self, from: data)
            else { return }
            apply(content)
        case .failure(let error):
            Self.log("preview \(Short.id(tileID)): preview.get failed: \(error)")
        }
    }

    private func apply(_ content: PreviewContent) {
        // Same file, same or older revision: already showing it (or newer).
        if content.revision < revision || (content.revision == revision && content.path == shownPath) {
            return
        }
        revision = content.revision
        // Rendered ↔ source is a different page, not an edit: load it afresh.
        let switchedMode = (content.mode == "source") != shownSource
        shownSource = content.mode == "source"
        if switchedMode, shownMode == .html { loading = nil; shownPath = nil }
        header.setPath(content.path, detail: content.detail, exists: content.kind != "missing")
        header.setSourceAvailable(!content.isNative)
        if content.isNative {
            showNative(content)
        } else if let html = content.html {
            showHTML(html, content: content)
        } else {
            showEmpty("Nothing to show for \(content.path)")
        }
    }

    private func showHTML(_ html: String, content: PreviewContent) {
        let web = ensureWebView()
        let started = Date()
        if shownMode == .html, shownPath == content.path, loading == nil {
            let script = "window.azulejoPreview ? window.azulejoPreview.swap(\(Self.jsString(html))) : false"
            web.evaluateJavaScript(script) { [weak self] result, error in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    if error == nil, result as? Bool == true {
                        self.logApplied(content, mode: "swap", started: started)
                    } else {
                        self.load(html, content: content)
                    }
                }
            }
        } else {
            load(html, content: content)
        }
    }

    private func load(_ html: String, content: PreviewContent) {
        let web = ensureWebView()
        show(web, mode: .html)
        shownPath = content.path
        loading = (content.path, content.revision)
        loadStarted = Date()
        appliedKind = content.kind
        // `loadHTMLString(_:baseURL:)` gives the page no read access to the
        // file's folder, so local images stay broken. Write the document to a
        // cache file and load that with read access to the closest folder
        // holding both it and the previewed file (the home folder for files
        // under it). The core rewrites relative links to absolute file URLs, so
        // the page's own location doesn't matter.
        let folder = URL(fileURLWithPath: content.path).deletingLastPathComponent()
        do {
            let page = try Self.pageFile(for: tileID)
            try html.write(to: page, atomically: true, encoding: .utf8)
            web.loadFileURL(page, allowingReadAccessTo: Self.commonAncestor(page, folder))
        } catch {
            Self.log("preview \(Short.id(tileID)): can't write the page (\(error)); loading without file access")
            web.loadHTMLString(html, baseURL: folder)
        }
    }

    /// `~/Library/Caches/Azulejo/preview/<tile>.html`.
    private static func pageFile(for tile: String) throws -> URL {
        let caches = try FileManager.default.url(
            for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        let dir = caches.appendingPathComponent("Azulejo/preview", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("\(tile).html")
    }

    /// The deepest folder containing both `a` and `b`.
    private static func commonAncestor(_ a: URL, _ b: URL) -> URL {
        let x = a.standardizedFileURL.resolvingSymlinksInPath().pathComponents
        let y = b.standardizedFileURL.resolvingSymlinksInPath().pathComponents
        let shared = zip(x, y).prefix { $0 == $1 }.map(\.0)
        guard !shared.isEmpty else { return URL(fileURLWithPath: "/", isDirectory: true) }
        return URL(fileURLWithPath: NSString.path(withComponents: shared), isDirectory: true)
    }

    private var loadStarted = Date()
    private var focused = false

    // Automation state (`tile.*` keys).
    private var applied = 0
    private var appliedMode = "-"
    private var appliedKind = "-"
    private var lastScriptResult = "-"

    fileprivate func didFinishLoad() {
        guard let loading else { return }
        self.loading = nil
        applied += 1
        appliedMode = "load"
        Self.log(String(
            format: "preview.applied tile=%@ rev=%llu mode=load path=%@ t=%.6f took=%.1fms",
            Short.id(tileID), loading.revision, loading.path,
            Date().timeIntervalSince1970, Date().timeIntervalSince(loadStarted) * 1000))
        guard Self.diagnostics, let webView else { return }
        // Dev runs: what the page holds, and whether local images resolved.
        let probe = """
            [document.getElementById('azj-content') ? document.getElementById('azj-content').innerText.length : -1,
             Array.from(document.images).map(function (i) { return i.naturalWidth + 'x' + i.naturalHeight; }).join(','),
             typeof window.azulejoPreview].join(' ')
            """
        webView.evaluateJavaScript(probe) { [tileID] result, error in
            MainActor.assumeIsolated {
                Self.log("preview.dom tile=\(Short.id(tileID)) \(result.map { "\($0)" } ?? "error: \(String(describing: error))")")
            }
        }
    }

    /// Dev and automation runs log what each loaded page holds.
    private static let diagnostics = ProcessInfo.processInfo.environment["AZULEJO_PROFILE"] == "dev"
    private static let background = ProcessInfo.processInfo.environment["AZULEJO_BACKGROUND"] == "1"
        || ProcessInfo.processInfo.environment["AZULEJO_AUTOMATION"] != nil

    private func logApplied(_ content: PreviewContent, mode: String, started: Date) {
        applied += 1
        appliedMode = mode
        appliedKind = content.kind
        Self.log(String(
            format: "preview.applied tile=%@ rev=%llu mode=%@ path=%@ t=%.6f took=%.1fms",
            Short.id(tileID), content.revision, mode, content.path,
            Date().timeIntervalSince1970, Date().timeIntervalSince(started) * 1000))
    }

    private func showNative(_ content: PreviewContent) {
        let started = Date()
        let url = URL(fileURLWithPath: content.path)
        let preview = ensureNativeView()
        show(preview, mode: .native)
        if shownPath == content.path, preview.previewItem != nil {
            // Same file changed on disk.
            preview.refreshPreviewItem()
        } else {
            preview.previewItem = url as NSURL
        }
        shownPath = content.path
        logApplied(content, mode: "native-\(content.kind)", started: started)
    }

    private func showEmpty(_ text: String = "No file yet.\nazj preview open <path>") {
        placeholder.stringValue = text
        show(placeholder, mode: .empty)
        shownPath = nil
    }

    /// Puts `view` in the body, removing whatever was there.
    private func show(_ view: NSView, mode: Mode) {
        shownMode = mode
        // Quick Look and the empty state have nothing to find in.
        if mode != .html, finder.bar.isOpen {
            finder.bar.hide()
            findHeight.constant = 0
        }
        guard view.superview !== body else { return }
        body.subviews.forEach { $0.removeFromSuperview() }
        view.translatesAutoresizingMaskIntoConstraints = false
        body.addSubview(view)
        if view === placeholder {
            NSLayoutConstraint.activate([
                view.centerXAnchor.constraint(equalTo: body.centerXAnchor),
                view.centerYAnchor.constraint(equalTo: body.centerYAnchor),
                view.widthAnchor.constraint(lessThanOrEqualTo: body.widthAnchor, constant: -24),
            ])
        } else {
            NSLayoutConstraint.activate([
                view.topAnchor.constraint(equalTo: body.topAnchor),
                view.leadingAnchor.constraint(equalTo: body.leadingAnchor),
                view.trailingAnchor.constraint(equalTo: body.trailingAnchor),
                view.bottomAnchor.constraint(equalTo: body.bottomAnchor),
            ])
        }
        if let window, window.firstResponder === self || window.firstResponder === nil {
            window.makeFirstResponder(view === placeholder ? self : view)
        }
    }

    private func ensureWebView() -> WKWebView {
        if let webView { return webView }
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.suppressesIncrementalRendering = false
        let web = WKWebView(frame: .zero, configuration: config)
        web.navigationDelegate = navigation
        // Transparent until the page paints (avoids a white flash in dark
        // mode). Private, so only when WebKit still has it: an unknown KVC key
        // raises NSUnknownKeyException and aborts the app.
        if web.responds(to: NSSelectorFromString("_setDrawsBackground:"))
            || web.responds(to: NSSelectorFromString("setDrawsBackground:"))
        {
            web.setValue(false, forKey: "drawsBackground")
        }
        if Self.background {
            // Background runs (dev --bg, automation) keep the window behind
            // others; WebKit stops painting occluded windows, so screenshots of
            // our own window would come out blank. SPI, dev-only.
            // A nil object argument arrives as BOOL NO.
            let setter = NSSelectorFromString("_setWindowOcclusionDetectionEnabled:")
            if web.responds(to: setter) { _ = web.perform(setter, with: nil) }
        }
        navigation.view = self
        webView = web
        return web
    }

    private lazy var navigation = PreviewNavigation()

    private func ensureNativeView() -> QLPreviewView {
        if let nativeView { return nativeView }
        let view = QLPreviewView(frame: .zero, style: .normal)!
        view.shouldCloseWithWindow = false
        view.autostarts = false
        nativeView = view
        return view
    }

    // MARK: Header actions

    /// Preview | Source (F29): the core re-renders and saves the choice.
    private func setSource(_ source: Bool) {
        guard source != model.showsSource || source != shownSource else { return }
        let mode = source ? "source" : "rendered"
        if case .failure(let error)? = Self.core?.execute([
            "method": "preview.set_mode", "params": ["tile": tileID, "mode": mode],
        ]) {
            Self.log("preview \(Short.id(tileID)): preview.set_mode failed: \(error)")
            header.setSource(model.showsSource)
        }
    }

    private func openInDefaultApp() {
        guard let path = shownPath ?? model.path else { return }
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    private func revealInFinder() {
        guard let path = shownPath ?? model.path else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    /// Link clicks: local files open in this tile, everything else in the
    /// default browser.
    fileprivate func follow(_ url: URL) {
        if url.isFileURL {
            _ = Self.core?.execute([
                "method": "preview.open", "params": ["tile": tileID, "path": url.path],
            ])
        } else if ["http", "https", "mailto"].contains(url.scheme?.lowercased() ?? "") {
            NSWorkspace.shared.open(url)
        }
    }

    /// A JavaScript string literal for `text`.
    private static func jsString(_ text: String) -> String {
        let data = (try? JSONSerialization.data(withJSONObject: [text])) ?? Data("[\"\"]".utf8)
        let array = String(decoding: data, as: UTF8.self)
        return String(array.dropFirst().dropLast())
    }
}

/// Blocks every navigation except the tile's own loads, and reports when a
/// load finishes.
@MainActor
private final class PreviewNavigation: NSObject, WKNavigationDelegate {
    weak var view: PreviewTileView?

    func webView(
        _ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        switch action.navigationType {
        case .other, .reload:
            // Our own loadHTMLString (and in-page anchors, which stay on the page).
            decisionHandler(.allow)
        case .linkActivated:
            if let url = action.request.url {
                if url.fragment != nil, url.path == webView.url?.path || url.scheme == "about" {
                    decisionHandler(.allow) // in-page anchor
                    return
                }
                view?.follow(url)
            }
            decisionHandler(.cancel)
        default:
            decisionHandler(.cancel)
        }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        view?.didFinishLoad()
    }
}

/// The preview's controls in its tile header (TileFrame): the syntax,
/// Preview | Source (F29), and two actions. The file's name and folder are
/// the header's own title and subtitle.
@MainActor
/// The preview tile's part of the header strip (D.7): the Preview | Source
/// toggle on the right, like the mockup. Open and Reveal show while the
/// pointer is over the header; the syntax and kind go in the toggle's tooltip.
private final class PreviewHeader: NSView, HeaderHoverAware {
    var onOpen: () -> Void = {}
    var onReveal: () -> Void = {}
    var onMode: (Bool) -> Void = { _ in }
    private let mode = PillToggle(labels: ["Preview", "Source"])
    private let open = NSButton()
    private let reveal = NSButton()
    private let stack: NSStackView

    init() {
        stack = NSStackView(views: [open, reveal, mode])
        super.init(frame: .zero)
        mode.onChange = { [weak self] _ in self?.modeChanged() }
        mode.toolTip = "Show the file rendered, or its source"
        for (button, symbol, tip) in [
            (open, "arrow.up.forward.app", "Open in the default app"),
            (reveal, "folder", "Reveal in Finder"),
        ] {
            button.bezelStyle = .accessoryBarAction
            button.isBordered = false
            button.controlSize = .small
            button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: tip)
            button.contentTintColor = Design.Color.textMuted
            button.toolTip = tip
            button.target = self
            button.isHidden = true
        }
        open.action = #selector(openClicked)
        reveal.action = #selector(revealClicked)
        stack.orientation = .horizontal
        stack.spacing = 4
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var fittingSize: NSSize { stack.fittingSize }

    func setPath(_ path: String?, detail text: String? = nil, exists: Bool = true) {
        if let text { mode.toolTip = "\(text) — show the file rendered, or its source" }
        open.isEnabled = path != nil && exists
        reveal.isEnabled = path != nil && exists
    }

    /// Which segment shows as selected.
    func setSource(_ source: Bool) {
        mode.selectedSegment = source ? 1 : 0
    }

    func headerHover(_ inside: Bool) {
        open.isHidden = !inside
        reveal.isHidden = !inside
    }

    /// Images, video and PDF have no source to show.
    func setSourceAvailable(_ available: Bool) {
        mode.isHidden = !available
    }

    /// Automation: what the toggle shows.
    var selectedMode: String { mode.isHidden ? "none" : (mode.selectedSegment == 1 ? "source" : "rendered") }

    /// Automation: what a click on a segment does.
    func choose(source: Bool) {
        mode.selectedSegment = source ? 1 : 0
        modeChanged()
    }

    /// Automation: Open and Reveal show (pointer over the header).
    var actionsShown: Bool { !open.isHidden }

    @objc private func modeChanged() { onMode(mode.selectedSegment == 1) }
    @objc private func openClicked() { onOpen() }
    @objc private func revealClicked() { onReveal() }
}

extension PreviewTileView: TileHeaderAccessory {
    var headerAccessory: NSView? { header }
}

/// `preview.updated` / `preview.get`, as the core sends them
/// (`azulejo_protocol::PreviewContent`).
struct PreviewContent: Decodable {
    var tile: String
    var path: String
    var kind: String
    var html: String?
    var language: String?
    var truncated: Bool?
    /// `source` when the file shows as highlighted source (F29).
    var mode: String?
    var revision: UInt64

    var isNative: Bool { ["image", "video", "audio", "pdf"].contains(kind) }

    /// A short note for the header: the syntax or kind, and truncation.
    var detail: String {
        var parts = [language ?? kind]
        if truncated == true { parts.append("truncated") }
        return parts.joined(separator: " · ")
    }
}

// MARK: - Find (⌘F)

extension PreviewTileView: Findable {
    var findBar: FindBar? { finder.bar }

    func beginFind() -> String? {
        guard shownMode == .html, webView != nil else {
            return shownMode == .native ? "Find works in rendered and source previews, not Quick Look"
                : "nothing to find yet"
        }
        findHeight.constant = FindBar.height
        layoutSubtreeIfNeeded()
        finder.open()
        return nil
    }

    fileprivate func findFocused() {
        // A click into the bar of a tile that isn't focused focuses it.
        if !focused { Self.core?.send(["method": "tile.focus", "params": ["tile": tileID]]) }
    }

    fileprivate func findClosed() {
        findHeight.constant = 0
        if let webView { window?.makeFirstResponder(webView) }
    }
}

// MARK: - Automation

extension PreviewTileView: AutomatableTile {
    /// `tile js <script>`: evaluates in the page; the result shows as `tile.js`.
    func automate(_ verb: String, _ arg: String) -> String? {
        switch verb {
        case "js":
            guard let webView else { return "no page loaded" }
            lastScriptResult = "pending"
            webView.evaluateJavaScript(arg) { [weak self] value, error in
                MainActor.assumeIsolated {
                    self?.lastScriptResult = error.map { "error: \($0.localizedDescription)" }
                        ?? value.map { "\($0)" } ?? "nil"
                }
            }
            return nil
        case "mode":
            // `tile mode source|rendered`: clicks the header's toggle.
            guard arg == "source" || arg == "rendered" else { return "mode takes source or rendered" }
            header.choose(source: arg == "source")
            return nil
        default:
            return "unknown preview verb \(verb)"
        }
    }

    func automationState() -> [String: String] {
        [
            "tile.kind": "preview",
            "tile.applied": "\(applied)",
            "tile.mode": appliedMode,
            "tile.content": appliedKind,
            "tile.path": shownPath ?? "-",
            "tile.js": lastScriptResult,
            "tile.source": shownSource ? "source" : "rendered",
            "tile.toggle": header.selectedMode,
        ].merging(finder.automationState()) { $1 }
    }
}

private struct PreviewEvent: Decodable {
    var event: String
    var data: PreviewContent
}

@MainActor
private final class WeakPreview {
    weak var view: PreviewTileView?
    init(_ view: PreviewTileView) { self.view = view }
}
