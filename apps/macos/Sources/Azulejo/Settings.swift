import Foundation

/// Where the app keeps its own small UI settings across launches: the
/// sidebar's Agents | Files mode and width, the ⌘K recents. Everything that
/// used to go straight to `UserDefaults.standard` goes through here.
///
/// - A plain launch (the installed app) uses `UserDefaults.standard`, the
///   `dev.azulejo.Azulejo` domain, with the same keys as ever.
/// - The dev profile (`AZULEJO_PROFILE=dev`) never touches that domain: it
///   keeps a plist next to its session state, `settings.plist` in the folder
///   of the core's `state_path` (`.dev/<name>/state/` with `dev.sh`). Each dev
///   instance gets its own, a relaunch of the same instance reads it back, and
///   `dev.sh --reset` deletes it with the rest of the state. A dev run without
///   a `state_path` keeps its settings in memory only.
///
/// System settings the app only reads (`AppleInterfaceStyle`,
/// `AppleActionOnDoubleClick`) still come from `UserDefaults.standard`.
@MainActor
final class SettingsStore {
    static let shared = SettingsStore(env: ProcessInfo.processInfo.environment)

    private enum Backing {
        case defaults(UserDefaults)
        /// The dev profile's own values, saved to `file` (nil: memory only).
        case file(URL?)
    }

    private let backing: Backing
    private var values: [String: Any] = [:]

    /// Whether these settings are a dev instance's own (and so safe for
    /// automation to wipe).
    var isDevInstance: Bool {
        if case .file = backing { return true }
        return false
    }

    /// Where a dev instance keeps its settings; nil for the installed app.
    var fileURL: URL? {
        if case .file(let url) = backing { return url }
        return nil
    }

    init(env: [String: String]) {
        guard env["AZULEJO_PROFILE"] == "dev" else {
            backing = .defaults(.standard)
            return
        }
        let url = Self.devFile(coreOptions: env["AZULEJO_CORE_OPTIONS"])
        backing = .file(url)
        if let url, let data = try? Data(contentsOf: url),
           let dict = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any] {
            values = dict
        }
    }

    /// `settings.plist` next to the core's `state_path`, if the options name one.
    nonisolated static func devFile(coreOptions: String?) -> URL? {
        guard let data = coreOptions?.data(using: .utf8),
              let options = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let state = options["state_path"] as? String, !state.isEmpty
        else { return nil }
        return URL(fileURLWithPath: state).deletingLastPathComponent().appendingPathComponent("settings.plist")
    }

    func object(forKey key: String) -> Any? {
        switch backing {
        case .defaults(let defaults): defaults.object(forKey: key)
        case .file: values[key]
        }
    }

    func string(forKey key: String) -> String? { object(forKey: key) as? String }
    func stringArray(forKey key: String) -> [String]? { object(forKey: key) as? [String] }
    /// 0 when unset, as `UserDefaults.double(forKey:)`.
    func double(forKey key: String) -> Double { (object(forKey: key) as? NSNumber)?.doubleValue ?? 0 }

    /// Sets `value` (a property-list value); nil removes the key.
    func set(_ value: Any?, forKey key: String) {
        switch backing {
        case .defaults(let defaults):
            if let value { defaults.set(value, forKey: key) } else { defaults.removeObject(forKey: key) }
        case .file(let url):
            values[key] = value
            if let url { save(to: url) }
        }
    }

    func removeObject(forKey key: String) { set(nil, forKey: key) }

    /// Writes the whole (small) dictionary, atomically, on every change.
    private func save(to url: URL) {
        do {
            try FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            let data = try PropertyListSerialization.data(fromPropertyList: values, format: .xml, options: 0)
            try data.write(to: url, options: .atomic)
        } catch {
            FileHandle.standardError.write(Data("azulejo: can't save dev settings: \(error)\n".utf8))
        }
    }
}
