import AppKit
import UniformTypeIdentifiers

/// What the sidebar's Files view does to entries, beyond opening them:
///
/// - **New File / New Folder**: a name field in the target folder (the
///   clicked or selected folder, a file's folder, else the root), made by the
///   core's `files.create`, which never replaces anything. The new entry is
///   selected once its folder's listing shows it.
/// - **Delete**: Local entries go to the Trash at once (`files.delete`;
///   Put Back works). A remote machine's are removed for good, so a bar in
///   the panel asks first (Return confirms, Esc cancels).
/// - **Copy** puts the entries on the pasteboard as files (a remote
///   machine's are downloaded into an app-owned temp folder first, through
///   `files.transfer`). **Paste** copies files on the pasteboard into the
///   target folder, uploading them for a remote workspace. Both use the
///   pasteboard terminals use: the general one, or a private one in
///   automation runs.
/// - **Drops** from Finder or elsewhere land in the folder under the
///   pointer (the root on empty space), the way Finder does it: moved on
///   the same volume, copied across volumes, ⌥ copies, ⌘ moves. A remote
///   workspace always uploads. Promised files (Mail, Photos) are received
///   into the folder.
/// - **Drags out** carry Local entries as file URLs and a remote machine's
///   as file promises that download on drop, plus their remote paths
///   (`FilesDragging.remoteType`), which a terminal on the same machine
///   types.
///
/// Everything runs in the core off the main thread; how it went shows as
/// a HUD line (`notice`), never an alert.
extension FilesPanelView {
    /// How an operation's outcome is shown: a HUD line in the panel's
    /// window. Set by the app at launch.
    static var notice: (NSWindow?, String, DropNotice) -> Void = { _, _, _ in }

    /// A tile's command line, by id, for automation (`sidebar.files.editor`).
    /// Set by the app at launch.
    static var tileCommand: (String) -> String? = { _ in nil }

    /// Where Copy and Paste go: the system pasteboard, or the private one
    /// in automation runs.
    static var pasteboard: NSPasteboard { TerminalRuntime.pasteboard }

    /// Says how an operation went, in the HUD and for automation.
    func say(_ text: String, _ kind: DropNotice) {
        lastOp = text
        lastOpState = switch kind {
        case .running: "running"
        case .done: "done"
        case .failed: "failed"
        }
        Self.log("files: \(Short.id(panelID)) \(lastOpState): \(text)")
        Self.notice(window, text, kind)
    }

    /// Automation runs only ever change files in a temp folder: a script
    /// whose view isn't on its fixtures (a step that didn't land) must not
    /// touch the user's files. True when the operation may go ahead.
    func mayChangeFiles() -> Bool {
        guard ContextMenus.automating else { return true }
        // `/var` and `/tmp` are `/private/…` resolved: compare without it.
        let plain = { (path: String) -> String in
            let path = (path as NSString).standardizingPath
            return path.hasPrefix("/private/") ? String(path.dropFirst("/private".count)) : path
        }
        let temp = [FileManager.default.temporaryDirectory.path, NSTemporaryDirectory(), "/tmp"].map(plain)
        let rootPath = plain(root.path)
        if !rootPath.isEmpty, temp.contains(where: { rootPath.hasPrefix($0 + "/") }) { return true }
        say("Automation runs only change files in a temp folder", .failed)
        return false
    }

    /// The machine's name in messages, nil for Local.
    var machineLabel: String? {
        machine.map { TerminalTileView.machineLabels[$0] ?? $0 }
    }

    /// This panel's folder for `FileTransfer`.
    func transferFolder(_ folder: FileNode) -> FileTransfer.Folder {
        FileTransfer.Folder(path: folder.path, machine: machine, label: machineLabel)
    }

    // MARK: Selection

    /// The selected entries, top to bottom.
    var selectedNodes: [FileNode] {
        outline.selectedRowIndexes.compactMap { outline.item(atRow: $0) as? FileNode }.filter { !$0.isDraft }
    }

    /// Where new entries and pasted files go for `node`: a folder itself, a
    /// file's folder, the root for nothing.
    func targetFolder(for node: FileNode?) -> FileNode {
        guard let node, !node.isDraft, root.dir, !root.path.isEmpty else { return root }
        if node.dir { return node }
        return node.parent ?? root
    }

    /// Opens `folder` (not the root), so what lands there shows.
    func reveal(_ folder: FileNode) {
        guard folder !== root, !outline.isItemExpanded(folder) else { return }
        var chain: [FileNode] = []
        var at: FileNode? = folder
        while let node = at, node !== root {
            chain.append(node)
            at = node.parent
        }
        for node in chain.reversed() { outline.expandItem(node) }
    }

    // MARK: Reloads

    /// `reloadData`, keeping a name field that's being typed in.
    func reloadAll() {
        reloadKeepingDraft { outline.reloadData() }
    }

    /// Runs a reload of the outline; a name field it takes away and puts
    /// back gets the keyboard again.
    func reloadKeepingDraft(_ body: () -> Void) {
        guard let draft else { return body() }
        let typing = draft.isTyping
        draft.reloading = true
        body()
        draft.reloading = false
        if typing { focusDraft() }
    }

    /// Listings changed: select what was waiting for its listing.
    func listingsChanged() {
        guard !pendingSelect.isEmpty else { return }
        if CACurrentMediaTime() - pendingSince > 15 {
            pendingSelect = []
            return
        }
        let found = pendingSelect.compactMap { nodes[$0] }
        let rows = IndexSet(found.map { outline.row(forItem: $0) }.filter { $0 >= 0 })
        guard let first = rows.first else { return }
        pendingSelect = []
        outline.selectRowIndexes(rows, byExtendingSelection: false)
        outline.scrollRowToVisible(first)
    }

    // MARK: New file, new folder

    /// Shows a name field for a new file or folder in `folder`.
    func beginCreate(_ kind: FileDraft.Kind, in folder: FileNode) {
        guard !root.path.isEmpty, mayChangeFiles() else { return }
        if !query.isEmpty { clearFilter() }
        hideConfirmation()
        if draft != nil { removeDraft() }
        let node = FileNode(path: (folder.path as NSString).appendingPathComponent("\u{1}new"), name: "",
                            dir: kind == .folder)
        node.isDraft = true
        node.parent = folder
        let draft = FileDraft(kind: kind, parent: folder, node: node, panel: self)
        self.draft = draft
        reloadKeepingDraft { outline.reloadItem(folder === root ? nil : folder, reloadChildren: true) }
        reveal(folder)
        let row = outline.row(forItem: node)
        if row >= 0 { outline.scrollRowToVisible(row) }
        draft.isTyping = true
        focusDraft()
    }

