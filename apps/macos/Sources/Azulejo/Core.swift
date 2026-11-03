import AzulejoCore
import Foundation

/// Swift handle to the Rust core. Commands and events cross the C ABI as JSON.
@MainActor
final class Core {
    nonisolated(unsafe) private let handle: OpaquePointer
    var onEvent: ((String) -> Void)?

    init() {
        handle = azj_core_new()
        let context = Unmanaged.passUnretained(self).toOpaque()
        azj_core_set_event_callback(handle, { context, json in
            guard let context, let json else { return }
            let text = String(cString: json)
            // The core calls back synchronously from `send`, which runs on the main actor.
            MainActor.assumeIsolated {
                Unmanaged<Core>.fromOpaque(context).takeUnretainedValue().onEvent?(text)
            }
        }, context)
    }

    @discardableResult
    func send(_ commandJSON: String) -> Bool {
        azj_core_send(handle, commandJSON) == 0
    }

    deinit {
        azj_core_free(handle)
    }
}
