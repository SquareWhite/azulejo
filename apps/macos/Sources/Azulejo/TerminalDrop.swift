import AppKit
import ImageIO
import UniformTypeIdentifiers

/// Drag and drop onto a terminal tile, the way Ghostty.app and kitty do it:
/// what a drop types into the terminal.
///
/// - **Files** (any kind, any number): their absolute paths, each escaped
///   the way Ghostty.app escapes them (a backslash before
///   `\ ()[]{}<>"'`!#$&;|*?` and tab), joined by spaces. Claude Code reads a
///   pasted image path as an image to attach.
/// - **Image data with no file** (an image from a web page): written to a
///   PNG in `directory`, then that path.
/// - **Promised files** (Photos, Mail, a browser's image with no data on
///   the drag): received into `directory`, then their paths. Taken when
///   they're images, or when the drag offers nothing else.
/// - **Text and URLs**: as they are.
///
/// It all goes in as one paste (`TerminalSurfaceView.pasteText`), framed as
/// a bracketed paste when the program asked for that. A drop reads only the
/// drag's own pasteboard: the general pasteboard (Claude Code's ctrl+v image
/// paste) is never read or written.
///
/// A drag of files splits the tile in two (`DropZonesView`) when the pane's
/// folder is known: the top half pastes as above, the bottom half moves the
/// files into that folder (⌥: copies them) through `FileTransfer`, an
/// upload with scp when the pane is on a herdr machine. Nothing is typed
/// then; a HUD line says how it went.
@MainActor
enum TerminalDrop {
    /// Image data a drop writes out as a PNG.
    static let imageTypes: [NSPasteboard.PasteboardType] =
        [UTType.png, .tiff, .jpeg, .gif, .heic, .webP, .bmp].map { NSPasteboard.PasteboardType($0.identifier) }

    /// What a terminal tile registers for.
    static let types: [NSPasteboard.PasteboardType] =
        [.fileURL, .URL, .string] + imageTypes
        + NSFilePromiseReceiver.readableDraggedTypes.map { NSPasteboard.PasteboardType($0) }
        + [FilesDragging.remoteType]

    /// Entries dragged out of a remote machine's file tree, as a terminal
    /// on `machine` (nil: Local) types them: their remote paths, escaped and
    /// joined by spaces. Nil when the drag has none from that machine (a
    /// terminal elsewhere receives their promises: downloaded, then the
    /// local copies' paths).
    static func remotePaths(_ pasteboard: NSPasteboard, on machine: String?) -> String? {
        let refs = FilesDragging.remoteRefs(pasteboard)
        guard let machine, machine != MachineModel.local, !refs.isEmpty,
              refs.allSatisfy({ $0.machine == machine })
        else { return nil }
        return refs.map { escape($0.path.precomposedStringWithCanonicalMapping) }.joined(separator: " ")
    }

