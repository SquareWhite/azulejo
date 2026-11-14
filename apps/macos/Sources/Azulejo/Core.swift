import AzulejoCore
import Foundation

/// Swift handle to the Rust core. Commands, replies, key routing and events
/// cross the C ABI as JSON (see `core/crates/ffi`).
///
/// Threading: every call goes through the main actor, matching the C ABI's
/// one-thread rule. Events arrive on the core's forwarder thread, never inside
/// `send` or `execute`; the callback copies the JSON and hops to the main
/// queue before `onEvent` runs.
@MainActor
final class Core {
    nonisolated(unsafe) private let handle: OpaquePointer
    var onEvent: ((String) -> Void)?

    /// Boots the core. `options` is the JSON `azj_core_new_with_options` takes;
    /// nil boots the real thing (the user's config, saved state, herdr session
    /// and API socket). Returns nil when the options are bad or the API socket
    /// is required and taken.
    init?(options: String?) {
        let made: OpaquePointer?
        if let options {
            made = azj_core_new_with_options(options)
        } else {
            made = azj_core_new()
        }
        guard let made else { return nil }
        handle = made
        azj_core_set_event_callback(handle, coreEventCallback, Unmanaged.passUnretained(self).toOpaque())
    }

    /// Runs a command; its events arrive through `onEvent`.
    @discardableResult
    func send(_ command: [String: Any]) -> Bool {
        if freed { return false }
        guard let json = Self.encode(command) else { return false }
        return azj_core_send(handle, json) == 0
    }

    /// Runs a command and returns its reply, or the core's error.
    func execute(_ command: [String: Any]) -> Result<[String: Any], CoreError> {
        if freed { return .failure(CoreError(code: "unavailable", message: "core stopped")) }
        guard let json = Self.encode(command) else {
            return .failure(CoreError(code: "invalid_request", message: "command isn't JSON"))
        }
        guard let object = take(azj_core_execute(handle, json)) else {
            return .failure(CoreError(code: "internal", message: "no result"))
        }
        if object["ok"] as? Bool == true {
            return .success(object["reply"] as? [String: Any] ?? [:])
        }
        let error = object["error"] as? [String: Any] ?? [:]
        return .failure(CoreError(
            code: error["code"] as? String ?? "internal",
            message: error["message"] as? String ?? "unknown error"))
    }

    /// Decodes a reply field (e.g. `session`) into a Codable type.
    func fetch<T: Decodable>(_ method: String, field: String, as type: T.Type) -> T? {
        guard case .success(let reply) = Perf.measure("fetch.exec:\(method)", { execute(["method": method]) }),
              let value = reply[field],
              let data = try? JSONSerialization.data(withJSONObject: value)
        else { return nil }
        do {
            return try Perf.measure("fetch.decode:\(method)") { try JSONDecoder().decode(T.self, from: data) }
        } catch {
            NSLog("azulejo: can't decode \(method).\(field): \(error)")
            return nil
        }
    }

    /// A reply field as the core sent it, with the raw reply text so callers
    /// can skip work when nothing changed. Decodes straight from the core's
    /// JSON (no `JSONSerialization` round trip): a session fetch runs after
    /// every change, so it's on the motion path.
    func fetchRaw<T: Decodable>(
        _ method: String, field: String, as type: T.Type, unlessEqualTo previous: String?
    ) -> (raw: String, value: T?)? {
        if freed { return nil }
        guard let json = Self.encode(["method": method]),
              let pointer = Perf.measure("fetch.exec:\(method)", { azj_core_execute(handle, json) })
        else { return nil }
        defer { azj_string_free(pointer) }
        let raw = String(cString: pointer)
        if raw == previous { return (raw, nil) }
        do {
            let decoder = JSONDecoder()
            decoder.userInfo[FieldEnvelope<T>.fieldKey] = field
            let envelope = try Perf.measure("fetch.decode:\(method)") {
                try decoder.decode(FieldEnvelope<T>.self, from: Data(raw.utf8))
            }
            return (raw, envelope.value)
        } catch {
            NSLog("azulejo: can't decode \(method).\(field): \(error)")
            return nil
        }
    }

    /// Routes a chord (`"alt+h"`) through the keymap, keeping prefix state.
    func key(chord: String, tile: String?) -> [String: Any] {
        if freed { return ["outcome": "error"] }
        var request: [String: Any] = ["chord": chord]
        request["tile"] = tile ?? NSNull()
        guard let json = Self.encode(request) else { return ["outcome": "error"] }
        return take(azj_core_key(handle, json)) ?? ["outcome": "error"]
    }

