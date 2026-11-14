import AppKit
import CryptoKit
import WebKit

/// A browser tile (F8): a WKWebView under a small toolbar (back, forward,
/// reload, address bar, menu), with a thin progress line and a banner for
/// crashes, failed loads and downloads.
///
/// URL ownership: `browser.open` (an agent, the CLI, restore) sets the tile's
/// URL in the core and this view loads it. Navigation inside the page (links,
/// redirects, back/forward) is reported back with `browser.state`, which
/// updates the core's URL and page title without asking for a load. To tell
/// those echoes apart from a real `browser.open`, the view remembers the URLs
/// it reported and skips them when they come back through `update(_:)`.
///
/// Data: each workspace gets its own persistent `WKWebsiteDataStore`
/// (cookies, local storage, cache), keyed by a UUID derived from the
/// workspace ID, so two projects' localhost logins don't collide. There is no
/// per-workspace process pool: `WKProcessPool` has had no effect since macOS
/// 12, and WebKit already isolates web content per site.
///
/// New windows (`target=_blank`, `window.open`) open in the same tile.
/// Downloads go to ~/Downloads with a banner.
@MainActor
final class BrowserTileView: NSView, TileView {
    let tileID: String
    let kind: TileKind = .browser

    private weak var app: AppController?
    private let webView: WKWebView
    private let toolbar = NSView()
    private let back = BrowserTileView.button("chevron.left", "Back (⌘[)")
    private let forward = BrowserTileView.button("chevron.right", "Forward (⌘])")
    private let reload = BrowserTileView.button("arrow.clockwise", "Reload (⌘R)")
    private let more = BrowserTileView.button("ellipsis.circle", "More")
    private let address = NSTextField()
    private let progress = NSView()
    private let banner = BrowserBanner()
    private var observations: [NSKeyValueObservation] = []

    /// The URL the core last told us about.
    private var coreURL: String?
    /// URLs we reported with `browser.state` and haven't seen echoed yet.
    private var pendingEchoes: [String] = []
    /// The last state we reported, so unchanged KVO bursts send nothing.
    private var reported: (url: String?, title: String?) = (nil, nil)
    private var reportScheduled = false
    private var focused = false

    static let toolbarHeight: CGFloat = 30

    /// Registers the browser tile kind. Called once at launch.
    static func register(app: AppController) {
        TileViews.register(.browser) { [weak app] tile in
            BrowserTileView(tile, app: app)
        }
    }