    /// Where images and promised files go: a folder of our own in the
    /// per-user temp folder. `sweep` empties it of anything a day old.
    nonisolated static var directory: URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("azulejo-drops", isDirectory: true)
    }

    /// Whether a drag carries anything a terminal takes (no data read: the
    /// drag is only hovering).
    static func accepts(_ pasteboard: NSPasteboard) -> Bool {
        pasteboard.canReadItem(withDataConformingToTypes: types.map(\.rawValue))
    }

    /// The files a drag carries (file URLs, no promises), in order; empty
    /// for anything else. A drag with files splits the tile into two drop
    /// zones (`DropZonesView`): paste the paths, or move the files into the
    /// terminal's folder.
    static func fileURLs(_ pasteboard: NSPasteboard) -> [URL] {
        let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true])
        return (urls as? [URL] ?? []).filter(\.isFileURL)
    }

    /// What a drop does: type this text, or receive promised files first.
    enum Plan {
        case paste(String)
        case receive([NSFilePromiseReceiver])
        case nothing
    }

    /// Reads the drop. Images with no file are written out here.
    static func plan(_ pasteboard: NSPasteboard) -> Plan {
        let items = pasteboard.pasteboardItems ?? []
        let promises = pasteboard.readObjects(forClasses: [NSFilePromiseReceiver.self]) as? [NSFilePromiseReceiver] ?? []
        if !promises.isEmpty, !items.contains(where: { fileURL($0) != nil || image($0) != nil }) {
            let images = promises.allSatisfy { receiver in
                !receiver.fileTypes.isEmpty
                    && receiver.fileTypes.allSatisfy { UTType($0)?.conforms(to: .image) == true }
            }
            if images || !items.contains(where: { text($0) != nil }) { return .receive(promises) }
        }
        let parts = items.compactMap { part(of: $0) }
        return parts.isEmpty ? .nothing : .paste(parts.joined(separator: " "))
    }

    /// One pasteboard item as text: a file's path, an image written out, or
    /// the item's text.
    private static func part(of item: NSPasteboardItem) -> String? {
        if let url = fileURL(item) { return typed(url) }
        if let found = image(item), let url = writeImage(found.0, type: found.1) { return typed(url) }
        return text(item)
    }

    private static func fileURL(_ item: NSPasteboardItem) -> URL? {
        guard let plist = item.propertyList(forType: .fileURL),
              let url = NSURL(pasteboardPropertyList: plist, ofType: .fileURL) as URL?, url.isFileURL
        else { return nil }
        return url
    }

    /// The item's image data, when the image is what it's about: alone, or
    /// with the URL it came from (a picture dragged out of a web page). Text
    /// that carries a picture of itself (a selection in a word processor)
    /// stays text.
    private static func image(_ item: NSPasteboardItem) -> (Data, NSPasteboard.PasteboardType)? {
        guard let type = item.availableType(from: imageTypes) else { return nil }
        if let text = item.string(forType: .string), !text.isEmpty, item.availableType(from: [.URL]) == nil,
           URL(string: text.trimmingCharacters(in: .whitespacesAndNewlines))?.scheme == nil {
            return nil
        }
        return item.data(forType: type).map { ($0, type) }
    }

    private static func text(_ item: NSPasteboardItem) -> String? {
        for type in [NSPasteboard.PasteboardType.string, .URL] {
            if let text = item.string(forType: type), !text.isEmpty { return text }
        }
        return nil
    }

    /// A file's path as a drop types it: escaped, and composed (NFC). A
    /// file URL's path comes out decomposed (`u` + combining diaeresis for
    /// `ü`); the file opens either way, but the composed form is the one
    /// shells, Claude Code and people read as the name they see.
    nonisolated static func typed(_ url: URL) -> String {
        escape(url.path.precomposedStringWithCanonicalMapping)
    }

    /// Ghostty.app's `Ghostty.Shell.escape`: a backslash before every
    /// character a shell would read as syntax. Fine for pasting a path at a
    /// prompt or into Claude Code, which takes the escaped form.
    nonisolated static func escape(_ path: String) -> String {
        var out = ""
        for ch in path {
            if "\\ ()[]{}<>\"'`!#$&;|*?\t".contains(ch) { out.append("\\") }
            out.append(ch)
        }
        return out
    }

    /// The image as a PNG of its own in `directory`.
    static func writeImage(_ data: Data, type: NSPasteboard.PasteboardType) -> URL? {
        let png: Data?
        if type == NSPasteboard.PasteboardType(UTType.png.identifier) {
            png = data
        } else if let source = CGImageSourceCreateWithData(data as CFData, nil),
                  let image = CGImageSourceCreateImageAtIndex(source, 0, nil) {
            png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
        } else {
            png = nil
        }
        guard let png, let folder = makeDirectory() else { return nil }
        let url = folder.appendingPathComponent("\(stamp()).png")
        do {
            try png.write(to: url, options: .atomic)
            return url
        } catch {
            return nil
        }
    }

    /// Receives promised files into a folder of their own, then hands back
    /// their escaped paths (nil when none arrived). Gives up waiting after
    /// a minute with what's there.
    static func receive(_ promises: [NSFilePromiseReceiver], then done: @escaping @MainActor (String?) -> Void) {
        guard let base = makeDirectory() else { return done(nil) }
        let folder = base.appendingPathComponent(stamp(), isDirectory: true)
        guard (try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)) != nil
        else { return done(nil) }
        let batch = PromiseBatch(expected: promises.reduce(0) { $0 + max(1, $1.fileTypes.count) }, done: done)
        let queue = OperationQueue()
        queue.qualityOfService = .userInitiated
        for (index, receiver) in promises.enumerated() {
            receiver.receivePromisedFiles(atDestination: folder, options: [:], operationQueue: queue) { url, error in
                batch.arrived(error == nil ? url : nil, order: index)
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 60) { batch.finish() }
    }

    /// Promised files as they come in, from the receivers' queue.
    private final class PromiseBatch: @unchecked Sendable {
        private let lock = NSLock()
        private var expected: Int
        private var urls: [(order: Int, url: URL)] = []
        private var finished = false
        private let done: @MainActor (String?) -> Void

        init(expected: Int, done: @escaping @MainActor (String?) -> Void) {
            self.expected = expected
            self.done = done
        }

        func arrived(_ url: URL?, order: Int) {
            let complete: Bool = lock.withLock {
                if let url { urls.append((order, url)) }
                expected -= 1
                return expected <= 0
            }
            if complete { DispatchQueue.main.async { self.finish() } }
        }

        func finish() {
            let paths: [String]? = lock.withLock {
                guard !finished else { return nil }
                finished = true
                return urls.sorted { $0.order < $1.order }.map { TerminalDrop.typed($0.url) }
            }
            guard let paths else { return }
            let done = self.done
            MainActor.assumeIsolated { done(paths.isEmpty ? nil : paths.joined(separator: " ")) }
        }
    }

    private static func makeDirectory() -> URL? {
        let folder = directory
        do {
            try FileManager.default.createDirectory(
                at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            return folder
        } catch {
            return nil
        }
    }

    /// `drop-20261105-142301-1a2b3c`: sorts by time, never clashes.
    private static func stamp() -> String {
        let format = DateFormatter()
        format.locale = Locale(identifier: "en_US_POSIX")
        format.dateFormat = "yyyyMMdd-HHmmss"
        let tail = UUID().uuidString.prefix(6).lowercased()
        return "drop-\(format.string(from: Date()))-\(tail)"
    }

    /// Removes what earlier drops left in `directory` more than `age`
    /// seconds ago (at launch, off the main thread): Claude Code reads an
    /// image when it's pasted, and a day leaves time to paste it again.
    nonisolated static func sweep(olderThan age: TimeInterval = 24 * 60 * 60) {
        let fm = FileManager.default
        let cutoff = Date().addingTimeInterval(-age)
        guard let entries = try? fm.contentsOfDirectory(
            at: directory, includingPropertiesForKeys: [.contentModificationDateKey], options: [])
        else { return }
        for entry in entries {
            let modified = (try? entry.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate
            if let modified, modified < cutoff { try? fm.removeItem(at: entry) }
        }
    }
}

/// What a drop's HUD line is about: a transfer still running (the line
/// stays), done, or failed (it stays longer, to be read).
enum DropNotice {
    case running, done, failed
}

/// A terminal tile's look while a drag it would take hovers over it: a faint
/// wash and a thin border in the theme's accent. Never takes a click.
final class DropHighlightView: NSView, ThemeAware {
    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.borderWidth = Design.Border.drop
        layer?.cornerRadius = Design.Radius.overlay
        autoresizingMask = [.width, .height]
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func applyTheme() {
        layer?.backgroundColor = Design.Color.accent.withAlphaComponent(Design.Alpha.dropFill).cgColor
        layer?.borderColor = Design.Color.accent.withAlphaComponent(Design.Alpha.dropBorder).cgColor
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// A terminal tile's look while a drag of files hovers over it, the way
/// Telegram splits a chat: the tile goes under `color.overlay` and two
/// stacked cards share it. The top one pastes the files' paths (what any
/// drop does); the bottom one moves them into the terminal's folder, or
/// copies them with ⌥ held. With no folder known, one card pastes. Each
/// card says what it does in two centred lines, and the one under the
/// pointer lights up. Fades in, and the highlight cross-fades, over
/// `motion.dropZone` (instant with motion off or Reduce Motion). Never
/// takes a click.
final class DropZonesView: NSView, ThemeAware {
    enum Zone: String {
        case paste, move
    }

    private let cards: [Zone: DropZoneCard] = [.paste: DropZoneCard(), .move: DropZoneCard()]
    /// The cards showing, top first.
    private(set) var zones: [Zone] = []
    /// The card under the pointer.
    private(set) var hot: Zone?

    override var isFlipped: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        autoresizingMask = [.width, .height]
        for zone in [Zone.paste, .move] {
            guard let card = cards[zone] else { continue }
            card.layer.cornerRadius = Design.Radius.dropZone
            card.layer.cornerCurve = .continuous
            layer?.insertSublayer(card.layer, at: 0)
            card.hint.font = Design.Font.dropZoneHint
            card.action.font = Design.Font.dropZoneAction
            for label in [card.hint, card.action] {
                label.alignment = .center
                label.lineBreakMode = .byTruncatingMiddle
                label.cell?.truncatesLastVisibleLine = true
                addSubview(label)
            }
        }
        Theme.track(self)
        configure(paste: "paste path", move: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// What the cards say: `move` nil shows the paste card alone.
    func configure(paste: String, move: String?) {
        let zones: [Zone] = move == nil ? [.paste] : [.paste, .move]
        cards[.paste]?.action.stringValue = paste
        if let move { cards[.move]?.action.stringValue = move }
        guard zones != self.zones else { return }
        self.zones = zones
        let moveShown = zones.contains(.move)
        if let card = cards[.move] {
            card.layer.isHidden = !moveShown
            card.hint.isHidden = !moveShown
            card.action.isHidden = !moveShown
        }
        if let hot, !zones.contains(hot) { self.hot = nil }
        needsLayout = true
    }

    /// The overlay fades in (at once with motion off).
    func appear() {
        alphaValue = 0
        guard Motion.active else { return alphaValue = 1 }
        NSAnimationContext.runAnimationGroup { context in
            context.duration = Design.Motion.dropZone
            animator().alphaValue = 1
        }
    }

    /// The zone a drop at `point` (this view's coordinates) lands in: the
    /// top half pastes, the bottom half moves.
    func zone(at point: NSPoint) -> Zone {
        zones.count > 1 && point.y >= bounds.midY ? .move : .paste
    }

    func setHot(_ zone: Zone?) {
        guard zone != hot else { return }
        hot = zone
        restyle(animated: Motion.active)
    }

    /// A card's frame (top-left origin), for automation.
    func frame(of zone: Zone) -> NSRect? {
        guard zones.contains(zone) else { return nil }
        return cards[zone]?.layer.frame
    }

    /// What a card says, its two lines joined by `|`.
    func label(of zone: Zone) -> String? {
        guard zones.contains(zone), let card = cards[zone] else { return nil }
        return "\(card.hint.stringValue)|\(card.action.stringValue)"
    }

    override func layout() {
        super.layout()
        let inset = Design.Space.dropZoneInset, gap = Design.Space.dropZoneGap
        let area = bounds.insetBy(dx: inset, dy: inset)
        var frames: [Zone: NSRect] = [.paste: area]
        if zones.count > 1 {
            let height = max(0, (area.height - gap) / 2)
            frames[.paste] = NSRect(x: area.minX, y: area.minY, width: area.width, height: height)
            frames[.move] = NSRect(x: area.minX, y: area.minY + height + gap, width: area.width, height: height)
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for (zone, frame) in frames {
            guard let card = cards[zone] else { continue }
            card.layer.frame = frame
            let width = max(0, frame.width - 2 * inset)
            let hint = card.hint.intrinsicContentSize.height
            let action = card.action.intrinsicContentSize.height
            let top = frame.midY - (hint + Design.Space.dropZoneLabelGap + action) / 2
            card.hint.frame = NSRect(x: frame.minX + inset, y: top, width: width, height: hint)
            card.action.frame = NSRect(
                x: frame.minX + inset, y: top + hint + Design.Space.dropZoneLabelGap, width: width, height: action)
        }
        CATransaction.commit()
    }

    func applyTheme() {
        layer?.backgroundColor = Design.Color.overlay.cgColor
        for card in cards.values {
            card.hint.textColor = Design.Color.textMuted
            card.action.textColor = Design.Color.textPrimary
        }
        restyle(animated: false)
    }

    private func restyle(animated: Bool) {
        // theme: applyTheme calls it, and so does every change of the hot card.
        CATransaction.begin()
        CATransaction.setDisableActions(!animated)
        CATransaction.setAnimationDuration(Design.Motion.dropZone)
        for (zone, card) in cards {
            let hot = zone == self.hot
            card.layer.backgroundColor = Design.Color.accent
                .withAlphaComponent(hot ? Design.Alpha.dropZoneHot : Design.Alpha.dropZoneFill).cgColor
            card.layer.borderColor = Design.Color.accent
                .withAlphaComponent(hot ? Design.Alpha.dropBorder : Design.Alpha.dropZoneBorder).cgColor
            card.layer.borderWidth = hot ? Design.Border.drop : Design.Border.dropZone
        }
        CATransaction.commit()
    }

    /// Automation: how the cards sit, e.g. `stacked inset=12 gap=8` (paste
    /// over move, the same size, the same inset all round) or
    /// `single inset=12`.
    var geometry: String {
        guard let paste = frame(of: .paste) else { return "none" }
        let inset = paste.minX
        let even = { (r: NSRect) in
            abs(r.minX - inset) < 0.5 && abs(self.bounds.maxX - r.maxX - inset) < 0.5
        }
        guard even(paste), abs(paste.minY - inset) < 0.5 else { return "uneven \(paste)" }
        guard let move = frame(of: .move) else {
            return abs(bounds.maxY - paste.maxY - inset) < 0.5 ? "single inset=\(Int(inset))" : "uneven \(paste)"
        }
        guard even(move), abs(bounds.maxY - move.maxY - inset) < 0.5, abs(move.height - paste.height) < 0.5,
              move.minY > paste.maxY
        else { return "uneven \(paste) \(move)" }
        return "stacked inset=\(Int(inset)) gap=\(Int((move.minY - paste.maxY).rounded()))"
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// One of `DropZonesView`'s cards: its fill and outline, and two lines.
@MainActor
private final class DropZoneCard {
    let layer = CALayer()
    let hint = NSTextField(labelWithString: "Drop to")
    let action = NSTextField(labelWithString: "")
}

// MARK: - Automation

/// Automation verbs for drops (`drop …`): a private pasteboard, never the
/// general one, handed to a terminal tile's real drag-destination methods
/// through a stand-in for the drag.
///
///     drop [@N] files PATH[|PATH…]  file URLs, one pasteboard item each
///     drop [@N] text TEXT           plain text (`\n`: a newline)
///     drop [@N] custom TEXT         a type no terminal takes
///     drop [@N] url URL             a web URL (public.url and its text)
///     drop [@N] image PATH          the image's bytes alone, under its own
///                                   type (png, tiff…), no file URL
///     drop [@N] web-image PATH URL  the image's bytes with the URL it came
///                                   from (a picture out of a web page)
///     drop [@N] tree PATH[|PATH…]   what dragging those entries out of the
///                                   sidebar's Files view carries (paths
///                                   relative to its root)
///     drop [@N] hover KIND ARG…     the drag only enters (any kind above)
///     drop [@N] leave               …and leaves
///     drop [@N] at FX,FY …          where the drag is, as fractions of the
///                                   tile from its top left (0.5,0.25 by
///                                   default: the top half, which pastes)
///     drop [@N] copy …              ⌥ held: the drag's mask narrows to copy
///
/// `@N` is the tile's place in the tab (`expect focus`); the focused tile
/// by default. Keys: `drop.op` (what the last hover or drop got:
/// `copy`, `move`, `none`, `dropped`, `refused`), `drop.highlight` (places
/// showing the highlight, `2` or `none`), `drop.last` (the text the last
/// drop typed), `drop.dir` (where images go). A drag of files shows drop
/// zones instead of the highlight: `drop.zones` (`paste,move`, `paste` or
/// `none`), `drop.zone` (the one under the pointer), `drop.label.paste` and
/// `drop.label.move` (two lines joined by `|`), `drop.geometry` (`stacked
/// inset=12 gap=8`). A drop on the move zone transfers the files:
/// `drop.transfer` (`running`, `done` or `failed`), `drop.transfer.text`
/// (the HUD line), `drop.transfer.dests` (where they went, names joined by
/// `|`).
@MainActor
enum DropAutomation {
    private static var lastOp = "-"
    private(set) static var lastText = "-"
    private static var transfer = "-"
    private static var transferText = "-"
    private static var transferDests = "-"

    static func noteDropped(_ text: String) { lastText = text }

    /// A drop's transfer started (nil) or finished.
    static func noteTransfer(_ done: (result: FileTransfer.Result, text: String, failed: Bool)?) {
        guard let done else {
            (transfer, transferText, transferDests) = ("running", "-", "-")
            return
        }
        transfer = done.failed ? "failed" : "done"
        transferText = done.text
        let dests = done.result.items.compactMap(\.dest).map { ($0 as NSString).lastPathComponent }
        transferDests = dests.isEmpty ? "-" : dests.joined(separator: "|")
    }

    static func run(_ arg: String, controller: WindowController, tab: TabModel?) -> String? {
        var words = arg.split(separator: " ", maxSplits: 1).map(String.init)
        var place: Int?
        var at = NSPoint(x: 0.5, y: 0.25)
        var copy = false
        func shift() {
            words = words.count > 1 ? words[1].split(separator: " ", maxSplits: 1).map(String.init) : []
        }
        while let first = words.first {
            if first.hasPrefix("@") {
                place = Int(first.dropFirst())
            } else if first == "copy" {
                copy = true
            } else if first == "at" {
                shift()
                let xy = (words.first ?? "").split(separator: ",").compactMap { Double($0) }
                guard xy.count == 2 else { return "drop at FX,FY: fractions of the tile" }
                at = NSPoint(x: xy[0], y: xy[1])
            } else {
                break
            }
            shift()
        }
        guard let tab else { return "no tab" }
        let id = place.map { $0 >= 1 && $0 <= tab.order.count ? tab.order[$0 - 1] : "" } ?? tab.focused
        guard let tile = controller.window.contentView?.terminalTile(id) else { return "no terminal tile there" }
        // The tile isn't flipped: y counts from the bottom.
        let point = NSPoint(x: tile.bounds.width * at.x, y: tile.bounds.height * (1 - at.y))
        let location = tile.convert(point, to: nil)
        let mask: NSDragOperation = copy ? .copy : [.copy, .move, .link, .generic]
        let verb = words.first ?? "", rest = words.count > 1 ? words[1] : ""
        switch verb {
        case "leave":
            tile.draggingExited(nil)
            lastOp = "left"
            return nil
        case "hover":
            let bits = rest.split(separator: " ", maxSplits: 1).map(String.init)
            guard let pasteboard = pasteboard(bits.first ?? "", bits.count > 1 ? bits[1] : "", controller) else {
                return "drop hover files|text|custom|url|image|web-image ARG"
            }
            lastOp = name(tile.draggingEntered(StandInDrag(pasteboard, location: location, mask: mask)))
            return nil
        default:
            guard let pasteboard = pasteboard(verb, rest, controller) else {
                return "drop [@N] [at FX,FY] [copy] files|text|custom|url|image|web-image|tree|hover|leave ARG"
            }
            let drag = StandInDrag(pasteboard, location: location, mask: mask)
            guard tile.draggingEntered(drag) != [], tile.prepareForDragOperation(drag) else {
                tile.draggingExited(drag)
                lastOp = "refused"
                return nil
            }
            lastOp = tile.performDragOperation(drag) ? "dropped" : "refused"
            tile.concludeDragOperation(drag)
            tile.draggingEnded(drag)
            return nil
        }
    }

    /// A private pasteboard with the drag's contents.
    private static func pasteboard(_ kind: String, _ arg: String, _ controller: WindowController) -> NSPasteboard? {
        let pasteboard = NSPasteboard(name: NSPasteboard.Name("dev.azulejo.automation.drop"))
        pasteboard.clearContents()
        switch kind {
        case "tree":
            guard let panel = controller.sidebarView.filesPanel else { return nil }
            let writers = arg.split(separator: "|").compactMap { panel.dragWriter(forPath: String($0)) }
            guard !writers.isEmpty else { return nil }
            pasteboard.writeObjects(writers)
        case "files":
            let urls = arg.split(separator: "|").map { URL(fileURLWithPath: String($0)) as NSURL }
            pasteboard.writeObjects(urls)
        case "text":
            pasteboard.setString(arg.replacingOccurrences(of: "\\n", with: "\n"), forType: .string)
        case "custom":
            // Something only the app itself would drag (a tile, say).
            pasteboard.setString(arg, forType: NSPasteboard.PasteboardType("dev.azulejo.automation.private"))
        case "url":
            guard let url = URL(string: arg) else { return nil }
            pasteboard.writeObjects([url as NSURL])
        case "image", "web-image":
            let bits = arg.split(separator: " ", maxSplits: 1).map(String.init)
            let file = URL(fileURLWithPath: bits.first ?? "")
            guard let data = try? Data(contentsOf: file),
                  let type = UTType(filenameExtension: file.pathExtension) else { return nil }
            let item = NSPasteboardItem()
            item.setData(data, forType: NSPasteboard.PasteboardType(type.identifier))
            if kind == "web-image" {
                guard bits.count > 1 else { return nil }
                item.setString(bits[1], forType: .URL)
                item.setString(bits[1], forType: .string)
            }
            pasteboard.writeObjects([item])
        default:
            return nil
        }
        return pasteboard
    }

    static func state(controller: WindowController, tab: TabModel?) -> [String: String] {
        var places: [String] = []
        for (i, id) in (tab?.order ?? []).enumerated()
        where controller.window.contentView?.terminalTile(id)?.showsDropHighlight == true {
            places.append("\(i + 1)")
        }
        let zones = (tab?.order ?? []).lazy
            .compactMap { controller.window.contentView?.terminalTile($0)?.dropZonesShown }.first
        return [
            "drop.op": lastOp,
            "drop.highlight": places.isEmpty ? "none" : places.joined(separator: ","),
            "drop.last": lastText,
            "drop.dir": TerminalDrop.directory.path,
            "drop.zones": zones.map { $0.zones.map(\.rawValue).joined(separator: ",") } ?? "none",
            "drop.zone": zones?.hot?.rawValue ?? "none",
            "drop.label.paste": zones?.label(of: .paste) ?? "-",
            "drop.label.move": zones?.label(of: .move) ?? "-",
            "drop.geometry": zones?.geometry ?? "none",
            "drop.transfer": transfer,
            "drop.transfer.text": transferText,
            "drop.transfer.dests": transferDests,
        ]
    }

    private static func name(_ op: NSDragOperation) -> String {
        op.contains(.move) ? "move" : op.contains(.copy) ? "copy" : "none"
    }
}

/// A drag as far as a drop destination sees one: its pasteboard, where it
/// is (window coordinates) and what its source allows.
@MainActor
final class StandInDrag: NSObject, @preconcurrency NSDraggingInfo {
    let draggingPasteboard: NSPasteboard
    let draggingLocation: NSPoint
    let draggingSourceOperationMask: NSDragOperation
    init(_ pasteboard: NSPasteboard, location: NSPoint, mask: NSDragOperation) {
        draggingPasteboard = pasteboard
        draggingLocation = location
        draggingSourceOperationMask = mask
    }

    var draggingDestinationWindow: NSWindow? { nil }
    var draggedImageLocation: NSPoint { .zero }
    var draggedImage: NSImage? { nil }
    var draggingSource: Any? { nil }
    var draggingSequenceNumber: Int { 0 }
    var draggingFormation: NSDraggingFormation = .default
    var animatesToDestination = false
    var numberOfValidItemsForDrop = 0
    var springLoadingHighlight: NSSpringLoadingHighlight { .none }
    func slideDraggedImage(to screenPoint: NSPoint) {}
    func enumerateDraggingItems(
        options enumOpts: NSDraggingItemEnumerationOptions, for view: NSView?, classes classArray: [AnyClass],
        searchOptions: [NSPasteboard.ReadingOptionKey: Any],
        using block: (NSDraggingItem, Int, UnsafeMutablePointer<ObjCBool>) -> Void
    ) {}
    func resetSpringLoading() {}
}

extension NSView {
    /// The terminal tile for a tile ID, anywhere under this view.
    func terminalTile(_ id: String) -> TerminalTileView? {
        if let tile = self as? TerminalTileView, tile.tileID == id { return tile }
        for sub in subviews { if let found = sub.terminalTile(id) { return found } }
        return nil
    }
}
