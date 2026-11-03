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

    /// What's on screen: the file and how it's shown.
    private var shownPath: String?
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
        layer?.cornerRadius = 6
        layer?.masksToBounds = true
        layer?.backgroundColor = NSColor.textBackgroundColor.cgColor

        header.translatesAutoresizingMaskIntoConstraints = false
        header.onOpen = { [weak self] in self?.openInDefaultApp() }
        header.onReveal = { [weak self] in self?.revealInFinder() }
        body.translatesAutoresizingMaskIntoConstraints = false
        addSubview(header)
        addSubview(body)
        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: topAnchor),
            header.leadingAnchor.constraint(equalTo: leadingAnchor),
            header.trailingAnchor.constraint(equalTo: trailingAnchor),
            header.heightAnchor.constraint(equalToConstant: 26),
            body.topAnchor.constraint(equalTo: header.bottomAnchor),
            body.leadingAnchor.constraint(equalTo: leadingAnchor),
            body.trailingAnchor.constraint(equalTo: trailingAnchor),
            body.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        placeholder.alignment = .center
        placeholder.textColor = .secondaryLabelColor
        placeholder.font = .systemFont(ofSize: 13)
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
        layer?.backgroundColor = NSColor.textBackgroundColor.cgColor
    }

    // MARK: TileView

    func update(_ tile: TileModel) {
        guard tile != model else { return }
        let pathChanged = tile.path != model.path
        model = tile
        header.setPath(tile.path)
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
        guard focused, let window else { return }
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
        header.setPath(content.path, detail: content.detail, exists: content.kind != "missing")
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

    fileprivate func didFinishLoad() {
        guard let loading else { return }
        self.loading = nil
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

/// The strip above the content: the file's path and two actions.
@MainActor
private final class PreviewHeader: NSView {
    var onOpen: () -> Void = {}
    var onReveal: () -> Void = {}
    private let label = NSTextField(labelWithString: "")
    private let detail = NSTextField(labelWithString: "")
    private let open = NSButton(title: "Open", target: nil, action: nil)
    private let reveal = NSButton(title: "Reveal", target: nil, action: nil)

    init() {
        super.init(frame: .zero)
        wantsLayer = true
        label.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        label.textColor = .secondaryLabelColor
        label.lineBreakMode = .byTruncatingMiddle
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        detail.font = .systemFont(ofSize: 10)
        detail.textColor = .tertiaryLabelColor
        for button in [open, reveal] {
            button.bezelStyle = .accessoryBarAction
            button.controlSize = .small
            button.font = .systemFont(ofSize: 11)
            button.target = self
        }
        open.action = #selector(openClicked)
        open.toolTip = "Open in the default app"
        reveal.action = #selector(revealClicked)
        reveal.toolTip = "Reveal in Finder"
        let stack = NSStackView(views: [label, detail, NSView(), open, reveal])
        stack.orientation = .horizontal
        stack.spacing = 6
        stack.edgeInsets = NSEdgeInsets(top: 0, left: 8, bottom: 0, right: 6)
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
        updateColors()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func setPath(_ path: String?, detail text: String? = nil, exists: Bool = true) {
        label.stringValue = path.map { ($0 as NSString).abbreviatingWithTildeInPath } ?? "preview"
        label.toolTip = path
        if let text { detail.stringValue = text }
        open.isEnabled = path != nil && exists
        reveal.isEnabled = path != nil && exists
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        updateColors()
    }

    private func updateColors() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        }
    }

    @objc private func openClicked() { onOpen() }
    @objc private func revealClicked() { onReveal() }
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
    var revision: UInt64

    var isNative: Bool { ["image", "video", "audio", "pdf"].contains(kind) }

    /// A short note for the header: the syntax or kind, and truncation.
    var detail: String {
        var parts = [language ?? kind]
        if truncated == true { parts.append("truncated") }
        return parts.joined(separator: " · ")
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