    init(_ tile: TileModel, app: AppController?) {
        tileID = tile.id
        self.app = app
        let config = WKWebViewConfiguration()
        config.websiteDataStore = BrowserStores.store(forWorkspace: BrowserTileView.workspace(of: tile.id, in: app?.session))
        config.preferences.isElementFullscreenEnabled = true
        BrowserTileView.enableDeveloperExtras(config.preferences)
        webView = WKWebView(frame: .zero, configuration: config)
        super.init(frame: .zero)

        wantsLayer = true
        layer?.cornerRadius = 6
        layer?.masksToBounds = true
        layer?.backgroundColor = NSColor.textBackgroundColor.cgColor

        webView.isInspectable = true
        webView.allowsBackForwardNavigationGestures = true
        webView.allowsMagnification = true
        webView.navigationDelegate = self
        webView.uiDelegate = self

        address.placeholderString = "Address or search (⌘L)"
        address.bezelStyle = .roundedBezel
        address.font = .systemFont(ofSize: 12)
        address.lineBreakMode = .byTruncatingTail
        address.usesSingleLineMode = true
        address.cell?.isScrollable = true
        address.target = self
        address.action = #selector(addressEntered)
        address.delegate = self

        back.target = self; back.action = #selector(goBack)
        forward.target = self; forward.action = #selector(goForward)
        reload.target = self; reload.action = #selector(reloadOrStop)
        more.target = self; more.action = #selector(showMore(_:))

        progress.wantsLayer = true
        progress.layer?.backgroundColor = NSColor.controlAccentColor.cgColor
        progress.isHidden = true

        banner.isHidden = true

        [back, forward, reload, address, more].forEach(toolbar.addSubview)
        [toolbar, webView, progress, banner].forEach(addSubview)

        observe()
        refreshControls()

        coreURL = tile.url
        // Agents may hand over bare addresses (`localhost:3000`); load() fixes
        // them up the same way the address bar does.
        if let url = tile.url { load(url) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    // MARK: TileView

    func update(_ tile: TileModel) {
        guard tile.url != coreURL else { return }
        coreURL = tile.url
        guard let url = tile.url else { return }
        if let i = pendingEchoes.firstIndex(of: url) {
            // Our own browser.state coming back (possibly several coalesced
            // into one refresh): nothing to load.
            pendingEchoes.removeFirst(i + 1)
            return
        }
        load(url)
    }

    func setFocused(_ focused: Bool) {
        self.focused = focused
        guard focused, let window, TileFocus.canTake(in: window) else { return }
        if isEditingAddress { return }
        if webView.url == nil && coreURL == nil {
            focusAddress()
        } else if !containsFirstResponder {
            window.makeFirstResponder(webView)
        }
    }

    func tileWillClose() {
        observations.removeAll()
        webView.stopLoading()
        webView.navigationDelegate = nil
        webView.uiDelegate = nil
        // Releases the web content process for this page.
        webView.loadHTMLString("", baseURL: nil)
    }

    // MARK: Layout

    override var isFlipped: Bool { true }

    override func layout() {
        super.layout()
        let w = bounds.width
        let h = Self.toolbarHeight
        toolbar.frame = NSRect(x: 0, y: 0, width: w, height: h)
        var x: CGFloat = 6
        for b in [back, forward, reload] {
            b.frame = NSRect(x: x, y: 4, width: 24, height: 22)
            x += 26
        }
        more.frame = NSRect(x: w - 30, y: 4, width: 24, height: 22)
        address.frame = NSRect(x: x + 4, y: 4, width: max(40, w - x - 4 - 36), height: 22)
        webView.frame = NSRect(x: 0, y: h, width: w, height: max(0, bounds.height - h))
        layoutProgress()
        let bh: CGFloat = 30
        banner.frame = NSRect(x: 8, y: h + 8, width: max(0, w - 16), height: bh)
    }

    private func layoutProgress() {
        let fraction = CGFloat(webView.estimatedProgress)
        progress.frame = NSRect(x: 0, y: Self.toolbarHeight - 2, width: bounds.width * fraction, height: 2)
    }

    // MARK: Keys

    /// Browser chords. The key router has already let these through because
    /// the keymap doesn't bind them; only the tile holding the first responder
    /// acts, since every view in the window sees key equivalents.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard event.type == .keyDown, containsFirstResponder, let chord = KeyRouter.chord(for: event) else {
            return super.performKeyEquivalent(with: event)
        }
        return handle(chord: chord) || super.performKeyEquivalent(with: event)
    }

    /// Runs a browser chord; false if it isn't one.
    @discardableResult
    func handle(chord: String) -> Bool {
        switch chord {
        case "cmd+l": focusAddress()
        case "cmd+r": webView.reload()
        case "cmd+shift+r": webView.reloadFromOrigin()
        case "cmd+bracketleft": goBack()
        case "cmd+bracketright": goForward()
        case "alt+cmd+i": showInspector()
        case "cmd+period": webView.stopLoading()
        case "cmd+equal": webView.magnification = min(3, webView.magnification + 0.1)
        case "cmd+minus": webView.magnification = max(0.5, webView.magnification - 0.1)
        case "cmd+0": webView.magnification = 1
        case "escape" where isEditingAddress:
            address.stringValue = webView.url?.absoluteString ?? coreURL ?? ""
            window?.makeFirstResponder(webView)
        default: return false
        }
        return true
    }

