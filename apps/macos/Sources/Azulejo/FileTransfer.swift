import Foundation

/// Moves or copies files into a folder, here or on a herdr machine, through
/// the core's `files.transfer` (which does the work off its thread: renames
/// and copies locally, `scp` over the machine's SSH connection otherwise,
/// and never overwrites: a taken name gets Finder's "name 2.ext"). No UI:
/// callers show `Result.summary` however they like. Drops on a terminal use
/// it, and so does the sidebar's file tree (pastes, drops, and downloads
/// for Copy and drags out of a remote tree).
@MainActor
enum FileTransfer {
    enum Mode: String {
        case move, copy
    }

    /// A folder on a machine: `machine` nil for Local, else herdr's ID;
    /// `label` names the machine in messages (nil for Local).
    struct Folder: Equatable {
        var path: String
        var machine: String?
        var label: String?

        var isRemote: Bool { machine != nil }

        /// How the folder reads in a label: `~/src/app` here,
        /// `box:/srv/app` on a machine.
        var shown: String {
            guard let label else { return (path as NSString).abbreviatingWithTildeInPath }
            return "\(label):\(path)"
        }
    }

    /// What became of one source.
    struct Item: Sendable {
        var source: String
        /// Where it is now; nil when it failed.
        var dest: String?
        var error: String?
    }

    struct Result: Sendable {
        var items: [Item] = []
        /// Nothing happened at all (the folder or machine isn't there).
        var failure: String?
    }

    /// Transfers `sources` (absolute paths, Local or on herdr machine
    /// `machine`: a download) into `folder`; `then` runs on the main actor
    /// when it's done.
    static func run(
        _ sources: [String], from machine: String? = nil, into folder: Folder, mode: Mode, core: Core,
        then: @escaping @MainActor (Result) -> Void
    ) {
        var params: [String: Any] = ["sources": sources, "folder": folder.path, "mode": mode.rawValue]
        if let machine = folder.machine { params["to"] = machine }
        if let machine { params["from"] = machine }
        core.executeAsync(["method": "files.transfer", "params": params], decode: { reply in
            switch reply {
            case .failure(let error):
                return Result(failure: error.message)
            case .success(let reply):
                let items = (reply["items"] as? [[String: Any]] ?? []).map {
                    Item(source: $0["source"] as? String ?? "", dest: $0["dest"] as? String,
                         error: $0["error"] as? String)
                }
                return Result(items: items)
            }
        }, then: then)
    }

    /// "Uploading 2 items to box…", while a transfer to a machine runs.
    static func progress(_ count: Int, into folder: Folder, mode: Mode) -> String {
        let what = count == 1 ? "1 item" : "\(count) items"
        if let label = folder.label { return "Uploading \(what) to \(label)…" }
        return "\(mode == .move ? "Moving" : "Copying") \(what)…"
    }

    /// One line for a HUD: what happened, or the first thing that failed.
    static func summary(_ result: Result, into folder: Folder, mode: Mode) -> (text: String, failed: Bool) {
        let verb = mode == .move ? "move" : "copy"
        if let failure = result.failure { return ("Couldn't \(verb) to \(folder.shown): \(failure)", true) }
        let done = result.items.filter { $0.dest != nil && $0.error == nil }
        let failed = result.items.filter { $0.error != nil }
        let name = { (path: String) in (path as NSString).lastPathComponent }
        if let first = failed.first {
            let more = failed.count > 1 ? " (and \(failed.count - 1) more)" : ""
            return ("Couldn't \(verb) \(name(first.source))\(more): \(first.error ?? "")", true)
        }
        let past = folder.isRemote
            ? (mode == .move ? "Moved" : "Uploaded")
            : (mode == .move ? "Moved" : "Copied")
        if done.count == 1, let only = done.first, only.dest == only.source {
            return ("\(name(only.source)) is already in \(folder.shown)", false)
        }
        guard let only = done.first, done.count == 1 else {
            return ("\(past) \(done.count) items to \(folder.shown)", false)
        }
        let renamed = only.dest.map(name).flatMap { $0 == name(only.source) ? nil : $0 }
        return ("\(past) \(name(only.source)) to \(folder.shown)\(renamed.map { " as \($0)" } ?? "")", false)
    }
}
