import Foundation

/// Where "Install Command-Line Tool…" puts the `azj` symlink, and doing it.
/// Foundation only, so scripts/release/test-cli-installer.sh can compile and
/// test it without AppKit or a running app.
struct CLIInstaller {
    struct Plan: Equatable {
        /// The bundled binary the link points at.
        let source: URL
        /// The symlink to create.
        let link: URL
        /// The link's directory isn't on the user's login-shell PATH.
        let needsPathHint: Bool
        /// Something already exists at `link`.
        let existing: Existing
    }

    enum Existing: Equatable {
        case none
        /// A symlink already pointing at `source`: nothing to do.
        case current
        /// A symlink pointing elsewhere (e.g. an older Azulejo.app); safe to replace.
        case otherLink(String)
        /// A real file or directory; never replaced.
        case file
    }

    enum Failure: Error, Equatable, CustomStringConvertible {
        case missingSource(String)
        case wouldClobber(String)

        var description: String {
            switch self {
            case .missingSource(let path):
                "azj isn't in this build (\(path)). It ships with release builds (just release-mac)."
            case .wouldClobber(let path):
                "\(path) is a regular file, not a link. Remove it yourself if you want Azulejo's azj there."
            }
        }
    }

    let fileManager = FileManager.default

    /// Picks the link's directory: `override` (AZJ_CLI_INSTALL_DIR) if set;
    /// else ~/.local/bin when it's on the login PATH; else /usr/local/bin when
    /// it exists and is writable; else ~/.local/bin with a PATH hint.
    func plan(source: URL, loginPath: [String], home: URL, override: String?) throws -> Plan {
        guard fileManager.isExecutableFile(atPath: source.path) else {
            throw Failure.missingSource(source.path)
        }
        let localBin = home.appendingPathComponent(".local/bin")
        let onPath = Set(loginPath.map { ($0 as NSString).standardizingPath })
        let dir: URL
        if let override, !override.isEmpty {
            dir = URL(fileURLWithPath: override)
        } else if onPath.contains(localBin.path) {
            dir = localBin
        } else if fileManager.isWritableFile(atPath: "/usr/local/bin") {
            dir = URL(fileURLWithPath: "/usr/local/bin")
        } else {
            dir = localBin
        }
        let link = dir.appendingPathComponent("azj")
        return Plan(
            source: source, link: link,
            needsPathHint: !onPath.contains((dir.path as NSString).standardizingPath),
            existing: existing(at: link, source: source))
    }

    func existing(at link: URL, source: URL) -> Existing {
        if let target = try? fileManager.destinationOfSymbolicLink(atPath: link.path) {
            return target == source.path ? .current : .otherLink(target)
        }
        return fileManager.fileExists(atPath: link.path) ? .file : .none
    }

    /// Creates (or replaces) the symlink. Never touches a regular file.
    func install(_ plan: Plan) throws {
        switch existing(at: plan.link, source: plan.source) {
        case .current:
            return
        case .file:
            throw Failure.wouldClobber(plan.link.path)
        case .otherLink:
            try fileManager.removeItem(at: plan.link)
        case .none:
            break
        }
        try fileManager.createDirectory(
            at: plan.link.deletingLastPathComponent(), withIntermediateDirectories: true)
        try fileManager.createSymbolicLink(atPath: plan.link.path, withDestinationPath: plan.source.path)
    }

    /// PATH as the user's login shell sets it. Apps launched from Finder get a
    /// minimal PATH, so asking the shell is the only way to know. Falls back to
    /// this process's PATH if the shell doesn't answer within two seconds.
    static func loginShellPath() -> [String] {
        let fallback = (ProcessInfo.processInfo.environment["PATH"] ?? "").split(separator: ":").map(String.init)
        let shell = ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh"
        let process = Process()
        process.executableURL = URL(fileURLWithPath: shell)
        process.arguments = ["-l", "-c", "printenv PATH"]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        process.standardInput = FileHandle.nullDevice
        do { try process.run() } catch { return fallback }
        let deadline = Date().addingTimeInterval(2)
        while process.isRunning, Date() < deadline { usleep(20_000) }
        if process.isRunning {
            process.terminate()
            return fallback
        }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        let path = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        return path.isEmpty ? fallback : path.split(separator: ":").map(String.init)
    }
}