    /// Hands the keyboard to the name field once its row is on screen.
    func focusDraft(tries: Int = 10) {
        draft?.focusPending = true
        DispatchQueue.main.async { [weak self] in
            guard let self, let draft = self.draft else { return }
            guard !draft.busy, let window = self.window else { return draft.focusPending = false }
            // After a reload the outline makes the row's view when it lays
            // out: make it now, or come back once it's there.
            if draft.field.window !== window { self.outline.layoutSubtreeIfNeeded() }
            guard draft.field.window === window else {
                guard tries > 0 else { return draft.focusPending = false }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { self.focusDraft(tries: tries - 1) }
                return
            }
            draft.focusPending = false
            if let editor = window.firstResponder as? NSTextView, editor.delegate === draft.field { return }
            window.makeFirstResponder(draft.field)
            // The caret after what's typed, not all of it selected.
            if let editor = draft.field.currentEditor() {
                editor.selectedRange = NSRange(location: (editor.string as NSString).length, length: 0)
            }
        }
    }

    /// Why `name` can't be a new entry in `folder`, or nil. The core checks
    /// again (a name taken meanwhile, a case-insensitive file system).
    static func nameProblem(_ name: String, in folder: FileNode) -> String? {
        if name.trimmingCharacters(in: .whitespaces).isEmpty { return "A name can't be empty" }
        if name.contains("/") { return "A name can't contain “/”" }
        if name.contains("\u{0}") { return "A name can't contain a NUL" }
        if name == "." || name == ".." { return "“\(name)” can't be a name" }
        if name.utf8.count > 255 { return "That name is too long" }
        if (folder.children ?? []).contains(where: { $0.name == name }) { return "“\(name)” is already there" }
        return nil
    }

    /// Return in the name field: makes the entry (the field stays, busy,
    /// until the core says how it went). A name that can't be made keeps
    /// the field for another go; when the keyboard went elsewhere
    /// (`refocus` false) the field goes instead, and the keyboard stays
    /// where the user put it.
    func commitDraft(refocus: Bool = true) {
        guard let draft, !draft.busy, mayChangeFiles() else { return }
        let name = draft.field.stringValue
        if let problem = Self.nameProblem(name, in: draft.parent) {
            say(problem, .failed)
            if refocus {
                draft.isTyping = true
                focusDraft()
            } else {
                removeDraft()
            }
            return
        }
        guard let core = Self.core else { return }
        draft.busy = true
        let parent = draft.parent
        let path = (parent.path as NSString).appendingPathComponent(name)
        let kind = draft.kind
        core.executeAsync(
            ["method": "files.create",
             "params": ["tile": panelID, "path": path, "kind": kind == .folder ? "folder" : "file"]],
            decode: { result in result.map { $0["path"] as? String ?? path } },
            then: { [weak self] (result: Result<String, CoreError>) in
                guard let self, self.draft === draft else { return }
                switch result {
                case .success(let made):
                    self.pendingSelect = [made]
                    self.removeDraft()
                    if refocus { self.window?.makeFirstResponder(self.outline) }
                    let what = kind == .folder ? "folder" : "file"
                    self.say("Made the \(what) \(name)", .done)
                    self.changed([parent.path])
                case .failure(let error):
                    self.say("Couldn't make \(name): \(error.message)", .failed)
                    guard refocus else { return self.removeDraft() }
                    draft.busy = false
                    draft.isTyping = true
                    self.focusDraft()
                }
            })
    }

    /// Esc in the name field: nothing is made.
    func cancelDraft() {
        guard draft != nil else { return }
        removeDraft()
        window?.makeFirstResponder(outline)
    }

    /// The name field lost the keyboard elsewhere (a click): a name made so
    /// far is made, an empty one goes.
    func draftLostFocus() {
        guard let draft, !draft.busy else { return }
        if draft.field.stringValue.trimmingCharacters(in: .whitespaces).isEmpty {
            removeDraft()
        } else {
            commitDraft(refocus: false)
        }
    }

    /// Takes the name field's row away.
    func removeDraft() {
        guard let draft else { return }
        let parent = draft.parent
        draft.reloading = true
        self.draft = nil
        outline.reloadItem(parent === root ? nil : parent, reloadChildren: true)
        if parent !== root, parent.open { outline.expandItem(parent) }
        draft.reloading = false
    }

    // MARK: Delete

    func deleteSelection() {
        delete(selectedNodes)
    }

    /// Local entries go to the Trash; a remote machine's are deleted for
    /// good once the bar's Delete (or Return) says so.
    func delete(_ list: [FileNode]) {
        let list = list.filter { $0 !== root && !$0.isDraft }
        guard !list.isEmpty, mayChangeFiles() else { return }
        guard isRemote else { return runDelete(list) }
        let what = list.count == 1 ? "“\(list[0].name)”" : "\(list.count) items"
        showConfirmation("Delete \(what) from \(machineLabel ?? "the machine")? This can't be undone.",
                         button: "Delete") { [weak self] in
            self?.runDelete(list)
        }
    }

    private func runDelete(_ list: [FileNode]) {
        guard let core = Self.core, mayChangeFiles() else { return }
        // A test never fills the user's Trash: automation deletes go to a
        // folder of its own (the core's AZJ_TRASH_DIR) or nowhere.
        if !isRemote, ContextMenus.automating,
           ProcessInfo.processInfo.environment["AZJ_TRASH_DIR"]?.isEmpty ?? true {
            return say("Automation runs delete into AZJ_TRASH_DIR; it isn't set", .failed)
        }
        // What to select afterwards: the row below the last one going, else
        // the one above the first.
        let rows = list.map { outline.row(forItem: $0) }.filter { $0 >= 0 }
        let gone = Set(list.map { ObjectIdentifier($0) })
        let after = ((rows.max() ?? -1) + 1..<outline.numberOfRows).lazy
            .compactMap { self.outline.item(atRow: $0) as? FileNode }
            .first { !gone.contains(ObjectIdentifier($0)) && !$0.isDraft }
        let before = (0..<(rows.min() ?? 0)).reversed().lazy
            .compactMap { self.outline.item(atRow: $0) as? FileNode }
            .first { !gone.contains(ObjectIdentifier($0)) }
        let neighbour = (after ?? before)?.path
        let paths = list.map(\.path)
        let parents = Array(Set(list.compactMap { $0.parent?.path }))
        let remote = isRemote, label = machineLabel ?? "the machine"
        let name = { (path: String) in (path as NSString).lastPathComponent }
        if remote {
            say("Deleting \(list.count == 1 ? name(paths[0]) : "\(list.count) items") from \(label)…", .running)
        }
        core.executeAsync(
            ["method": "files.delete", "params": ["tile": panelID, "paths": paths]],
            decode: { result in DeleteResult(result) },
            then: { [weak self] result in
                guard let self else { return }
                if let failure = result.failure {
                    self.say("Couldn't delete: \(failure)", .failed)
                    return
                }
                let failed = result.items.filter { $0.error != nil }
                let done = result.items.count - failed.count
                if let first = failed.first {
                    let more = failed.count > 1 ? " (and \(failed.count - 1) more)" : ""
                    self.say("Couldn't delete \(name(first.path))\(more): \(first.error ?? "")", .failed)
                } else {
                    let what = done == 1 ? name(paths[0]) : "\(done) items"
                    self.say(remote ? "Deleted \(what) from \(label)" : "Moved \(what) to the Trash", .done)
                }
                if let neighbour { self.pendingSelect = [neighbour] }
                self.changed(parents)
            })
    }

