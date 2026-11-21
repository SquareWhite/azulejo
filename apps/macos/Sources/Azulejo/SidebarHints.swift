import AppKit

/// ⌘ held alone shows each workspace row's chord (⌘1…⌘8, ⌘9 for the last)
/// in the full sidebar, as Safari and Arc show their tab shortcuts. The rail
/// always shows its numbers, so nothing changes there.
///
/// The badges wait `Design.Motion.sidebarHintDelay` (100 ms) after ⌘ goes
/// down alone: a chord typed at normal speed (⌘C, ⌘T, ⌘1) has its key down
/// within that (60–80 ms after ⌘), and a key down hides them and cancels
/// the wait, so quick chords never flash them; a deliberate hold answers in
/// about a tenth of a second. They go on release, on any key, when
/// another modifier joins ⌘ (until ⌘ is let go), when the window resigns key
/// and when the app goes to the background. Only the key window shows them.
///
/// One exception to "a key with ⌘ keeps them away until ⌘ is let go": the
/// full sidebar opening while ⌘ is still held alone (⌘A, toggle_agents_sidebar,
/// and ⌘ kept down) starts the wait again, so the badges show on the rows
/// that just came in. They wait for the sidebar's motion to end too, so they
/// don't fade in on rows still growing in. The keyboard must say ⌘ alone
/// after the expand: a click on the rail or ⌘K's Toggle Sidebar, with ⌘ up,
/// shows nothing. ⌘C, ⌘V and the other chords keep the rule.
///
/// It only watches: the flagsChanged monitor hands every event on, and key
/// downs reach it from the key router before it decides them, so the
/// terminal and the keymap see every key as before.
@MainActor
final class WorkspaceHints {
    private weak var app: AppController?
    private var monitor: Any?
    private var resignObserver: Any?
    /// The badges' wait after ⌘ went down alone.
    private var pending: DispatchWorkItem?
    /// The window the wait is for.
    private weak var pendingWindow: NSWindow?
    /// The window showing the badges.
    private weak var showing: WindowController?
    /// A key or another modifier came with ⌘: no badges until ⌘ is let go.
    private var suppressed = false
    /// flagsChanged events the monitor has seen (automation checks it).
    private(set) var flagsSeen = 0
    /// The modifiers the last flagsChanged held.
    private var held: NSEvent.ModifierFlags = []
    /// Automation: treat the window as key in a background run, which has
    /// none (`sidebar hints key on|off`).
    static var assumeKey = false
    /// Automation hands the modifiers in (`sidebar hints cmd|none|MODS`):
    /// what's held is what it last said, not the keyboard.
    static var handedIn = false

    init(app: AppController) {
        self.app = app
    }

    func install() {
        monitor = NSEvent.addLocalMonitorForEvents(matching: .flagsChanged) { [weak self] event in
            self?.flagsSeen += 1
            self?.flagsChanged(event)
            return event
        }
        resignObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didResignActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.appResignedActive() }
        }
    }

    /// The modifiers that count: caps lock and the keypad flag don't.
    private static let modifiers: NSEvent.ModifierFlags = [.command, .shift, .option, .control, .function]

    func flagsChanged(_ event: NSEvent) {
        let held = event.modifierFlags.intersection(Self.modifiers)
        self.held = held
        if held == .command {
            guard !suppressed, pending == nil, showing == nil else { return }
            wait(Design.Motion.sidebarHintDelay, in: event.window)
            return
        }
        // Let go of everything: the next ⌘ shows them again. Another
        // modifier with ⌘ (⌘⇧…): a chord's on its way.
        suppressed = !held.isEmpty
        hide()
    }

    /// Any key down, before the router decides it.
    func keyPressed(_ event: NSEvent) {
        if event.modifierFlags.contains(.command) || pending != nil || showing != nil { suppressed = true }
        hide()
    }

    /// The full sidebar opened in `controller` (from the rail or hidden).
    /// With ⌘ still held alone in the key window, the wait starts again,
    /// even after the chord that opened it; it lasts until the sidebar's
    /// motion (`settling`) ends if that's longer.
    func sidebarExpanded(in controller: WindowController, settling: TimeInterval) {
        let now = Self.handedIn ? held : NSEvent.modifierFlags.intersection(Self.modifiers)
        guard now == .command, showing == nil,
              Self.assumeKey || (NSApp.isActive && controller.window.isKeyWindow)
        else { return }
        suppressed = false
        wait(max(Design.Motion.sidebarHintDelay, settling), in: controller.window)
    }

    /// The sidebar in `controller` collapsed or hid: its wait stops (the
    /// sidebar hides its own badges).
    func sidebarCollapsed(in controller: WindowController) {
        if showing === controller { showing = nil }
        if pendingWindow === controller.window { cancelWait() }
    }

    func windowResignedKey() { hide() }

    func appResignedActive() {
        suppressed = false
        held = []
        hide()
    }

    private func wait(_ delay: TimeInterval, in window: NSWindow?) {
        cancelWait()
        let work = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.pending = nil
            self.show(in: window)
        }
        pending = work
        pendingWindow = window
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
    }

    private func cancelWait() {
        pending?.cancel()
        pending = nil
        pendingWindow = nil
    }

    private func show(in window: NSWindow?) {
        guard let app, let controller = app.controller(for: window ?? NSApp.keyWindow),
              Self.assumeKey || (NSApp.isActive && controller.window.isKeyWindow),
              controller.sidebarState == .full
        else { return }
        showing = controller
        controller.sidebarView.setHints(true)
    }

    func hide() {
        cancelWait()
        showing?.sidebarView.setHints(false)
        showing = nil
    }

    /// Automation: the wait for the badges is running.
    var isPending: Bool { pending != nil }
}