    private var containsFirstResponder: Bool {
        guard let responder = window?.firstResponder as? NSView else { return false }
        return responder === self || responder.isDescendant(of: self)
    }

    private var isEditingAddress: Bool {
        guard let editor = address.currentEditor() else { return false }
        return window?.firstResponder === editor
    }

    override func cancelOperation(_ sender: Any?) {
        if isEditingAddress { handle(chord: "escape") } else { super.cancelOperation(sender) }
    }

    // MARK: Actions

    func load(_ text: String) {
        guard let url = Self.normalise(text) else { return }
        banner.hide()
        address.stringValue = url.absoluteString
        webView.load(URLRequest(url: url))
    }

    func focusAddress() {
        guard let window else { return }
        window.makeFirstResponder(address)
        address.currentEditor()?.selectAll(nil)
    }

    @objc private func addressEntered() {
        let text = address.stringValue
        guard let url = Self.normalise(text) else { return }
        // Typed navigation goes through the core like any other open, so
        // agents and restore see it. The echo comes back via update(_:).
        if let app, case .success = app.send([
            "method": "browser.open", "params": ["tile": tileID, "url": url.absoluteString],
        ]) {
            // update(_:) will load it; if the core already had this URL,
            // nothing changes there, so load directly.
            if coreURL == url.absoluteString { load(url.absoluteString) }
        } else {
            load(url.absoluteString)
        }
        window?.makeFirstResponder(webView)
    }

    @objc func goBack() { if webView.canGoBack { webView.goBack() } }
    @objc func goForward() { if webView.canGoForward { webView.goForward() } }
    @objc private func reloadOrStop() {
        if webView.isLoading { webView.stopLoading() } else { webView.reload() }
    }

    @objc private func showMore(_ sender: NSButton) {
        let menu = NSMenu()
        menu.addItem(withTitle: "Web Inspector", action: #selector(showInspector), keyEquivalent: "i")
            .keyEquivalentModifierMask = [.command, .option]
        menu.addItem(withTitle: "Copy URL", action: #selector(copyURL), keyEquivalent: "")
        menu.addItem(withTitle: "Open in Default Browser", action: #selector(openExternally), keyEquivalent: "")
        menu.addItem(.separator())
        menu.addItem(withTitle: "Reload from Origin", action: #selector(reloadFromOrigin), keyEquivalent: "")
        menu.items.forEach { $0.target = self }
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: sender.bounds.height + 2), in: sender)
    }

    /// Opens the Web Inspector. WebKit has no public call for it, so this uses
    /// `_inspector.show` when present; `isInspectable` also makes the page
    /// show up in Safari's Develop menu, and the context menu has Inspect
    /// Element.
    @objc func showInspector() {
        guard let inspector = Self.inspector(of: webView) else {
            showBanner("Web Inspector isn't available; use Safari ▸ Develop or right-click ▸ Inspect Element", .info)
            return
        }
        _ = inspector.perform(NSSelectorFromString("show"))
    }

    @objc private func copyURL() {
        guard let url = webView.url?.absoluteString ?? coreURL else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(url, forType: .string)
    }

    @objc private func openExternally() {
        if let url = webView.url { NSWorkspace.shared.open(url) }
    }

    @objc private func reloadFromOrigin() { webView.reloadFromOrigin() }

    // MARK: State

    private func observe() {
        observations = [
            webView.observe(\.url) { [weak self] _, _ in MainActor.assumeIsolated { self?.navigated() } },
            webView.observe(\.title) { [weak self] _, _ in MainActor.assumeIsolated { self?.scheduleReport() } },
            webView.observe(\.estimatedProgress) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.layoutProgress() }
            },
            webView.observe(\.isLoading) { [weak self] _, _ in MainActor.assumeIsolated { self?.refreshControls() } },
            webView.observe(\.canGoBack) { [weak self] _, _ in MainActor.assumeIsolated { self?.refreshControls() } },
            webView.observe(\.canGoForward) { [weak self] _, _ in MainActor.assumeIsolated { self?.refreshControls() } },
        ]
    }