    func showConfirmation(_ text: String, button: String, action: @escaping () -> Void) {
        confirmBar.show(text, button: button)
        confirmBar.onConfirm = { [weak self] in
            self?.hideConfirmation()
            action()
        }
        confirmBar.onCancel = { [weak self] in self?.hideConfirmation() }
        confirmBar.isHidden = false
        confirmHeight.constant = FilesConfirmBar.height
        lastOp = "asked: \(text)"
        lastOpState = "asking"
    }

    func hideConfirmation() {
        guard confirmBar.isShowing else { return }
        confirmBar.onConfirm = nil
        confirmBar.onCancel = nil
        confirmBar.isHidden = true
        confirmHeight.constant = 0
        if lastOpState == "asking" { lastOpState = "-" }
    }

    // MARK: Copy, paste

    func copySelection() {
        copy(selectedNodes)
    }

    /// The entries onto the pasteboard as files; a remote machine's are
    /// downloaded into a temp folder of the app's own first.
    func copy(_ list: [FileNode]) {
        let list = list.filter { !$0.isDraft && $0 !== root }
        guard !list.isEmpty else { return }
        let name = { (path: String) in (path as NSString).lastPathComponent }
        let what = list.count == 1 ? list[0].name : "\(list.count) items"
        guard isRemote, let machine else {
            write(list.map { URL(fileURLWithPath: $0.path) })
            return say("Copied \(what)", .done)
        }
        guard let core = Self.core else { return }
        guard let folder = RemoteCopies.newFolder() else {
            return say("Couldn't make a folder to download into", .failed)
        }
        let label = machineLabel ?? "the machine"
        say("Downloading \(what) from \(label)…", .running)
        FileTransfer.run(list.map(\.path), from: machine, into: FileTransfer.Folder(path: folder.path), mode: .copy,
                         core: core) { [weak self] result in
            guard let self else { return }
            if let failure = result.failure { return self.say("Couldn't download \(what): \(failure)", .failed) }
            let urls = result.items.compactMap(\.dest).map { URL(fileURLWithPath: $0) }
            if !urls.isEmpty { self.write(urls) }
            if let first = result.items.first(where: { $0.error != nil }) {
                self.say("Couldn't download \(name(first.source)): \(first.error ?? "")", .failed)
            } else {
                self.say("Copied \(what) from \(label)", .done)
            }
        }
    }

    func write(_ urls: [URL]) {
        let pasteboard = Self.pasteboard
        pasteboard.clearContents()
        pasteboard.writeObjects(urls.map { $0 as NSURL })
    }

    func pasteIntoSelection() {
        paste(into: targetFolder(for: selectedNode))
    }

    /// Files on the pasteboard, copied into `folder` (uploaded on a remote
    /// machine). Never overwrites: a taken name gets Finder's number.
    func paste(into folder: FileNode) {
        let urls = FilesDragging.fileURLs(Self.pasteboard)
        guard !urls.isEmpty else { return say("Nothing to paste: copy some files first", .failed) }
        transfer(urls, into: folder, mode: .copy)
    }

    /// Moves or copies local files into `folder` through the core.
    func transfer(_ urls: [URL], into folder: FileNode, mode: FileTransfer.Mode) {
        guard let core = Self.core, !root.path.isEmpty, mayChangeFiles() else { return }
        reveal(folder)
        let dest = transferFolder(folder)
        let paths = urls.map(\.path)
        if dest.isRemote || paths.count > 1 { say(FileTransfer.progress(paths.count, into: dest, mode: mode), .running) }
        let sources = Array(Set(paths.map { ($0 as NSString).deletingLastPathComponent }))
        FileTransfer.run(paths, into: dest, mode: mode, core: core) { [weak self] result in
            guard let self else { return }
            let (text, failed) = FileTransfer.summary(result, into: dest, mode: mode)
            self.say(text, failed ? .failed : .done)
            self.pendingSelect = result.items.compactMap(\.dest)
            self.changed([folder.path] + (mode == .move ? sources : []))
        }
    }

    // MARK: Dragging out

    /// What a drag of `node` carries: a Local file's URL, or a remote one's
    /// promise (downloaded where it's dropped) and remote path.
    func dragWriter(for node: FileNode) -> (any NSPasteboardWriting)? {
        guard let machine else { return URL(fileURLWithPath: node.path) as NSURL }
        let type = node.dir ? UTType.folder : (UTType(filenameExtension: (node.name as NSString).pathExtension) ?? .data)
        let promise = RemoteFilePromise(fileType: type.identifier, delegate: RemotePromises.shared)
        promise.remote = RemoteFileRef(machine: machine, path: node.path)
        return promise
    }

    // MARK: Dropping in

    /// The folder a drop on `node` lands in: a folder itself, a file's
    /// folder, the root on empty space.
    func dropFolder(for node: FileNode?) -> FileNode {
        targetFolder(for: node)
    }