    /// Resolves an action by config name (`{"action":"split_right"}`), as menus
    /// and the palette need; same JSON as `key`.
    func action(_ request: [String: Any]) -> [String: Any] {
        guard !freed, let json = Self.encode(request) else { return ["outcome": "error"] }
        return take(azj_core_action(handle, json)) ?? ["outcome": "error"]
    }

    /// Drops an armed prefix.
    func resetKeys() {
        if freed { return }
        _ = take(azj_core_key(handle, #"{"reset":true}"#))
    }

    /// The global keymap: `{"platform","prefix","bindings":[{"action","index","chord"}]}`.
    func keymap() -> [String: Any] {
        if freed { return [:] }
        return take(azj_core_keymap(handle)) ?? [:]
    }

    /// Where the socket, state, config and herdr session live.
    /// `[animation]` settings (`azj_core_animation`); follows config reloads.
    func animation() -> [String: Any] {
        if freed { return [:] }
        return take(azj_core_animation(handle)) ?? [:]
    }

    func info() -> [String: Any] {
        if freed { return [:] }
        return take(azj_core_info(handle)) ?? [:]
    }

    /// Calls a JSON-in, JSON-out C function (`azj_core_terminal_command`, …)
    /// and parses its reply; `[:]` once the core is stopped.
    func callJSON(
        _ request: [String: Any],
        _ call: (OpaquePointer, String) -> UnsafeMutablePointer<CChar>?
    ) -> [String: Any] {
        guard !freed, let json = Self.encode(request) else { return [:] }
        return take(call(handle, json)) ?? [:]
    }

    /// Takes ownership of a string from the core and parses it.
    private func take(_ pointer: UnsafeMutablePointer<CChar>?) -> [String: Any]? {
        guard let pointer else { return nil }
        defer { azj_string_free(pointer) }
        let data = Data(String(cString: pointer).utf8)
        return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
    }

    private static func encode(_ object: [String: Any]) -> String? {
        guard let data = try? JSONSerialization.data(withJSONObject: object) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// Stops the API server (removing its socket), saves state and stops the
    /// core; herdr panes keep running. No callback runs after this returns.
    /// The app calls it on quit, since process exit skips `deinit`.
    func shutdown() {
        guard !freed else { return }
        freed = true
        onEvent = nil
        azj_core_free(handle)
    }

    nonisolated(unsafe) private var freed = false

    deinit {
        if !freed { azj_core_free(handle) }
    }
}

/// `{"ok":true,"reply":{"<field>":T,…}}`, with the field named in the
/// decoder's `userInfo`. A failed command (`ok` false) decodes as nil.
struct FieldEnvelope<T: Decodable>: Decodable {
    static var fieldKey: CodingUserInfoKey { CodingUserInfoKey(rawValue: "azj.field")! }
    let value: T?

    private struct Key: CodingKey {
        var stringValue: String
        var intValue: Int? { nil }
        init(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { nil }
    }

    init(from decoder: Decoder) throws {
        let top = try decoder.container(keyedBy: Key.self)
        guard try top.decodeIfPresent(Bool.self, forKey: Key(stringValue: "ok")) == true,
              let field = decoder.userInfo[Self.fieldKey] as? String,
              top.contains(Key(stringValue: "reply"))
        else {
            value = nil
            return
        }
        let reply = try top.nestedContainer(keyedBy: Key.self, forKey: Key(stringValue: "reply"))
        value = try reply.decodeIfPresent(T.self, forKey: Key(stringValue: field))
    }
}

struct CoreError: Error, CustomStringConvertible {
    let code: String
    let message: String
    var description: String { "\(code): \(message)" }
}

/// The core's event callback. A plain global function on purpose: it runs on
/// the core's forwarder thread, and a closure written inside `Core` (a
/// `@MainActor` type) would be main-actor isolated, which Swift 6 checks at
/// runtime and traps on. It copies the JSON (the pointer dies when this
/// returns) and hops to the main queue.
private func coreEventCallback(context: UnsafeMutableRawPointer?, json: UnsafePointer<CChar>?) {
    guard let context, let json else { return }
    let text = String(cString: json)
    let address = UInt(bitPattern: context)
    DispatchQueue.main.async {
        MainActor.assumeIsolated {
            guard let pointer = UnsafeMutableRawPointer(bitPattern: address) else { return }
            Unmanaged<Core>.fromOpaque(pointer).takeUnretainedValue().onEvent?(text)
        }
    }
}