    private func navigated() {
        if !isEditingAddress { address.stringValue = webView.url?.absoluteString ?? coreURL ?? "" }
        scheduleReport()
    }

    private func refreshControls() {
        back.isEnabled = webView.canGoBack
        forward.isEnabled = webView.canGoForward
        let loading = webView.isLoading
        reload.image = NSImage(systemSymbolName: loading ? "xmark" : "arrow.clockwise",
                               accessibilityDescription: loading ? "Stop" : "Reload")
        reload.toolTip = loading ? "Stop (⌘.)" : "Reload (⌘R)"
        progress.isHidden = !loading
        layoutProgress()
    }

    /// Reports URL and title to the core once per run-loop turn.
    private func scheduleReport() {
        guard !reportScheduled else { return }
        reportScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.reportScheduled = false
            self.report()
        }
    }

    private func report() {
        guard let app else { return }
        let url = webView.url?.absoluteString
        // about:blank while a new tile waits for an address isn't worth keeping.
        let keepURL = url.flatMap { $0 == "about:blank" ? nil : $0 }
        let title = webView.title.flatMap { $0.isEmpty ? nil : $0 }
        guard keepURL != reported.url || title != reported.title else { return }
        var params: [String: Any] = ["tile": tileID, "page_title": title ?? ""]
        if let keepURL {
            params["url"] = keepURL
            if keepURL != coreURL { pendingEchoes.append(keepURL) }
            if pendingEchoes.count > 32 { pendingEchoes.removeFirst(pendingEchoes.count - 32) }
        }
        reported = (keepURL, title)
        app.send(["method": "browser.state", "params": params])
    }

    fileprivate func showBanner(_ text: String, _ style: BrowserBanner.Style, action: (String, () -> Void)? = nil) {
        banner.show(text, style: style, action: action)
        needsLayout = true
    }

    // MARK: Helpers

    /// The workspace holding a tile, for its website data store.
    static func workspace(of tile: String, in session: SessionModel?) -> String? {
        session?.windows.lazy.flatMap(\.workspaces)
            .first { $0.tabs.contains { $0.tiles.contains { $0.id == tile } } }?.id
    }

    /// What the address bar accepts: a URL with a scheme, a local host
    /// (`localhost:3000`, `127.0.0.1:8080/x`, `myapp.test`) as http, anything
    /// else that looks like a host as https, and the rest as a search.
    static func normalise(_ raw: String) -> URL? {
        let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return nil }
        let local = #"^(localhost|127(\.\d{1,3}){3}|0\.0\.0\.0|\[::1\]|[\w.-]+\.(localhost|local|test))(:\d+)?([/?#].*)?$"#
        if text.range(of: local, options: [.regularExpression, .caseInsensitive]) != nil {
            return URL(string: "http://" + text)
        }
        let ipPort = #"^\d{1,3}(\.\d{1,3}){3}(:\d+)?([/?#].*)?$"#
        if text.range(of: ipPort, options: .regularExpression) != nil {
            return URL(string: "http://" + text)
        }
        if text.range(of: #"^[a-zA-Z][a-zA-Z0-9+.-]*://"#, options: .regularExpression) != nil
            || text.hasPrefix("about:") || text.hasPrefix("file:") || text.hasPrefix("data:") {
            return URL(string: text)
        }
        let hostLike = #"^[\w-]+(\.[\w-]+)+(:\d+)?([/?#].*)?$"#
        if !text.contains(" "), text.range(of: hostLike, options: .regularExpression) != nil {
            return URL(string: "https://" + text)
        }
        var search = URLComponents(string: "https://duckduckgo.com/")!
        search.queryItems = [URLQueryItem(name: "q", value: text)]
        return search.url
    }

    private static func button(_ symbol: String, _ tip: String) -> NSButton {
        let image = NSImage(systemSymbolName: symbol, accessibilityDescription: tip) ?? NSImage()
        let b = NSButton(image: image, target: nil, action: nil)
        b.bezelStyle = .accessoryBarAction
        b.isBordered = false
        b.toolTip = tip
        return b
    }

    /// Turns on the context menu's Inspect Element (a private preference,
    /// set only if WebKit still has it).
    private static func enableDeveloperExtras(_ preferences: WKPreferences) {
        if WKPreferences.instancesRespond(to: NSSelectorFromString("_setDeveloperExtrasEnabled:")) {
            preferences.setValue(true, forKey: "developerExtrasEnabled")
        }
    }

    fileprivate static func inspector(of webView: WKWebView) -> NSObject? {
        let selector = NSSelectorFromString("_inspector")
        guard webView.responds(to: selector),
              let inspector = webView.perform(selector)?.takeUnretainedValue() as? NSObject,
              inspector.responds(to: NSSelectorFromString("show"))
        else { return nil }
        return inspector
    }
}