    /// What a drag over `folder` would do: `.move` or `.copy` with files
    /// (Finder's rules locally, always a copy to a remote machine), `.copy`
    /// with promised files, nothing otherwise.
    func dropOperation(_ info: any NSDraggingInfo, into folder: FileNode) -> NSDragOperation {
        guard !root.path.isEmpty else { return [] }
        let pasteboard = info.draggingPasteboard
        // Entries dragged out of this machine's own tree: moving them
        // around on the machine isn't supported.
        if let machine, FilesDragging.remoteRefs(pasteboard).contains(where: { $0.machine == machine }) { return [] }
        let urls = FilesDragging.fileURLs(pasteboard)
        if !urls.isEmpty {
            // Into itself, or where they all are already: nothing to do.
            if !isRemote {
                if urls.contains(where: { folder.path == $0.path || folder.path.hasPrefix($0.path + "/") }) { return [] }
                if urls.allSatisfy({ ($0.path as NSString).deletingLastPathComponent == folder.path }) { return [] }
            }
            return dropMode(info, urls, into: folder) == .move ? .move : .copy
        }
        let promises = pasteboard.canReadObject(forClasses: [NSFilePromiseReceiver.self], options: nil)
        return promises ? .copy : []
    }

    /// Finder's choice for a drop of local files: ⌥ copies, ⌘ moves,
    /// otherwise the same volume moves and another copies. Uploads copy.
    func dropMode(_ info: any NSDraggingInfo, _ urls: [URL], into folder: FileNode) -> FileTransfer.Mode {
        if isRemote { return .copy }
        let mask = info.draggingSourceOperationMask
        // Automation never reads the keys the user holds right now.
        let flags: NSEvent.ModifierFlags = ContextMenus.automating ? [] : NSEvent.modifierFlags
        if flags.contains(.option) || (mask.contains(.copy) && !mask.contains(.move) && !mask.contains(.generic)) {
            return .copy
        }
        if flags.contains(.command) || (!mask.contains(.copy) && (mask.contains(.move) || mask.contains(.generic))) {
            return .move
        }
        return Self.sameVolume(urls, folder.path) ? .move : .copy
    }

    static func sameVolume(_ urls: [URL], _ folder: String) -> Bool {
        func volume(_ url: URL) -> NSObject? {
            (try? url.resourceValues(forKeys: [.volumeIdentifierKey]))?.volumeIdentifier as? NSObject
        }
        guard let target = volume(URL(fileURLWithPath: folder)) else { return false }
        return urls.allSatisfy { volume($0)?.isEqual(target) == true }
    }

    /// The drop: files moved or copied in, promised files received.
    func acceptDrop(_ info: any NSDraggingInfo, into folder: FileNode) -> Bool {
        guard !dropOperation(info, into: folder).isEmpty else { return false }
        let pasteboard = info.draggingPasteboard
        let urls = FilesDragging.fileURLs(pasteboard)
        if !urls.isEmpty {
            transfer(urls, into: folder, mode: dropMode(info, urls, into: folder))
            return true
        }
        let promises = pasteboard.readObjects(forClasses: [NSFilePromiseReceiver.self]) as? [NSFilePromiseReceiver] ?? []
        guard !promises.isEmpty else { return false }
        receive(promises, into: folder)
        return true
    }

