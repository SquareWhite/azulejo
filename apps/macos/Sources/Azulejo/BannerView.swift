import AppKit
import AzulejoCore

extension Core {
    /// Reads the config file again now; the core also sends `config.reloaded`.
    /// Returns that payload (no `path` when there's no config file).
    func reloadConfig() -> [String: Any] {
        callJSON([:]) { handle, _ in azj_core_reload_config(handle) }
    }
}

/// A strip above the tiles for config problems after a reload (F25b). It
/// takes its own row under the toolbar and the tiles move down for it, so it
/// never covers a header or terminal text; dismiss with ×, and it goes away
/// by itself after a later clean reload.
@MainActor
final class BannerView: NSView {
    static let maxLines = 4
    private let icon = NSTextField(labelWithString: "")
    private let text = NSTextField(wrappingLabelWithString: "")
    private let close = NSButton()
    /// What the banner says, for automation ("" when hidden).
    private(set) var message = ""

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.banner
        isHidden = true

        icon.font = Design.Font.bannerIcon
        text.font = Design.Font.bannerText
        text.maximumNumberOfLines = Self.maxLines
        text.lineBreakMode = .byWordWrapping
        text.cell?.truncatesLastVisibleLine = true
        text.isSelectable = true
        close.bezelStyle = .inline
        close.isBordered = false
        close.title = "✕"
        close.font = Design.Font.bannerClose
        close.target = self
        close.action = #selector(dismiss)
        close.refusesFirstResponder = true
        // The message takes the width; the × stays its own size at the right
        // end (it used to stretch and sit mid-banner).
        close.setContentHuggingPriority(.required, for: .horizontal)
        text.setContentHuggingPriority(.defaultLow, for: .horizontal)
        for v in [icon, text, close] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            icon.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            text.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 8),
            text.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            text.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
            text.trailingAnchor.constraint(equalTo: close.leadingAnchor, constant: -8),
            close.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            close.topAnchor.constraint(equalTo: topAnchor, constant: 6),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Applies a `config.reloaded` payload. Returns true when it was clean
    /// (nothing to show), so the caller can flash a short confirmation.
    @discardableResult
    func apply(reload: [String: Any]) -> Bool {
        let applied = reload["applied"] as? Bool ?? false
        let diagnostics = reload["diagnostics"] as? [[String: Any]] ?? []
        if applied && diagnostics.isEmpty {
            hide()
            return true
        }
        let errors = diagnostics.contains { ($0["severity"] as? String) == "error" } || !applied
        let path = reload["path"] as? String
        var lines: [String] = []
        if !applied {
            lines.append("Config not applied; keeping the previous one.")
        }
        for d in diagnostics {
            // The file name is enough here; the full path is in the tooltip.
            let file = (d["file"] as? String ?? path).map { ($0 as NSString).lastPathComponent } ?? "config"
            var location = file
            if let line = d["line"] as? Int {
                location += ":\(line)"
                if let col = d["column"] as? Int { location += ":\(col)" }
            }
            lines.append("\(location)  \(d["message"] as? String ?? "")")
        }
        if lines.count > Self.maxLines {
            let extra = lines.count - (Self.maxLines - 1)
            lines = Array(lines.prefix(Self.maxLines - 1)) + ["… and \(extra) more"]
        }
        message = lines.joined(separator: "\n")
        text.stringValue = message
        toolTip = path
        let tint = errors ? Design.Color.error : Design.Color.warning
        icon.stringValue = errors ? "⚠︎" : "ⓘ"
        icon.textColor = tint
        layer?.backgroundColor = tint.withAlphaComponent(Design.Alpha.bannerFill).blended(
            withFraction: Design.Alpha.bannerMix, of: Design.Color.window)?.cgColor
        layer?.borderWidth = Design.Border.hairline
        layer?.borderColor = tint.withAlphaComponent(Design.Alpha.bannerBorder).cgColor
        isHidden = false
        superview?.needsLayout = true
        return false
    }

    @objc func dismiss() { hide() }

    func hide() {
        message = ""
        isHidden = true
        superview?.needsLayout = true
    }

    /// Height for `width`, for the window's layout.
    func preferredHeight(width: CGFloat) -> CGFloat {
        text.preferredMaxLayoutWidth = max(100, width - 80)
        return max(32, text.fittingSize.height + 16)
    }
}