// MARK: - Navigation, UI and downloads

extension BrowserTileView: WKNavigationDelegate, WKUIDelegate, WKDownloadDelegate, NSTextFieldDelegate {
    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                 preferences: WKWebpagePreferences) async -> (WKNavigationActionPolicy, WKWebpagePreferences) {
        (navigationAction.shouldPerformDownload ? .download : .allow, preferences)
    }

    func webView(_ webView: WKWebView, decidePolicyFor navigationResponse: WKNavigationResponse) async
        -> WKNavigationResponsePolicy {
        navigationResponse.canShowMIMEType ? .allow : .download
    }

    func webView(_ webView: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) {
        download.delegate = self
    }

    func webView(_ webView: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) {
        download.delegate = self
    }

    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        banner.hide()
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        loadFailed(error)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        loadFailed(error)
    }

    private func loadFailed(_ error: Error) {
        let ns = error as NSError
        // Cancelled (a newer load replaced it) and "frame load interrupted"
        // (the response became a download) aren't failures.
        if ns.domain == NSURLErrorDomain && ns.code == NSURLErrorCancelled { return }
        if ns.domain == "WebKitErrorDomain" && ns.code == 102 { return }
        showBanner(ns.localizedDescription, .error, action: ("Retry", { [weak self] in self?.webView.reload() }))
    }

    /// Crash isolation: the page's web content process died. The tile and the
    /// rest of the app are fine; offer a reload.
    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        showBanner("This page stopped working.", .error, action: ("Reload", { [weak self] in
            self?.banner.hide()
            self?.webView.reload()
        }))
    }

    /// `target=_blank` and `window.open` load in this tile.
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if navigationAction.targetFrame == nil { webView.load(navigationAction.request) }
        return nil
    }

    // Downloads land in ~/Downloads under a free name.
    func download(_ download: WKDownload, decideDestinationUsing response: URLResponse,
                  suggestedFilename: String) async -> URL? {
        let folder = Self.downloadsFolder
        // File checks off the main thread: the first touch of ~/Downloads can
        // wait on macOS's privacy prompt.
        let destination = await Task.detached { Self.freeName(suggestedFilename, in: folder) }.value
        showBanner("Downloading \(destination.lastPathComponent)…", .info)
        downloads[ObjectIdentifier(download)] = destination
        return destination
    }

    func downloadDidFinish(_ download: WKDownload) {
        let name = downloads.removeValue(forKey: ObjectIdentifier(download))?.lastPathComponent ?? "file"
        let folder = Self.downloadsFolder
        showBanner("Saved \(name) to \(folder.lastPathComponent)", .info, action: ("Show", {
            NSWorkspace.shared.activateFileViewerSelecting([folder.appendingPathComponent(name)])
        }))
        banner.hide(after: 6)
    }

    func download(_ download: WKDownload, didFailWithError error: Error, resumeData: Data?) {
        downloads[ObjectIdentifier(download)] = nil
        showBanner("Download failed: \(error.localizedDescription)", .error)
    }

    private var downloads: [ObjectIdentifier: URL] {
        get { BrowserDownloads.byTile[tileID] ?? [:] }
        set { BrowserDownloads.byTile[tileID] = newValue }
    }

    /// ~/Downloads, unless automation pointed downloads elsewhere.
    static var downloadsFolder: URL {
        BrowserDownloads.folderOverride
            ?? FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("Downloads")
    }

    nonisolated static func freeName(_ name: String, in folder: URL) -> URL {
        let base = (name as NSString).deletingPathExtension
        let ext = (name as NSString).pathExtension
        var candidate = folder.appendingPathComponent(name.isEmpty ? "download" : name)
        var n = 2
        while FileManager.default.fileExists(atPath: candidate.path) {
            let next = ext.isEmpty ? "\(base) \(n)" : "\(base) \(n).\(ext)"
            candidate = folder.appendingPathComponent(next)
            n += 1
        }
        return candidate
    }

    // Escape in the address bar restores the page's URL.
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        guard control === address, selector == #selector(NSResponder.cancelOperation(_:)) else { return false }
        handle(chord: "escape")
        return true
    }
}