    /// Promised files: written straight into a Local folder; for a remote
    /// one, into a temp folder and then uploaded.
    private func receive(_ promises: [NSFilePromiseReceiver], into folder: FileNode) {
        guard mayChangeFiles() else { return }
        let local = !isRemote
        guard let destination = local ? URL(fileURLWithPath: folder.path) : RemoteCopies.newFolder() else {
            return say("Couldn't make a folder to receive into", .failed)
        }
        reveal(folder)
        say("Receiving \(promises.count == 1 ? "a file" : "\(promises.count) files")…", .running)
        let batch = ReceivedFiles(expected: promises.count) { [weak self] urls in
            guard let self else { return }
            guard !urls.isEmpty else { return self.say("Nothing arrived", .failed) }
            if local {
                self.pendingSelect = urls.map(\.path)
                self.say("Received \(urls.count == 1 ? urls[0].lastPathComponent : "\(urls.count) files")", .done)
                self.changed([folder.path])
            } else {
                self.transfer(urls, into: folder, mode: .copy)
            }
        }
        let queue = OperationQueue()
        queue.qualityOfService = .userInitiated
        for receiver in promises {
            receiver.receivePromisedFiles(atDestination: destination, options: [:], operationQueue: queue) { url, error in
                batch.arrived(error == nil ? url : nil)
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 120) { batch.finish() }
    }

    // MARK: Menu

    /// The right-click menu for `node` (with the selection it's part of),
    /// or for the root on empty space.
    func contextMenu(for node: FileNode?) -> NSMenu {
        let menu = NSMenu()
        let chords = Self.core.map { ContextMenus.catalogueChords(core: $0) } ?? [:]
        func add(_ title: String, _ action: String? = nil, enabled: Bool = true, _ run: @escaping () -> Void) {
            let item = ContextMenuItem(title, enabled: enabled, run: run)
            if let action, let chord = chords[action] {
                item.actionName = action
                item.chord = chord
                MainMenu.apply(chord, to: item)
            }
            menu.addItem(item)
        }
        let folder = targetFolder(for: node)
        let canPaste = !FilesDragging.fileURLs(Self.pasteboard).isEmpty
        guard let node else {
            // The root's.
            add("Open Folder in Editor", "files_open_folder_in_editor") { [weak self] in self?.editFolder(nil) }
            menu.addItem(.separator())
            add("New File…", "files_new_file") { [weak self] in self?.beginCreate(.file, in: folder) }
            add("New Folder…", "files_new_folder") { [weak self] in self?.beginCreate(.folder, in: folder) }
            menu.addItem(.separator())
            add("Paste", "files_paste", enabled: canPaste) { [weak self] in self?.paste(into: folder) }
            menu.addItem(.separator())
            add("Show or Hide Hidden Files", "files_toggle_hidden") { [weak self] in self?.toggleHidden() }
            return menu
        }
        let selection = selectedNodes.contains { $0 === node } ? selectedNodes : [node]
        if node.dir {
            add("Open Folder in Editor", "files_open_folder_in_editor") { [weak self] in self?.editFolder(node) }
        } else {
            add("Open in Editor", "files_open") { [weak self] in self?.edit(node) }
        }
        add("Open Terminal Here") { [weak self] in self?.openTerminal(node) }
        // A remote path isn't on this Mac.
        if !isRemote {
            add("Open in Default App", "files_open_external") { [weak self] in self?.openInDefaultApp(node) }
            add("Reveal in Finder") { [weak self] in self?.revealInFinder(node) }
        }
        menu.addItem(.separator())
        add("New File…", "files_new_file") { [weak self] in self?.beginCreate(.file, in: folder) }
        add("New Folder…", "files_new_folder") { [weak self] in self?.beginCreate(.folder, in: folder) }
        menu.addItem(.separator())
        add("Copy", "files_copy") { [weak self] in self?.copy(selection) }
        add("Paste", "files_paste", enabled: canPaste) { [weak self] in self?.paste(into: folder) }
        add("Copy Path") { [weak self] in self?.copyPath(node) }
        menu.addItem(.separator())
        add(isRemote ? "Delete…" : "Move to Trash", "files_delete") { [weak self] in self?.delete(selection) }
        menu.addItem(.separator())
        add("Show or Hide Hidden Files", "files_toggle_hidden") { [weak self] in self?.toggleHidden() }
        return menu
    }

    /// The listed entry at `path` (relative to the root, or absolute).
    func node(forPath path: String) -> FileNode? {
        if path == "." { return root }
        return nodes[path.hasPrefix("/") ? path : (root.path as NSString).appendingPathComponent(path)]
    }

    /// What a right-click on `node` selects (automation builds the menu
    /// without one): it joins the selection only if it was part of it.
    func selectForMenu(_ node: FileNode) {
        let row = outline.row(forItem: node)
        guard row >= 0, !outline.selectedRowIndexes.contains(row) else { return }
        outline.selectRowIndexes([row], byExtendingSelection: false)
    }

    /// What dragging the entry at `path` out carries (automation).
    func dragWriter(forPath path: String) -> (any NSPasteboardWriting)? {
        node(forPath: path).flatMap { $0 === root ? nil : dragWriter(for: $0) }
    }

    /// Where a right-click on `node` opens its menu, in window coordinates.
    func menuAnchor(for node: FileNode?) -> NSRect {
        guard let node else { return scrollFrameInWindow }
        let row = outline.row(forItem: node)
        guard row >= 0 else { return scrollFrameInWindow }
        return outline.convert(outline.rect(ofRow: row), to: nil)
    }

    private var scrollFrameInWindow: NSRect {
        outline.convert(outline.visibleRect, to: nil)
    }
}

// MARK: - Automation

extension FilesPanelView {
    /// The file operations' verbs (`sidebar files <verb> [arg]`), paths
    /// relative to the root:
    ///
    ///     new-file|new-folder [PATH]   a name field in PATH's folder (`.` the
    ///                                  root; the selection's when left out)
    ///     name TEXT                    type into the name field
    ///     commit | cancel              Return or Esc in it
    ///     select-many P1|P2            select several entries
    ///     delete | copy | paste        what ⌘⌫, ⌘C and ⌘V do
    ///     clip-files P1|P2             put files (absolute paths) on the
    ///                                  pasteboard Paste reads
    ///     confirm yes|no               answer the remote delete bar
    ///     drop [copy|move] [on PATH] files P1|P2
    ///                                  drop files (absolute paths) on the
    ///                                  tree through its drop methods; `copy`
    ///                                  holds ⌥, `move` holds ⌘
    ///     drop-hover [on PATH] files P1|P2   only hover (`drop-target`)
    ///     promise PATH DIR             what a remote entry's promise does
    ///                                  when it's dropped in DIR
    func automateOperation(_ verb: String, _ arg: String) -> String? {
        let node = { (path: String) -> FileNode? in self.node(forPath: path) }
        switch verb {
        case "new-file", "new-folder":
            let kind: FileDraft.Kind = verb == "new-file" ? .file : .folder
            if arg.isEmpty {
                beginCreate(kind, in: targetFolder(for: selectedNode))
            } else {
                guard let at = node(arg) else { return "\(arg) isn't listed" }
                beginCreate(kind, in: at === root ? root : targetFolder(for: at))
            }
        case "name":
            guard let draft else { return "no name field" }
            draft.field.stringValue = arg
            if let editor = draft.field.currentEditor() { editor.string = arg }
        case "commit":
            guard draft != nil else { return "no name field" }
            commitDraft()
        case "cancel":
            guard draft != nil else { return "no name field" }
            cancelDraft()
        case "select-many":
            let found = arg.split(separator: "|").map(String.init).compactMap(node)
            let rows = IndexSet(found.map { outline.row(forItem: $0) }.filter { $0 >= 0 })
            guard !rows.isEmpty else { return "none of \(arg) is listed" }
            outline.selectRowIndexes(rows, byExtendingSelection: false)
        case "clip-files":
            // Files on the pasteboard Paste reads, as Finder's Copy leaves them.
            write(arg.split(separator: "|").map { URL(fileURLWithPath: String($0)) })
        case "delete": deleteSelection()
        case "copy": copySelection()
        case "paste": pasteIntoSelection()
        case "confirm":
            guard confirmBar.isShowing else { return "nothing to confirm" }
            if arg == "yes" { confirmBar.confirm() } else { confirmBar.cancel() }
        case "drop", "drop-hover":
            var words = arg.split(separator: " ").map(String.init)
            var mask: NSDragOperation = [.copy, .move, .link, .generic]
            var on: FileNode?
            while let first = words.first, ["copy", "move", "on"].contains(first) {
                words.removeFirst()
                switch first {
                case "copy": mask = .copy
                case "move": mask = .generic
                default:
                    guard let path = words.first, let at = node(path) else { return "drop on PATH: not listed" }
                    on = at === root ? nil : at
                    words.removeFirst()
                }
            }
            guard words.first == "files", words.count == 2 else { return "drop [copy|move] [on PATH] files P1|P2" }
            let pasteboard = NSPasteboard(name: NSPasteboard.Name("dev.azulejo.automation.files-drop"))
            pasteboard.clearContents()
            pasteboard.writeObjects(words[1].split(separator: "|").map { URL(fileURLWithPath: String($0)) as NSURL })
            let drag = StandInDrag(pasteboard, location: .zero, mask: mask)
            let op = outlineView(outline, validateDrop: drag, proposedItem: on, proposedChildIndex: -1)
            lastDropOp = op.contains(.move) ? "move" : op.contains(.copy) ? "copy" : "none"
            guard verb == "drop" else { return nil }
            if op.isEmpty { lastDropOp = "refused"; dropTarget = nil; return nil }
            _ = outlineView(outline, acceptDrop: drag, item: on, childIndex: -1)
        case "promise":
            let bits = arg.split(separator: " ", maxSplits: 1).map(String.init)
            guard bits.count == 2, let at = node(bits[0]) else { return "promise PATH DIR" }
            guard let writer = dragWriter(for: at) as? RemoteFilePromise else { return "\(bits[0]) isn't remote" }
            lastPromise = "running"
            let url = URL(fileURLWithPath: bits[1]).appendingPathComponent(at.name)
            RemotePromises.shared.filePromiseProvider(writer, writePromiseTo: url) { error in
                DispatchQueue.main.async { [weak self] in
                    self?.lastPromise = error.map { "failed: \($0.localizedDescription)" } ?? "done"
                }
            }
        default:
            return "unknown files verb \(verb)"
        }
        return nil
    }

