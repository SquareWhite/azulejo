import AppKit

/// Azulejo ▸ Install Command-Line Tool…: symlinks the bundled `azj` onto the
/// user's PATH after asking. Logic lives in CLIInstaller; this is the UI.
/// AZJ_CLI_INSTALL_DIR overrides the target directory (tests, odd setups).
@MainActor
final class InstallCLIAction: NSObject {
    static let shared = InstallCLIAction()

    @objc func install(_ sender: Any?) {
        let installer = CLIInstaller()
        let source = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/azj")
        let plan: CLIInstaller.Plan
        do {
            plan = try installer.plan(
                source: source,
                loginPath: CLIInstaller.loginShellPath(),
                home: FileManager.default.homeDirectoryForCurrentUser,
                override: ProcessInfo.processInfo.environment["AZJ_CLI_INSTALL_DIR"])
        } catch {
            alert("Can't install azj", "\(error)", style: .warning)
            return
        }

        switch plan.existing {
        case .current:
            alert("azj is already installed", "\(plan.link.path) points at this Azulejo.", style: .informational)
            return
        case .file:
            alert("Can't install azj", "\(CLIInstaller.Failure.wouldClobber(plan.link.path))", style: .warning)
            return
        case .none, .otherLink:
            break
        }

        let confirm = NSAlert()
        confirm.messageText = "Install the azj command?"
        var info = "Creates a link at \(plan.link.path) pointing at the azj inside this app."
        if case .otherLink(let old) = plan.existing {
            info += "\n\nIt replaces the existing link to \(old)."
        }
        confirm.informativeText = info
        confirm.addButton(withTitle: "Install")
        confirm.addButton(withTitle: "Cancel")
        guard confirm.runModal() == .alertFirstButtonReturn else { return }

        do {
            try installer.install(plan)
        } catch {
            alert("Couldn't install azj", "\(error)", style: .warning)
            return
        }
        var done = "Run `azj help` in a new terminal."
        if plan.needsPathHint {
            done = "\(plan.link.deletingLastPathComponent().path) isn't on your PATH yet. "
                + "Add it in your shell config, then run `azj help` in a new terminal."
        }
        alert("azj installed", done, style: .informational)
    }

    private func alert(_ title: String, _ text: String, style: NSAlert.Style) {
        let alert = NSAlert()
        alert.alertStyle = style
        alert.messageText = title
        alert.informativeText = text
        alert.runModal()
    }
}