/// Downloads in flight, per tile (extensions can't hold stored properties).
@MainActor
private enum BrowserDownloads {
    static var byTile: [String: [ObjectIdentifier: URL]] = [:]
    /// Set by automation so test downloads never touch ~/Downloads.
    static var folderOverride: URL?
}

// MARK: - Per-workspace website data

@MainActor
enum BrowserStores {
    private static var stores: [String: WKWebsiteDataStore] = [:]

    /// The persistent store for a workspace, made once and shared by its
    /// browser tiles. Tiles with no known workspace use the default store.
    static func store(forWorkspace workspace: String?) -> WKWebsiteDataStore {
        guard let workspace else { return .default() }
        if let store = stores[workspace] { return store }
        let store = WKWebsiteDataStore(forIdentifier: uuid(for: workspace))
        stores[workspace] = store
        return store
    }

    /// A stable UUID from the workspace ID (SHA-256, version 5 layout), so
    /// the same workspace finds its cookies again after a relaunch.
    static func uuid(for workspace: String) -> UUID {
        var bytes = Array(SHA256.hash(data: Data(("azulejo.browser." + workspace).utf8)).prefix(16))
        bytes[6] = (bytes[6] & 0x0F) | 0x50
        bytes[8] = (bytes[8] & 0x3F) | 0x80
        return UUID(uuid: (bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                           bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]))
    }
}

// MARK: - Banner

/// A one-line notice over the page: failed loads, crashes, downloads.
@MainActor
final class BrowserBanner: NSView {
    enum Style { case info, error }

    private let label = NSTextField(labelWithString: "")
    private let button = NSButton(title: "", target: nil, action: nil)
    private var action: (() -> Void)?
    private var hideWork: DispatchWorkItem?

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = 6
        label.font = .systemFont(ofSize: 12)
        label.lineBreakMode = .byTruncatingTail
        button.bezelStyle = .rounded
        button.controlSize = .small
        button.target = self
        button.action = #selector(run)
        addSubview(label)
        addSubview(button)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    var text: String { label.stringValue }

    func show(_ text: String, style: Style, action: (String, () -> Void)?) {
        hideWork?.cancel()
        label.stringValue = text
        label.textColor = style == .error ? .white : .labelColor
        layer?.backgroundColor = (style == .error ? NSColor.systemRed : NSColor.controlBackgroundColor)
            .withAlphaComponent(0.95).cgColor
        layer?.borderWidth = style == .error ? 0 : 1
        layer?.borderColor = NSColor.separatorColor.cgColor
        self.action = action?.1
        button.title = action?.0 ?? ""
        button.isHidden = action == nil
        isHidden = false
        needsLayout = true
    }