    /// Keys for the operations (`sidebar.files.*`).
    func operationState() -> [String: String] {
        let relative = { (path: String) -> String in
            if path == self.root.path { return "." }
            return path.hasPrefix(self.root.path + "/") ? String(path.dropFirst(self.root.path.count + 1)) : path
        }
        let copied = FilesDragging.fileURLs(Self.pasteboard).map(\.lastPathComponent)
        var s: [String: String] = [
            "draft": draft.map { $0.kind == .folder ? "folder" : "file" } ?? "none",
            "draft.in": draft.map { relative($0.parent.path) } ?? "-",
            "draft.busy": draft?.busy == true ? "yes" : "no",
            "op": lastOp,
            "op.state": lastOpState,
            "confirm": confirmBar.isShowing ? confirmBar.text.stringValue : "-",
            "selection": selectedNodes.map { relative($0.path) }.joined(separator: "|").nonEmpty ?? "-",
            "drop-target": dropTarget.map(relative) ?? "-",
            "drop-op": lastDropOp,
            "editor": lastEditor + (lastEditorTile.map { ":" + (Self.tileCommand($0) ?? "?") } ?? ""),
            "editor.app": EditorApp.app.isEmpty ? "-" : EditorApp.app,
            "editor.command": EditorApp.command,
            "opened": EditorApp.opened.last ?? "-",
            "pasteboard": copied.isEmpty ? "-" : copied.joined(separator: "|"),
            "promise": lastPromise,
        ]
        if let draft, let window {
            let editor = window.firstResponder as? NSTextView
            s["draft.focused"] = editor?.delegate === draft.field ? "yes" : "no"
        } else {
            s["draft.focused"] = "-"
        }
        return s
    }
}

private extension String {
    var nonEmpty: String? { isEmpty ? nil : self }
}

// MARK: - The name field

/// A new entry's name, typed in its folder's row: the field, its folder
/// and its placeholder node.
@MainActor
final class FileDraft: NSObject, NSTextFieldDelegate {
    enum Kind { case file, folder }

    let kind: Kind
    let parent: FileNode
    let node: FileNode
    let field = DraftField()
    private weak var panel: FilesPanelView?
    private var view: DraftCell?
    /// The core is making it: the field waits.
    var busy = false
    /// A reload is taking the field away and putting it back: losing the
    /// keyboard then isn't the user leaving it.
    var reloading = false
    /// The field should have the keyboard (it's being typed in).
    var isTyping = false
    /// `focusDraft` is about to give the field the keyboard back.
    var focusPending = false

    init(kind: Kind, parent: FileNode, node: FileNode, panel: FilesPanelView) {
        self.kind = kind
        self.parent = parent
        self.node = node
        self.panel = panel
        super.init()
        field.delegate = self
        field.placeholderString = kind == .folder ? "Folder name" : "File name"
    }

    /// The row's view: always the same one, so a reload keeps what's typed.
    func cell(icon: NSImage) -> NSView {
        let cell = view ?? DraftCell(field: field)
        view = cell
        cell.icon.image = icon
        return cell
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.insertNewline(_:)):
            panel?.commitDraft()
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            panel?.cancelDraft()
            return true
        default:
            return false
        }
    }

    func controlTextDidEndEditing(_ obj: Notification) {
        guard !reloading, !busy else { return }
        let movement = (obj.userInfo?["NSTextMovement"] as? Int).flatMap(NSTextMovement.init(rawValue:))
        guard movement != .return, movement != .cancel else { return }
        // Only when the keyboard really went elsewhere: the notification
        // can land after the field took it back (a reload, a failed name).
        DispatchQueue.main.async { [weak self] in
            guard let self, !self.reloading, !self.busy, !self.focusPending, let window = self.field.window
            else { return }
            if let editor = window.firstResponder as? NSTextView, editor.delegate === self.field { return }
            self.isTyping = false
            self.panel?.draftLostFocus()
        }
    }
}

/// The name field. Its keys are a text field's: `find_bar`, whose actions
/// it declines, so Return and Esc reach the field (`FileDraft`).
final class DraftField: NSTextField, KeyScope {
    var keyScope: String { "find_bar" }
    func perform(keyAction name: String, event: NSEvent?) -> Bool { false }
}

/// The name field's row: the entry's icon, then the field on a pill.
final class DraftCell: NSTableCellView, ThemeAware {
    let icon = NSImageView()
    private let field: DraftField
    /// The pill the field sits in: the find field's fill, an accent outline.
    private let pill = NSView()

    init(field: DraftField) {
        self.field = field
        super.init(frame: .zero)
        imageView = icon
        field.font = Design.Font.row
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.lineBreakMode = .byTruncatingHead
        field.cell?.isScrollable = true
        field.cell?.wraps = false
        pill.wantsLayer = true
        pill.layer?.cornerRadius = Design.Radius.row
        pill.layer?.borderWidth = 1
        for v in [icon, pill, field] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
        }
        addSubview(icon)
        addSubview(pill)
        pill.addSubview(field)
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 2),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 16),
            icon.heightAnchor.constraint(equalToConstant: 16),
            pill.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 2),
            pill.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            pill.centerYAnchor.constraint(equalTo: centerYAnchor),
            pill.heightAnchor.constraint(equalToConstant: 19),
            // The name lines up with the names above and below it.
            field.leadingAnchor.constraint(equalTo: pill.leadingAnchor, constant: 1),
            field.trailingAnchor.constraint(equalTo: pill.trailingAnchor, constant: -4),
            field.centerYAnchor.constraint(equalTo: pill.centerYAnchor),
        ])
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func applyTheme() {
        field.textColor = Design.Color.textPrimary
        pill.layer?.backgroundColor = Design.Color.findField.cgColor
        pill.layer?.borderColor = Design.Color.accent.cgColor
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        applyTheme()
    }
}

// MARK: - The confirmation bar

/// Asks before a remote delete, in the panel: what's going and two
/// buttons. Return confirms and Esc cancels while the tree has the keyboard
/// (`FilesPanelView.perform(keyAction:)`).
final class FilesConfirmBar: NSView, ThemeAware {
    static let height: CGFloat = 70

