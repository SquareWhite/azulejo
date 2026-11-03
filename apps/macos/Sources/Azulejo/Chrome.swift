import AppKit

/// A flat, highlightable row used by the sidebar and tab bar.
@MainActor
final class RowButton: NSButton {
    private let handler: () -> Void

    /// Automation: the marker's text (" ● 2"), or "".
    let markerText: String
    /// Automation: the right-aligned detail (a branch), or "".
    let detailText: String

    /// `detail` shows dim and right-aligned (a workspace's branch); it needs
    /// `detailWidth`, the row's width.
    init(title: String, selected: Bool, marker: NSAttributedString? = nil,
         detail: String? = nil, detailWidth: CGFloat = 0, handler: @escaping () -> Void) {
        self.handler = handler
        markerText = marker?.string ?? ""
        detailText = detail ?? ""
        super.init(frame: .zero)
        self.title = title
        isBordered = false
        alignment = .left
        font = selected ? Design.Font.rowSelected : Design.Font.row
        contentTintColor = selected ? Design.Color.textPrimary : Design.Color.textSecondary
        if marker != nil || detail != nil {
            let style = NSMutableParagraphStyle()
            style.lineBreakMode = .byTruncatingTail
            if detail != nil {
                style.tabStops = [NSTextTab(textAlignment: .right, location: max(detailWidth - 16, 40))]
            }
            let text = NSMutableAttributedString(string: title, attributes: [
                .font: font ?? Design.Font.row,
                .foregroundColor: selected ? Design.Color.textPrimary : Design.Color.textSecondary,
                .paragraphStyle: style,
            ])
            if let marker { text.append(marker) }
            if let detail {
                text.append(NSAttributedString(string: "\t" + Self.shorten(detail, to: 16), attributes: [
                    .font: Design.Font.detail,
                    .foregroundColor: Design.Color.textMuted,
                    .paragraphStyle: style,
                ]))
                toolTip = detail
            }
            attributedTitle = text
        }
        wantsLayer = true
        layer?.cornerRadius = Design.Radius.row
        layer?.backgroundColor = selected ? Design.Color.selected.cgColor : nil
        target = self
        action = #selector(clicked)
        refusesFirstResponder = true
        setContentHuggingPriority(.defaultHigh, for: .horizontal)
        heightAnchor.constraint(equalToConstant: Design.Space.rowHeight).isActive = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: NSSize {
        let size = super.intrinsicContentSize
        return NSSize(width: size.width + 14, height: 22)
    }

    @objc private func clicked() { handler() }

    /// `text` cut to `max` characters with an ellipsis.
    static func shorten(_ text: String, to max: Int) -> String {
        text.count <= max ? text : String(text.prefix(max - 1)) + "…"
    }
}