    func hide(after seconds: Double? = nil) {
        hideWork?.cancel()
        guard let seconds else { isHidden = true; return }
        let work = DispatchWorkItem { [weak self] in self?.isHidden = true }
        hideWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + seconds, execute: work)
    }

    override func layout() {
        super.layout()
        let buttonWidth: CGFloat = button.isHidden ? 0 : 70
        button.frame = NSRect(x: bounds.width - buttonWidth - 6, y: (bounds.height - 22) / 2, width: buttonWidth, height: 22)
        label.frame = NSRect(x: 10, y: (bounds.height - 16) / 2, width: bounds.width - buttonWidth - 22, height: 16)
    }

    @objc private func run() { action?() }
}

// MARK: - Automation

extension BrowserTileView: AutomatableTile {
    /// `tile <verb> [arg]` in automation scripts (Automation.swift).
    func automate(_ verb: String, _ arg: String) -> String? {
        switch verb {
        case "key":
            // The router's verdict first (must let browser chords through),
            // then the tile's own handler, as performKeyEquivalent would.
            let routed = app?.core.key(chord: arg, tile: keyContext)["outcome"] as? String ?? "?"
            if routed == "action" || routed == "prefix_armed" { return "router consumed \(arg)" }
            return handle(chord: arg) ? nil : "not a browser chord: \(arg)"
        case "type":
            // Fill the address bar as if typed, then press enter.
            focusAddress()
            address.stringValue = arg
            addressEntered()
            return nil
        case "js":
            webView.evaluateJavaScript(arg) { [weak self] value, error in
                MainActor.assumeIsolated {
                    self?.lastScriptResult = error.map { "error: \($0.localizedDescription)" }
                        ?? value.map { "\($0)" } ?? "nil"
                }
            }
            return nil
        case "snapshot":
            let out = arg
            webView.takeSnapshot(with: nil) { image, error in
                guard let image, let tiff = image.tiffRepresentation,
                      let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:])
                else {
                    FileHandle.standardError.write(Data("snapshot failed: \(String(describing: error))\n".utf8))
                    return
                }
                try? png.write(to: URL(fileURLWithPath: out))
            }
            return nil
        case "downloads-dir":
            try? FileManager.default.createDirectory(atPath: arg, withIntermediateDirectories: true)
            BrowserDownloads.folderOverride = URL(fileURLWithPath: arg, isDirectory: true)
            return nil
        case "crash":
            // Simulate the web content process dying.
            webViewWebContentProcessDidTerminate(webView)
            return nil
        default:
            return "unknown browser verb \(verb)"
        }
    }

    func automationState() -> [String: String] {
        [
            "tile.kind": "browser",
            "tile.url": webView.url?.absoluteString ?? "-",
            "tile.core_url": coreURL ?? "-",
            "tile.title": webView.title.flatMap { $0.isEmpty ? nil : $0 } ?? "-",
            "tile.loading": webView.isLoading ? "yes" : "no",
            "tile.back": webView.canGoBack ? "yes" : "no",
            "tile.forward": webView.canGoForward ? "yes" : "no",
            "tile.inspectable": webView.isInspectable ? "yes" : "no",
            "tile.inspector": Self.inspector(of: webView) != nil ? "yes" : "no",
            "tile.banner": banner.isHidden ? "-" : banner.text,
            "tile.address_focused": isEditingAddress ? "yes" : "no",
            "tile.store": webView.configuration.websiteDataStore.identifier?.uuidString ?? "default",
            "tile.js": lastScriptResult ?? "-",
            "tile.pending": "\(pendingEchoes.count)",
        ]
    }

    private var lastScriptResult: String? {
        get { BrowserScriptResults.byTile[tileID] }
        set { BrowserScriptResults.byTile[tileID] = newValue }
    }
}

@MainActor
private enum BrowserScriptResults {
    static var byTile: [String: String] = [:]
}