    let text = NSTextField(wrappingLabelWithString: "")
    private let cancelButton = NSButton(title: "Cancel", target: nil, action: nil)
    private let confirmButton = NSButton(title: "Delete", target: nil, action: nil)
    var onConfirm: (() -> Void)?
    var onCancel: (() -> Void)?

    var isShowing: Bool { !isHidden }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.field
        layer?.borderWidth = 1
        text.font = Design.Font.detail
        text.maximumNumberOfLines = 2
        text.lineBreakMode = .byWordWrapping
        text.cell?.truncatesLastVisibleLine = true
        for button in [cancelButton, confirmButton] {
            button.bezelStyle = .rounded
            button.controlSize = .small
            button.font = Design.Font.detail
            button.target = self
        }
        cancelButton.action = #selector(cancelPressed)
        confirmButton.action = #selector(confirmPressed)
        confirmButton.hasDestructiveAction = true
        for v in [text, cancelButton, confirmButton] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            text.topAnchor.constraint(equalTo: topAnchor, constant: 6),
            text.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            text.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            confirmButton.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            confirmButton.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -6),
            cancelButton.trailingAnchor.constraint(equalTo: confirmButton.leadingAnchor, constant: -4),
            cancelButton.centerYAnchor.constraint(equalTo: confirmButton.centerYAnchor),
        ])
        Theme.track(self)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func show(_ message: String, button: String) {
        text.stringValue = message
        text.toolTip = message
        confirmButton.title = button
    }

    func confirm() { onConfirm?() }
    func cancel() { onCancel?() }

    @objc private func confirmPressed() { confirm() }
    @objc private func cancelPressed() { cancel() }

    func applyTheme() {
        layer?.backgroundColor = Design.Color.control.cgColor
        layer?.borderColor = Design.Color.border.cgColor
        text.textColor = Design.Color.textPrimary
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        applyTheme()
    }
}

// MARK: - Pasteboard pieces

/// A file on a herdr machine, as a drag out of the tree carries it.
struct RemoteFileRef: Codable, Equatable, Sendable {
    var machine: String
    var path: String
}

@MainActor
enum FilesDragging {
    /// A remote entry's machine and path, beside its file promise: a
    /// terminal on that machine types the path.
    static let remoteType = NSPasteboard.PasteboardType("dev.azulejo.remote-file")

    /// What the tree takes: files, promised files.
    static let types: [NSPasteboard.PasteboardType] =
        [.fileURL] + NSFilePromiseReceiver.readableDraggedTypes.map { NSPasteboard.PasteboardType($0) }
        + [remoteType]

    static func fileURLs(_ pasteboard: NSPasteboard) -> [URL] {
        let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true])
        return (urls as? [URL] ?? []).filter(\.isFileURL)
    }

    /// The remote entries a drag out of a remote tree carries.
    static func remoteRefs(_ pasteboard: NSPasteboard) -> [RemoteFileRef] {
        (pasteboard.pasteboardItems ?? []).compactMap { item in
            guard let data = item.data(forType: remoteType) else { return nil }
            return try? JSONDecoder().decode(RemoteFileRef.self, from: data)
        }
    }
}

/// A remote entry dragged out of the tree: a file promise that downloads
/// it where it's dropped (`RemotePromises`), with its remote path as
/// `FilesDragging.remoteType`.
final class RemoteFilePromise: NSFilePromiseProvider {
    var remote: RemoteFileRef?

    override func writableTypes(for pasteboard: NSPasteboard) -> [NSPasteboard.PasteboardType] {
        super.writableTypes(for: pasteboard) + [NSPasteboard.PasteboardType("dev.azulejo.remote-file")]
    }

    override func pasteboardPropertyList(forType type: NSPasteboard.PasteboardType) -> Any? {
        if type.rawValue == "dev.azulejo.remote-file" {
            return remote.flatMap { try? JSONEncoder().encode($0) }
        }
        return super.pasteboardPropertyList(forType: type)
    }
}

/// Fulfils remote entries' promises: a download through the core's
/// `files.transfer` into the folder the drop chose.
final class RemotePromises: NSObject, NSFilePromiseProviderDelegate, @unchecked Sendable {
    static let shared = RemotePromises()
    private let queue: OperationQueue = {
        let queue = OperationQueue()
        queue.qualityOfService = .userInitiated
        return queue
    }()

    func filePromiseProvider(_ provider: NSFilePromiseProvider, fileNameForType fileType: String) -> String {
        guard let path = (provider as? RemoteFilePromise)?.remote?.path else { return "file" }
        return (path as NSString).lastPathComponent
    }

    func operationQueue(for provider: NSFilePromiseProvider) -> OperationQueue { queue }

    func filePromiseProvider(
        _ provider: NSFilePromiseProvider, writePromiseTo url: URL, completionHandler: @escaping (Error?) -> Void
    ) {
        let done = SendableBox(completionHandler)
        guard let remote = (provider as? RemoteFilePromise)?.remote else {
            return done.value(Self.failure("nothing to download"))
        }
        let folder = url.deletingLastPathComponent().path
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                guard let core = FilesPanelView.core else { return done.value(Self.failure("the core isn't running")) }
                FileTransfer.run([remote.path], from: remote.machine, into: FileTransfer.Folder(path: folder),
                                 mode: .copy, core: core) { result in
                    if let why = result.failure ?? result.items.first(where: { $0.error != nil })?.error {
                        done.value(Self.failure(why))
                    } else {
                        done.value(nil)
                    }
                }
            }
        }
    }

    private static func failure(_ why: String) -> NSError {
        NSError(domain: "dev.azulejo.files", code: 1, userInfo: [NSLocalizedDescriptionKey: why])
    }
}

/// A `files.delete` reply, parsed off the main thread.
struct DeleteResult: Sendable {
    struct Item: Sendable {
        var path: String
        var error: String?
    }

    var items: [Item] = []
    /// Nothing was deleted at all (the machine isn't there).
    var failure: String?

    init(_ reply: Result<[String: Any], CoreError>) {
        switch reply {
        case .failure(let error):
            failure = error.message
        case .success(let reply):
            items = (reply["items"] as? [[String: Any]] ?? []).map {
                Item(path: $0["path"] as? String ?? "", error: $0["error"] as? String)
            }
        }
    }
}

/// A value carried across threads that only one of them touches at a time.
final class SendableBox<T>: @unchecked Sendable {
    let value: T
    init(_ value: T) { self.value = value }
}

/// Promised files as they arrive (from the receivers' queue), handed to
/// `done` on the main actor once all have or the wait ends.
final class ReceivedFiles: @unchecked Sendable {
    private let lock = NSLock()
    private var expected: Int
    private var urls: [URL] = []
    private var finished = false
    private let done: @MainActor ([URL]) -> Void

