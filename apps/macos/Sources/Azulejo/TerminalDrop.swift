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
@MainActor
enum TerminalDrop {
    /// Image data a drop writes out as a PNG.
    static let imageTypes: [NSPasteboard.PasteboardType] =
        [UTType.png, .tiff, .jpeg, .gif, .heic, .webP, .bmp].map { NSPasteboard.PasteboardType($0.identifier) }

    /// What a terminal tile registers for.
    static let types: [NSPasteboard.PasteboardType] =
        [.fileURL, .URL, .string] + imageTypes
        + NSFilePromiseReceiver.readableDraggedTypes.map { NSPasteboard.PasteboardType($0) }

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
///     drop [@N] hover KIND ARG…     the drag only enters (any kind above)
///     drop [@N] leave               …and leaves
///
/// `@N` is the tile's place in the tab (`expect focus`); the focused tile
/// by default. Keys: `drop.op` (what the last hover or drop got:
/// `copy`, `none`, `dropped`, `refused`), `drop.highlight` (places showing
/// the highlight, `2` or `none`), `drop.last` (the text the last drop typed),
/// `drop.dir` (where images go).
@MainActor
enum DropAutomation {
    private static var lastOp = "-"
    private(set) static var lastText = "-"

    static func noteDropped(_ text: String) { lastText = text }

    static func run(_ arg: String, controller: WindowController, tab: TabModel?) -> String? {
        var words = arg.split(separator: " ", maxSplits: 1).map(String.init)
        var place: Int?
        if let first = words.first, first.hasPrefix("@") {
            place = Int(first.dropFirst())
            words = words.count > 1 ? words[1].split(separator: " ", maxSplits: 1).map(String.init) : []
        }
        guard let tab else { return "no tab" }
        let id = place.map { $0 >= 1 && $0 <= tab.order.count ? tab.order[$0 - 1] : "" } ?? tab.focused
        guard let tile = controller.window.contentView?.terminalTile(id) else { return "no terminal tile there" }
        let verb = words.first ?? "", rest = words.count > 1 ? words[1] : ""
        switch verb {
        case "leave":
            tile.draggingExited(nil)
            lastOp = "left"
            return nil
        case "hover":
            let bits = rest.split(separator: " ", maxSplits: 1).map(String.init)
            guard let pasteboard = pasteboard(bits.first ?? "", bits.count > 1 ? bits[1] : "") else {
                return "drop hover files|text|custom|url|image|web-image ARG"
            }
            lastOp = tile.draggingEntered(StandInDrag(pasteboard)) == .copy ? "copy" : "none"
            return nil
        default:
            guard let pasteboard = pasteboard(verb, rest) else {
                return "drop [@N] files|text|custom|url|image|web-image|hover|leave ARG"
            }
            let drag = StandInDrag(pasteboard)
            guard tile.draggingEntered(drag) == .copy, tile.prepareForDragOperation(drag) else {
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
    private static func pasteboard(_ kind: String, _ arg: String) -> NSPasteboard? {
        let pasteboard = NSPasteboard(name: NSPasteboard.Name("dev.azulejo.automation.drop"))
        pasteboard.clearContents()
        switch kind {
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
        return [
            "drop.op": lastOp,
            "drop.highlight": places.isEmpty ? "none" : places.joined(separator: ","),
            "drop.last": lastText,
            "drop.dir": TerminalDrop.directory.path,
        ]
    }
}

/// A drag as far as a drop destination sees one: its pasteboard, a copy.
@MainActor
private final class StandInDrag: NSObject, @preconcurrency NSDraggingInfo {
    let draggingPasteboard: NSPasteboard
    init(_ pasteboard: NSPasteboard) { draggingPasteboard = pasteboard }

    var draggingDestinationWindow: NSWindow? { nil }
    var draggingSourceOperationMask: NSDragOperation { .copy }
    var draggingLocation: NSPoint { .zero }
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
