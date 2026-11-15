import AppKit

/// ⌘S Run (`r`): runs a selected command in the current pane when it's an
/// idle shell, else in the nearest idle shell in the tab on the same
/// machine, else in a new pane beside the source (`[hints] run_target`).
/// The choice is the core's (`azj_hints_run_plan`, core/crates/hints
/// `run.rs`); this gathers the panes and carries it out.
@MainActor
enum TerminalRun {
    /// Tiles used as Run targets, most recent first: ties between equally
    /// near shells go to the one used last.
    private static var recent: [String] = []
    /// Bounds for the layout tree's rects: only their proportions matter.
    private static let unit = CGRect(x: 0, y: 0, width: 1000, height: 1000)

    /// Runs `value` from `source`'s screen. Returns the target for
    /// automation: `current`, `reuse:<N>` (the Nth tile in reading order, as
    /// `{pane:N}` counts) or `new`.
    @discardableResult
    static func run(_ value: String, cwd: String, source: String, tab: TabModel, workspace: WorkspaceModel,
                    app: AppController, controller: WindowController) -> String {
        let machine = { (tile: TileModel) in tile.herdr?.machine ?? workspace.machineID }
        let frames = tab.root.frames(in: unit, gap: 0)
        let panes: [[String: Any]] = tab.order.compactMap { id in
            guard let tile = tab.tile(id), tile.kind == .terminal, let rect = frames[id] else { return nil }
            var pane: [String: Any] = ["tile": id, "rect": [rect.minX, rect.minY, rect.width, rect.height],
                                       "machine": machine(tile), "idle": tile.idleShell]
            if let cwd = tile.terminalCwd { pane["cwd"] = cwd }
            return pane
        }
        let sourceMachine = tab.tile(source).map(machine) ?? workspace.machineID
        let request: [String: Any] = ["mode": TerminalHints.runTarget, "value": value, "cwd": cwd,
                                      "machine": sourceMachine, "current": tab.focused, "panes": panes,
                                      "recent": recent]
        let plan = TerminalHints.runPlan(request) ?? ("new", nil, TerminalHints.runLine(value))
        let tile = plan.target == "current" ? tab.focused : plan.target == "reuse" ? plan.tile : nil
        guard let tile else {
            let command: [String: Any] = ["method": "layout.split", "params": [
                "tile": source, "direction": "right",
                "content": ["kind": "terminal", "cwd": cwd, "command": [plan.line]],
            ]]
            switch app.send(command) {
            case .success(let reply): remember(reply["tile"] as? String)
            case .failure(let error): controller.report(error)
            }
            return "new"
        }
        remember(tile)
        // As a new pane takes focus, so does the shell reused: the output
        // shows (in Monocle, it comes to the front).
        if tile != tab.focused { app.send(["method": "tile.focus", "params": ["tile": tile]]) }
        type(plan.line, into: tile, app: app, controller: controller)
        if plan.target == "current" { return "current" }
        return "reuse:\((tab.order.firstIndex(of: tile) ?? -1) + 1)"
    }

    /// Clears whatever is half typed at the prompt, types `line` and
    /// presses Return, in order, without blocking on herdr. ⌃E ⌃U is end
    /// of line, then delete back to its start, by default in bash
    /// (readline), zsh (emacs and vi insert keymaps) and fish.
    private static func type(_ line: String, into tile: String, app: AppController, controller: WindowController) {
        let steps: [[String: Any]] = [
            ["method": "terminal.send_keys", "params": ["tile": tile, "keys": ["ctrl+e", "ctrl+u"]]],
            ["method": "terminal.send_text", "params": ["tile": tile, "text": line]],
            ["method": "terminal.send_keys", "params": ["tile": tile, "keys": ["enter"]]],
        ]
        func step(_ index: Int) {
            guard index < steps.count else { return }
            app.core.executeAsync(steps[index], decode: { $0.map { _ in () } }) { [weak controller] result in
                if case .failure(let error) = result {
                    controller?.report(error)
                    return
                }
                step(index + 1)
            }
        }
        step(0)
    }

    private static func remember(_ tile: String?) {
        guard let tile else { return }
        recent.removeAll { $0 == tile }
        recent.insert(tile, at: 0)
        if recent.count > 16 { recent.removeLast() }
    }

    /// Automation: each terminal tile's state for Run in reading order
    /// (`idle` / `busy`), `-` for other tiles.
    static func idleStates(_ tab: TabModel?) -> String {
        guard let tab else { return "-" }
        return tab.order.map { id in
            guard let tile = tab.tile(id), tile.kind == .terminal else { return "-" }
            return tile.idleShell ? "idle" : "busy"
        }.joined(separator: ",")
    }
}