    init(expected: Int, done: @escaping @MainActor ([URL]) -> Void) {
        self.expected = expected
        self.done = done
    }

    func arrived(_ url: URL?) {
        let complete: Bool = lock.withLock {
            if let url { urls.append(url) }
            expected -= 1
            return expected <= 0
        }
        if complete { DispatchQueue.main.async { self.finish() } }
    }

    func finish() {
        let urls: [URL]? = lock.withLock {
            guard !finished else { return nil }
            finished = true
            return self.urls
        }
        guard let urls else { return }
        let done = self.done
        MainActor.assumeIsolated { done(urls) }
    }
}

/// Where remote entries are downloaded for Copy and for drops: a folder of
/// the app's own in the per-user temp folder, swept of anything a day old
/// at launch.
enum RemoteCopies {
    nonisolated static var directory: URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("azulejo-remote", isDirectory: true)
    }

    /// A new, empty folder for one download.
    static func newFolder() -> URL? {
        let format = DateFormatter()
        format.locale = Locale(identifier: "en_US_POSIX")
        format.dateFormat = "yyyyMMdd-HHmmss"
        let name = "copy-\(format.string(from: Date()))-\(UUID().uuidString.prefix(6).lowercased())"
        let folder = directory.appendingPathComponent(name, isDirectory: true)
        do {
            try FileManager.default.createDirectory(
                at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            return folder
        } catch {
            return nil
        }
    }

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

// MARK: - [editor] app

/// `[editor] app`: Local files and folders open in a macOS app instead of
/// the terminal editor, and remote ones too when the app is Zed (its SSH
/// remoting).
/// Automation runs never launch it: the opener records the call.
@MainActor
enum EditorApp {
    /// `[editor] command`, for messages (the core runs it).
    static var command = "vim"
    /// `[editor] app`: a name, bundle ID or path; empty for none.
    static var app = ""

    static func apply(_ settings: [String: Any]) {
        command = settings["command"] as? String ?? "vim"
        app = settings["app"] as? String ?? ""
    }

    enum Outcome {
        case opened(String)
        case notFound
    }

    /// How a remote file fared: opened in Zed, not for this app (not set,
    /// not found or not Zed: the command opens it as before), or why Zed
    /// can't.
    enum RemoteOutcome {
        case opened(String)
        case notZed
        case failed(String)
    }

    /// A remote file's `ssh://` URL, or why there's none.
    enum SSHURL {
        case url(String)
        case unreachable(String)
    }

    /// Automation: what the opener was asked, `App|path` (`App|url` for a
    /// URL that isn't a file's), in order.
    static var opened: [String] = []

    /// Opens `file` (a file URL, or a URL the app handles, `zed://…`) in the
    /// app at `app`. Real runs hand it to NSWorkspace; automation only
    /// records it.
    static var opener: (URL, URL) -> Void = { file, app in
        if ContextMenus.automating {
            let what = file.isFileURL ? file.path : file.absoluteString
            opened.append("\(app.deletingPathExtension().lastPathComponent)|\(what)")
            return
        }
        let configuration = NSWorkspace.OpenConfiguration()
        NSWorkspace.shared.open([file], withApplicationAt: app, configuration: configuration) { _, error in
            if let error { NSLog("azulejo: opening in the editor app failed: \(error.localizedDescription)") }
        }
    }

    /// The app `name` means: a path, a bundle ID, or an app's name in the
    /// usual Applications folders.
    static func locate(_ name: String) -> URL? {
        let fm = FileManager.default
        if name.hasPrefix("/") || name.hasPrefix("~") {
            let path = (name as NSString).expandingTildeInPath
            return fm.fileExists(atPath: path) ? URL(fileURLWithPath: path) : nil
        }
        if name.contains("."), !name.hasSuffix(".app"),
           let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: name) {
            return url
        }
        let file = name.hasSuffix(".app") ? name : name + ".app"
        let folders = ["/Applications", "/Applications/Utilities", "/System/Applications",
                       "/System/Applications/Utilities", NSHomeDirectory() + "/Applications"]
        for folder in folders {
            let path = (folder as NSString).appendingPathComponent(file)
            if fm.fileExists(atPath: path) { return URL(fileURLWithPath: path) }
        }
        return nil
    }

    /// Opens a Local file or folder in `[editor] app`, or says it can't be
    /// found. Zed opens a folder as a project.
    static func open(_ path: String) -> Outcome {
        guard let url = locate(app) else { return .notFound }
        opener(URL(fileURLWithPath: path), url)
        return .opened(url.deletingPathExtension().lastPathComponent)
    }

    /// Whether the app at `url` is Zed: its bundle ID (`dev.zed.Zed`, and
    /// `dev.zed.Zed-Preview` and the like for other channels), or failing
    /// that its name (`Zed`, `Zed Preview`).
    static func isZed(_ url: URL) -> Bool {
        if let id = Bundle(url: url)?.bundleIdentifier {
            return id == "dev.zed.Zed" || id.hasPrefix("dev.zed.Zed-")
        }
        let name = url.deletingPathExtension().lastPathComponent
        return name == "Zed" || name.hasPrefix("Zed ")
    }

    /// Zed's link for an `ssh://[user@]host[:port]/path` URL:
    /// `zed://ssh/[user@]host[:port]/path`, which Zed opens as a remote
    /// file over its own SSH (it reads `~/.ssh/config` for the host).
    static func zedURL(ssh: String) -> URL? {
        guard ssh.hasPrefix("ssh://") else { return nil }
        return URL(string: "zed://ssh/" + ssh.dropFirst("ssh://".count))
    }

    /// Opens a remote file or folder in `[editor] app` when that's Zed, with
    /// its `ssh://` URL (`sshURL`, asked only then). Zed gets the file
    /// alone: a stable Zed refuses a project folder and a file in one
    /// request, and a Zed window already on that host's folder takes the
    /// file anyway. A folder opens as a remote project.
    static func openRemote(sshURL: () -> SSHURL) -> RemoteOutcome {
        guard !app.isEmpty, let url = locate(app), isZed(url) else { return .notZed }
        let name = url.deletingPathExtension().lastPathComponent
        switch sshURL() {
        case .unreachable(let why):
            return .failed("Can't open it in \(name): \(why)")
        case .url(let ssh):
            guard let link = zedURL(ssh: ssh) else { return .failed("Can't open it in \(name): no SSH URL") }
            opener(link, url)
            return .opened(name)
        }
    }
}
