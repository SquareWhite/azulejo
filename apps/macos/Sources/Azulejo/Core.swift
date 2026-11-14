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
            // The JSON pointer dies when the callback returns, so copy it first.
            let text = String(cString: json)
            let deliver: @MainActor @Sendable () -> Void = {
                Unmanaged<Core>.fromOpaque(context).takeUnretainedValue().onEvent?(text)
            }
            // Today the core calls back inside `send`, on the main thread. Later the
            // herdr adapter and API server will call back from their own threads.
            if Thread.isMainThread {
                MainActor.assumeIsolated(deliver)
            } else {
                DispatchQueue.main.async(execute: deliver)
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
