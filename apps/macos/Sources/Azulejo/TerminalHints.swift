import Foundation

/// Ranges are UTF-16 offsets in the captured rows, not terminal columns.
/// The surface maps them back through Ghostty, which owns Unicode cell widths.
struct TerminalHintSpan: Equatable {
    let row: Int
    let range: NSRange
}

struct TerminalHintColorSpan {
    let row: Int
    let range: NSRange
    /// Palette index, or RGB with bit 24 set. Default foreground is omitted.
    let color: UInt32
}

struct TerminalHintItem: Equatable {
    enum Kind: String { case url, path, branch, command, ip, domain, uuid, hash }
    let kind: Kind
    let value: String
    let spans: [TerminalHintSpan]
    let openURL: URL?
}
enum TerminalHints {
    /// Text wrappers (coding agents, `fmt`) may stop a little short of the
    /// pane's last column. Measured in cells.
    static let wrapTolerance = 2
    private static let ipv4 = try! NSRegularExpression(
        pattern: #"(?<![\w.:-])(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(?::(\d{1,5}))?(?![\w-]|\.\d|:\d)"#)
    private static let ipv6 = try! NSRegularExpression(
        pattern: #"(?<![\w:.\[])(\[)?([0-9A-Fa-f]{0,4}(?::[0-9A-Fa-f]{0,4}){2,7}(?:\.\d{1,3}){0,3})(\](?::(\d{1,5}))?)?(?![\w.]*[\w])"#)
    private static let domain = try! NSRegularExpression(
        pattern: #"(?<![\w.@/:\\-])(?:[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?\.)+([A-Za-z]{2,24})(?::(\d{1,5}))?(?![\w@-]|\.[A-Za-z0-9])"#)
    private static let uuid = try! NSRegularExpression(
        pattern: #"(?<![\w-])[0-9A-Fa-f]{8}(?:-[0-9A-Fa-f]{4}){3}-[0-9A-Fa-f]{12}(?![\w-])"#)
    private static let hash = try! NSRegularExpression(pattern: #"(?<![\w./:-])[0-9a-f]{7,40}(?![\w-]|\.\w)"#)
    /// Indent plus a secondary prompt (`> `, zsh `pipe> `, `dquote> `).
    private static let ps2 = try! NSRegularExpression(
        pattern: #"^[ \t]*(?:(?:(?:pipe|cmdand|cmdor|d?quote|bquote|heredoc|subsh|cursh) ?)*> )?"#)
    /// Deliberately small: TLDs that double as common file extensions
    /// (md, sh, py, rs, pl, in, so, cc, app, zip, ...) are left out so
    /// filenames never look like hosts.
    private static let topLevelDomains: Set<String> = Set("""
        com org net edu gov mil int info biz name pro io dev ai co me tv xyz
        tech cloud site online page wiki blog shop news email link club
        design studio digital network software services company group team
        world today life media tools works zone systems host
        example invalid test localhost local internal lan home arpa onion
        uk de fr nl be eu us ca au nz jp cn kr tw hk sg ru ua br ar mx es pt
        it ch at se dk fi no ie is cz sk hu ro bg gr tr il za ly gg je fm
        """.split(whereSeparator: \.isWhitespace).map(String.init))
    private static let urls = try! NSRegularExpression(pattern: #"(?:https?://|mailto:|file://)[^\s<>\"'`]+"#)
    private static let paths = try! NSRegularExpression(
        pattern: #"(?:\"[^\"\n]+\"|'[^'\n]+'|(?:\\[^\n]|[^\s<>\"'`|;,])+)"#)
    private static let prompt = try! NSRegularExpression(pattern: #"^\s*(?:(?:[^\s]*[@~/][^\s]*|\[[^\]\r\n]+\])\s*)?[$%❯➜] +(.+?)\s*$"#)
    private static let agentPrompt = try! NSRegularExpression(pattern: #"^\s*![ \t]*([A-Za-z_./~][^\r\n]*?)\s*$"#)
    private static let branchLine = try! NSRegularExpression(pattern: #"^\s*[*+] ([^\s]+)\s*$"#)
    private static let locationSuffix = try! NSRegularExpression(pattern: #":\d+(?::\d+)?$"#)
    private static let otherBranchLine = try! NSRegularExpression(pattern: #"^  ([^\s]+)\s*$"#)
    private static let pathStart = try! NSRegularExpression(pattern: #"(?:^|[\(\[\{:=])(?=/|~/|\.\.?/)"#)
    private static let code = try! NSRegularExpression(pattern: #"`([^`\r\n]+)`"#)
    private static let listPrefix = try! NSRegularExpression(pattern: #"^\s*(?:[-*•] |\d+[.)] )?"#)
    // Recognition vocabulary, not an execution allowlist. Never consult PATH:
    // visible commands may target a different OS or a remote machine.
    private static let commands: Set<String> = Set("""
        sudo doas su env xargs sh bash zsh fish dash ksh csh tcsh
        ls cd pwd cat head tail less more grep egrep fgrep rg ag find fd
        awk sed cut sort uniq wc tr tee printf echo test true false
        uname whoami id hostname date uptime which command man file stat
        diff patch strings od hexdump base64 md5 md5sum shasum sha256sum
        source export unset alias unalias history read exec exit clear tput
        cp mv rm mkdir rmdir touch chmod chown chgrp ln readlink realpath
        du df mount umount ps top htop kill killall pkill pgrep sleep watch
        free vmstat iostat lsblk blkid dmesg nvidia-smi
        tar gzip gunzip zip unzip xz bzip2 curl wget ssh scp sftp rsync
        ping traceroute dig nslookup host ip ifconfig ss netstat lsof
        git hg svn make cmake ninja gcc clang cc c++ g++ rustc cargo rustup
        python python3 pip pip3 pipx uv node npm npx pnpm yarn bun deno
        ruby gem bundle go java javac gradle mvn dotnet docker podman
        kubectl helm terraform ansible ansible-playbook jq yq sqlite3
        brew port defaults plutil launchctl diskutil hdiutil softwareupdate
        xcodebuild xcrun swift swiftc open osascript pbcopy pbpaste say
        apt apt-get apt-cache dpkg yum dnf rpm pacman zypper apk
        systemctl journalctl service usermod useradd groupadd passwd
        tailscale just task ffmpeg ffprobe convert magick
        """.split(whereSeparator: \.isWhitespace).map(String.init))
    private static let windowsCommands: Set<String> = Set("""
        cmd powershell pwsh dir copy xcopy robocopy move del erase ren rename
        echo cd chdir mkdir rmdir path pause prompt ver vol date time title color help exit
        md rd type cls set setx where start call pushd popd mklink attrib
        icacls takeown tasklist taskkill ipconfig net netsh sc schtasks
        reg winget choco scoop sfc dism wsl shutdown findstr certutil systeminfo
        get-childitem get-content get-item get-command get-help get-process
        get-service get-location set-location set-content add-content
        copy-item move-item remove-item new-item rename-item test-path
        select-object where-object foreach-object sort-object select-string
        measure-object write-output write-host invoke-webrequest invoke-restmethod
        start-process stop-process start-service stop-service restart-service
        expand-archive compress-archive
        """.split(whereSeparator: \.isWhitespace).map(String.init))
    private static let vtControls = try! NSRegularExpression(
        pattern: #"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\r?\n"#)

    /// Decode our Ghostty formatter's SGR snapshot, never raw PTY output.
    /// Keep UTF-16 ranges so color boundaries use the same coordinate space
    /// as all other detectors; bold/underline changes don't split a color run.
    static func styledRows(_ vt: String) -> (rows: [String], colors: [TerminalHintColorSpan]) {
        let ns = vt as NSString
        var rows = [""], colors: [TerminalHintColorSpan] = []
        var foreground: UInt32?
        var offset = 0, column = 0
        func append(_ range: NSRange) {
            guard range.length > 0 else { return }
            let row = rows.count - 1
            let start = column
            rows[row] += ns.substring(with: range)
            column += range.length
            if let foreground {
                let range = NSRange(location: start, length: range.length)
                if let last = colors.last, last.row == row, last.color == foreground,
                   NSMaxRange(last.range) == start {
                    colors[colors.count - 1] = TerminalHintColorSpan(
                        row: row, range: NSUnionRange(last.range, range), color: foreground)
                } else {
                    colors.append(TerminalHintColorSpan(row: row, range: range, color: foreground))
                }
            }
        }
        for match in vtControls.matches(in: vt, range: NSRange(location: 0, length: ns.length)) {
            append(NSRange(location: offset, length: match.range.location - offset))
            let control = ns.substring(with: match.range)
            if control == "\r\n" || control == "\n" {
                rows.append("")
                column = 0
            } else if control.hasPrefix("\u{1b}["), control.hasSuffix("m") {
                let parameters = control.dropFirst(2).dropLast().split(separator: ";", omittingEmptySubsequences: false)
                var i = 0
                while i < parameters.count {
                    guard let n = parameters[i].isEmpty ? 0 : Int(parameters[i]) else { i += 1; continue }
                    switch n {
                    case 0, 39: foreground = nil
                    case 30...37: foreground = UInt32(n - 30)
                    case 90...97: foreground = UInt32(n - 90 + 8)
                    case 38, 48, 58:
                        // Consume background/underline payloads too: their RGB
                        // components must never be mistaken for SGR operations.
                        if i + 2 < parameters.count, parameters[i + 1] == "5" {
                            if n == 38 { foreground = UInt32(parameters[i + 2]) }
                            i += 2
                        } else if i + 4 < parameters.count, parameters[i + 1] == "2" {
                            if n == 38, let r = UInt32(parameters[i + 2]),
                               let g = UInt32(parameters[i + 3]), let b = UInt32(parameters[i + 4]) {
                                foreground = 0x1000000 | r << 16 | g << 8 | b
                            }
                            i += 4
                        }
                    default: break
                    }
                    i += 1
                }
            }
            offset = NSMaxRange(match.range)
        }
        append(NSRange(location: offset, length: ns.length - offset))
        return (rows, colors)
    }

    /// A slice of one physical row placed at `start` in a logical line.
    private struct Part {
        let row: Int
        var rowStart: Int
        var start: Int
        var length: Int
    }

    /// Physical rows joined into one logical line. `inserted` holds the
    /// separator spaces of hard wraps (no cell behind them); `dropped` holds
    /// offsets left out of a command's value (line-continuation backslashes).
    private struct Line {
        var text: String
        var parts: [Part]
        var inserted: Set<Int> = []
        var dropped: Set<Int> = []
        var length: Int { text.utf16.count }
    }

    private enum Join { case none, cut, wrap, explicit, quote }

    /// `joins` contains physical rows whose token continues at column zero
    /// of the next row (terminal soft wraps). With `columns`, rows that an
    /// application wrapped itself (newline plus repeated indent) also join:
    /// prompted commands continue across wrapped or continued rows, and URLs
    /// and paths cut at the right edge are rebuilt. Spans keep the original
    /// physical row ranges for the overlay.
    static func detectWrapped(rows: [String], joins: Set<Int>, cwd: String?, local: Bool,
                              branch: String?, colors: [TerminalHintColorSpan] = [],
                              columns: Int? = nil) -> [TerminalHintItem] {
        var soft: [Line] = []
        for (row, text) in rows.enumerated() {
            let length = text.utf16.count
            if row > 0, joins.contains(row - 1), !soft.isEmpty {
                let last = soft.count - 1
                soft[last].parts.append(Part(row: row, rowStart: 0, start: soft[last].length, length: length))
                soft[last].text += text
            } else {
                soft.append(Line(text: text, parts: [Part(row: row, rowStart: 0, start: 0, length: length)]))
            }
        }
        let lines = joinHardWraps(soft, rows: rows, columns: columns)
        var logicalColors: [TerminalHintColorSpan] = []
        for (index, line) in lines.enumerated() {
            for part in line.parts {
                for color in colors where color.row == part.row {
                    let clipped = NSIntersectionRange(color.range, NSRange(location: part.rowStart, length: part.length))
                    guard clipped.length > 0 else { continue }
                    let range = NSRange(location: part.start + clipped.location - part.rowStart, length: clipped.length)
                    // A hard wrap's separator doesn't break a color run.
                    if let last = logicalColors.last, last.row == index, last.color == color.color,
                       NSMaxRange(last.range) == range.location
                        || (NSMaxRange(last.range) + 1 == range.location && line.inserted.contains(NSMaxRange(last.range))) {
                        logicalColors[logicalColors.count - 1] = TerminalHintColorSpan(
                            row: index, range: NSUnionRange(last.range, range), color: color.color)
                    } else {
                        logicalColors.append(TerminalHintColorSpan(row: index, range: range, color: color.color))
                    }
                }
            }
        }
        return detect(rows: lines.map(\.text), cwd: cwd, local: local, branch: branch, colors: logicalColors).map { item in
            let span = item.spans[0]
            let line = lines[span.row]
            let spans = line.parts.compactMap { part -> TerminalHintSpan? in
                let overlap = NSIntersectionRange(span.range, NSRange(location: part.start, length: part.length))
                guard overlap.length > 0 else { return nil }
                return TerminalHintSpan(row: part.row, range: NSRange(location: overlap.location - part.start + part.rowStart,
                                                                      length: overlap.length))
            }
            var value = item.value
            let dropped = line.dropped.filter { NSLocationInRange($0, span.range) }
            if item.kind == .command, !dropped.isEmpty {
                let ns = line.text as NSString
                var units: [unichar] = []
                for offset in span.range.location..<NSMaxRange(span.range) where !dropped.contains(offset) {
                    units.append(ns.character(at: offset))
                }
                value = String(utf16CodeUnits: units, count: units.count)
            }
            return TerminalHintItem(kind: item.kind, value: value, spans: spans, openURL: item.openURL)
        }
    }

    private static func joinHardWraps(_ soft: [Line], rows: [String], columns: Int?) -> [Line] {
        var result: [Line] = []
        var index = 0
        while index < soft.count {
            var line = soft[index]
            let command = commandStart(line.text)
            var joined = 0
            // Rows joined only to close a quote: undone if it never closes,
            // so a stray apostrophe can't swallow the output below.
            var unclosed: (line: Line, index: Int)?
            // A screenful is the natural bound; the cap guards odd input.
            while index + 1 < soft.count, joined < 64 {
                let join = joinKind(line, soft[index + 1], rows: rows, columns: columns, command: command)
                if join == .none { break }
                if join == .quote, unclosed == nil { unclosed = (line, index) }
                append(soft[index + 1], to: &line, join)
                index += 1
                joined += 1
                if let command, !openQuote((line.text as NSString).substring(from: command)) { unclosed = nil }
            }
            if let unclosed { (line, index) = unclosed }
            result.append(line)
            index += 1
        }
        return result
    }

    private static func joinKind(_ line: Line, _ next: Line, rows: [String], columns: Int?, command: Int?) -> Join {
        guard !next.text.allSatisfy(\.isWhitespace), commandStart(next.text) == nil,
              let firstRow = line.parts.first?.row, let lastRow = line.parts.last?.row else { return .none }
        let trimmed = line.text.replacingOccurrences(of: #"\s+$"#, with: "", options: .regularExpression)
        let lastToken = trimmed.split(whereSeparator: \.isWhitespace).last.map(String.init) ?? ""
        let firstToken = next.text.split(whereSeparator: \.isWhitespace).first.map(String.init) ?? ""
        // Application wraps repeat a non-zero indent. Rows at column zero
        // come from a shell, where the terminal soft-wraps instead.
        let indent = indentation(rows[firstRow])
        let width = cells(rows[lastRow].replacingOccurrences(of: #"\s+$"#, with: "", options: .regularExpression))
        let wrapped = columns != nil && indent > 0 && indentation(next.text) == indent
        let edge = (columns ?? 0) - wrapTolerance
        // A token longer than the wrap width can only have been cut;
        // a shorter one would have moved to the next row whole.
        if wrapped, width >= edge, lastToken.contains("/"),
           cells(lastToken) + cells(firstToken) > edge - indent { return .cut }
        guard let command else { return .none }
        let text = (trimmed as NSString).substring(from: min(command, (trimmed as NSString).length))
        if continues(text) { return .explicit }
        if openQuote(text) { return .quote }
        if wrapped, width + 1 + cells(firstToken) > edge { return .wrap }
        return .none
    }

    private static func append(_ next: Line, to line: inout Line, _ join: Join) {
        var end = line.length
        let ns = line.text as NSString
        while end > 0, let scalar = UnicodeScalar(ns.character(at: end - 1)),
              CharacterSet.whitespaces.contains(scalar) { end -= 1 }
        line.text = ns.substring(to: end)
        line.parts = line.parts.compactMap { part in
            var part = part
            part.length = min(part.length, end - part.start)
            return part.length > 0 ? part : nil
        }
        line.inserted = line.inserted.filter { $0 < end }
        line.dropped = line.dropped.filter { $0 < end }
        let backslash = join == .explicit && line.text.hasSuffix("\\")
            && line.text.reversed().prefix(while: { $0 == "\\" }).count % 2 == 1
        if backslash { line.dropped.insert(end - 1) }
        let strip = join == .explicit || join == .quote ? ps2 : leadingSpace
        let skip = strip.firstMatch(in: next.text, range: NSRange(location: 0, length: next.length))?.range.length ?? 0
        if join != .cut {
            line.inserted.insert(line.length)
            if backslash { line.dropped.insert(line.length) }
            line.text += " "
        }
        let base = line.length - skip
        line.text += (next.text as NSString).substring(from: skip)
        for (index, part) in next.parts.enumerated() {
            var part = part
            if index == 0 {
                part.rowStart += skip
                part.length -= skip
            }
            part.start += base + (index == 0 ? skip : 0)
            if part.length > 0 { line.parts.append(part) }
        }
        line.inserted.formUnion(next.inserted.map { $0 + base })
        line.dropped.formUnion(next.dropped.map { $0 + base })
    }

    private static let leadingSpace = try! NSRegularExpression(pattern: #"^[ \t]*"#)

    /// UTF-16 offset of the command text after a prompt marker, if any.
    private static func commandStart(_ text: String) -> Int? {
        let whole = NSRange(location: 0, length: text.utf16.count)
        guard let match = prompt.firstMatch(in: text, range: whole) ?? agentPrompt.firstMatch(in: text, range: whole)
        else { return nil }
        return match.range(at: 1).location
    }

    /// Explicit shell continuation: trailing `\`, `|`, `&&` or `||`.
    private static func continues(_ text: String) -> Bool {
        if text.hasSuffix("|") || text.hasSuffix("&&") { return true }
        return text.reversed().prefix(while: { $0 == "\\" }).count % 2 == 1
    }

    private static func openQuote(_ text: String) -> Bool {
        var quote: Character?, escaped = false
        for ch in text {
            if escaped { escaped = false; continue }
            switch (quote, ch) {
            case (nil, "\\"), ("\"", "\\"): escaped = true
            case (nil, "\""), (nil, "'"): quote = ch
            case ("\"", "\""), ("'", "'"): quote = nil
            default: break
            }
        }
        return quote != nil
    }

    private static func indentation(_ text: String) -> Int {
        text.prefix(while: { $0 == " " || $0 == "\t" }).count
    }

    /// Approximate terminal cell width; good enough for wrap decisions.
    /// Exact overlay geometry still comes from Ghostty.
    private static func cells<S: StringProtocol>(_ text: S) -> Int {
        text.reduce(0) { total, ch in
            guard let scalar = ch.unicodeScalars.first else { return total }
            let wide: Bool
            switch scalar.value {
            case 0x1100...0x115F, 0x2E80...0xA4CF, 0xAC00...0xD7A3, 0xF900...0xFAFF,
                 0xFE30...0xFE4F, 0xFF00...0xFF60, 0xFFE0...0xFFE6, 0x20000...0x3FFFD: wide = true
            default: wide = scalar.properties.isEmojiPresentation
            }
            return total + (wide ? 2 : 1)
        }
    }

    /// Reading order is deterministic. A command stays selectable next to
    /// the items inside it; other items never overlap each other.
    static func detect(rows: [String], cwd: String?, local: Bool, branch: String?,
                       colors: [TerminalHintColorSpan] = []) -> [TerminalHintItem] {
        var result: [TerminalHintItem] = []
        // Recognise the rest of a plain `git branch` listing only when its
        // contiguous block includes an explicit current/worktree marker.
        var branchRows: Set<Int> = []
        for (row, text) in rows.enumerated() where branchLine.firstMatch(
            in: text, range: NSRange(location: 0, length: text.utf16.count)) != nil {
            branchRows.insert(row)
            for step in [-1, 1] {
                var next = row + step
                while rows.indices.contains(next), otherBranchLine.firstMatch(
                    in: rows[next], range: NSRange(location: 0, length: rows[next].utf16.count)) != nil {
                    branchRows.insert(next)
                    next += step
                }
            }
        }
        for (row, text) in rows.enumerated() {
            let ns = text as NSString
            let whole = NSRange(location: 0, length: ns.length)
            let listedBranch = branchRows.contains(row)
                ? (branchLine.firstMatch(in: text, range: whole) ?? otherBranchLine.firstMatch(in: text, range: whole)) : nil
            let listedBranchName = listedBranch.map { ns.substring(with: $0.range(at: 1)) }
            var commandRanges: [NSRange] = []
            func add(_ kind: TerminalHintItem.Kind, _ range: NSRange, value: String? = nil, url: URL? = nil) {
                guard range.length > 0 else { return }
                if kind == .command { commandRanges.append(range) }
                result.append(TerminalHintItem(kind: kind, value: value ?? ns.substring(with: range),
                                               spans: [TerminalHintSpan(row: row, range: range)], openURL: url))
            }
            if let match = prompt.firstMatch(in: text, range: whole) ?? agentPrompt.firstMatch(in: text, range: whole) {
                let range = match.range(at: 1)
                let value = ns.substring(with: range)
                // A displayed command must be a single shell line. Never turn
                // terminal control characters into executable input.
                if !value.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) {
                    add(.command, range)
                }
            } else {
                // Style/code boundaries keep surrounding prose out of inline
                // commands. Without those boundaries only consider a complete
                // line (optionally a Markdown list item), never arbitrary words.
                // On styled rows, don't expand an unrecognised colored path
                // or noun into the surrounding sentence.
                let prefix = listPrefix.firstMatch(in: text, range: whole)!.range
                let line = NSRange(location: NSMaxRange(prefix), length: ns.length - NSMaxRange(prefix))
                let rowColors = colors.filter { $0.row == row }.map(\.range)
                var candidates: [NSRange] = rowColors.isEmpty ? [line] : []
                candidates += code.matches(in: text, range: whole).map { $0.range(at: 1) }
                candidates += rowColors
                var accepted: [NSRange] = []
                for candidate in candidates {
                    guard NSMaxRange(candidate) <= ns.length,
                          let range = commandRange(candidate, in: ns),
                          !accepted.contains(where: { NSIntersectionRange($0, range).length > 0 }) else { continue }
                    accepted.append(range)
                    add(.command, range)
                }
            }
            // Non-command items compete for their text: overlapping
            // candidates resolve to the longest (ip:port over ip, a URL over
            // its host). Commands keep their own hint above these.
            var found: [(kind: TerminalHintItem.Kind, range: NSRange, value: String?, url: URL?)] = []
            for match in urls.matches(in: text, range: whole) {
                let range = trimmedURL(match.range, in: ns)
                let value = ns.substring(with: range)
                guard let url = URL(string: value), let scheme = url.scheme?.lowercased(),
                      ["http", "https", "mailto", "file"].contains(scheme) else { continue }
                if ["http", "https"].contains(scheme), url.host?.isEmpty != false { continue }
                let canOpen = scheme != "file" || (local && (url.host == nil || url.host == "" || url.host == "localhost"))
                found.append((.url, range, nil, canOpen ? url : nil))
            }
            for match in paths.matches(in: text, range: whole) {
                var range = match.range
                var value = ns.substring(with: range)
                let quoted = value.first == "\"" || value.first == "'"
                if quoted {
                    range = NSRange(location: range.location + 1, length: range.length - 2)
                    value = ns.substring(with: range)
                } else {
                    range = pathRange(range, in: ns, cwd: cwd, local: local)
                    value = ns.substring(with: range)
                }
                guard !value.isEmpty, !value.contains("://") else { continue }
                let decoded = quoted ? value : unescapePath(value)
                if branch == decoded || listedBranchName == decoded { continue }
                let suffix = locationSuffix.firstMatch(in: decoded, range: NSRange(location: 0, length: (decoded as NSString).length))
                let path = suffix.map { (decoded as NSString).substring(to: $0.range.location) } ?? decoded
                let explicit = path.hasPrefix("/") || path.hasPrefix("./") || path.hasPrefix("../") || path.hasPrefix("~/")
                let url = resolvedPath(path, cwd: cwd, local: local)
                let exists = url.map { FileManager.default.fileExists(atPath: $0.path) } ?? false
                let relative = path.contains("/") && !path.contains(":")
                // An existing bare word ("docs") only counts as a path inside
                // a command or when it looks like a filename; prose is full
                // of words that happen to name a directory.
                let filename = path.range(of: #"^[^/]*\.[A-Za-z0-9_-]{1,12}$"#, options: .regularExpression) != nil
                let inCommand = commandRanges.contains { NSIntersectionRange($0, range).length > 0 }
                guard explicit || relative || (exists && (filename || inCommand)) else { continue }
                found.append((.path, range, decoded, local && exists ? url : nil))
            }
            for match in ipv4.matches(in: text, range: whole) {
                if match.range(at: 1).location != NSNotFound,
                   Int(ns.substring(with: match.range(at: 1)))! > 65535 { continue }
                found.append((.ip, match.range, nil, nil))
            }
            for match in ipv6.matches(in: text, range: whole) {
                let bracketed = match.range(at: 1).location != NSNotFound
                guard bracketed == (match.range(at: 3).location != NSNotFound) else { continue }
                var address = match.range(at: 2)
                var value = ns.substring(with: address)
                // Sentence punctuation after an unbracketed address.
                while !bracketed, value.hasSuffix(":"), !value.hasSuffix("::") {
                    value.removeLast(); address.length -= 1
                }
                var buffer = in6_addr()
                guard value.contains(where: \.isNumber), value.filter({ $0 == ":" }).count >= 2,
                      inet_pton(AF_INET6, value, &buffer) == 1 else { continue }
                if match.range(at: 4).location != NSNotFound,
                   Int(ns.substring(with: match.range(at: 4)))! > 65535 { continue }
                found.append((.ip, bracketed ? match.range : address, nil, nil))
            }
            for match in domain.matches(in: text, range: whole) {
                guard topLevelDomains.contains(ns.substring(with: match.range(at: 1)).lowercased()) else { continue }
                if match.range(at: 2).location != NSNotFound,
                   Int(ns.substring(with: match.range(at: 2)))! > 65535 { continue }
                found.append((.domain, match.range, nil, nil))
            }
            for match in uuid.matches(in: text, range: whole) {
                found.append((.uuid, match.range, nil, nil))
            }
            for match in hash.matches(in: text, range: whole) {
                // Plain words and plain numbers aren't hashes.
                let value = ns.substring(with: match.range)
                guard value.contains(where: \.isNumber), value.contains(where: \.isLetter) else { continue }
                found.append((.hash, match.range, nil, nil))
            }
            if let match = listedBranch {
                found.append((.branch, match.range(at: 1), nil, nil))
            }
            if let branch, !branch.isEmpty {
                var search = whole
                while search.length > 0 {
                    let range = ns.range(of: branch, range: search)
                    if range.location == NSNotFound { break }
                    let end = NSMaxRange(range)
                    func character(_ offset: Int) -> Character? {
                        offset < 0 || offset >= ns.length ? nil : ns.substring(with: NSRange(location: offset, length: 1)).first
                    }
                    func boundary(_ ch: Character?) -> Bool {
                        ch == nil || ch!.isWhitespace || "()[]{}<>\"'`,;:".contains(ch!)
                    }
                    // Sentence punctuation ends a branch name, but `main.rs` isn't `main`.
                    let after = character(end)
                    let ends = boundary(after) || (after.map { ".!?".contains($0) } == true && boundary(character(end + 1)))
                    if boundary(character(range.location - 1)), ends {
                        found.append((.branch, range, nil, nil))
                    }
                    search = NSRange(location: end, length: ns.length - end)
                }
            }
            let priority: [TerminalHintItem.Kind: Int] = [.branch: 0, .url: 1, .path: 2, .ip: 3, .uuid: 4, .hash: 5, .domain: 6]
            var taken: [NSRange] = []
            for candidate in found.sorted(by: {
                if $0.range.length != $1.range.length { return $0.range.length > $1.range.length }
                if $0.kind != $1.kind { return priority[$0.kind, default: 9] < priority[$1.kind, default: 9] }
                return $0.range.location < $1.range.location
            }) where !taken.contains(where: { NSIntersectionRange($0, candidate.range).length > 0 }) {
                taken.append(candidate.range)
                add(candidate.kind, candidate.range, value: candidate.value, url: candidate.url)
            }
        }
        return result.sorted {
            let a = $0.spans[0], b = $1.spans[0]
            if a.row != b.row { return a.row < b.row }
            if a.range.location != b.range.location { return a.range.location < b.range.location }
            return $0.kind.rawValue < $1.kind.rawValue
        }
    }

    private static func commandRange(_ candidate: NSRange, in text: NSString) -> NSRange? {
        let original = text.substring(with: candidate)
        let value = original.trimmingCharacters(in: .whitespaces)
        guard !value.isEmpty,
              !value.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
              let first = value.split(separator: " ", omittingEmptySubsequences: true).first else { return nil }
        let word = String(first)
        let lower = word.lowercased()
        let executable = lower.hasSuffix(".exe") ? String(lower.dropLast(4)) : lower
        guard commands.contains(word) || windowsCommands.contains(executable)
            || (lower.hasSuffix(".exe") && commands.contains(executable)) else { return nil }
        return text.range(of: value, options: .literal, range: candidate)
    }

    /// Which side of its item's first cell a hint label sits on.
    enum BadgeSide: String {
        /// Just left of the item, joined to its highlight (the default).
        case left
        /// Just right of the item, when the left has no room.
        case right
        /// Left or right over another item's text: no free room either side.
        case leftCover = "left-cover", rightCover = "right-cover"
        /// Nowhere free of other labels; drawn left anyway.
        case clash
    }

    /// One item for label placement, in terminal columns of one pane.
    struct BadgeRequest {
        /// Every row the item covers: `start..<end` on `row`. The label goes
        /// on the first one.
        let cells: [(row: Int, start: Int, end: Int)]
        let label: Int
        /// Commands contain other items, so other labels may sit over a
        /// command's text; never over their own item or a non-command.
        let command: Bool
    }

    /// Places labels outside their items, one cell per letter, in reading
    /// order: left of the first cell, else right of it on that row. A label
    /// never covers its own item. It avoids other labels and other items'
    /// text (command text excepted) and only covers that text when neither
    /// side is free. Returns each label's first column and side.
    static func badgeColumns(_ requests: [BadgeRequest], columns: Int) -> [(start: Int, side: BadgeSide)] {
        var placed: [(row: Int, range: Range<Int>)] = []
        var result: [(start: Int, side: BadgeSide)] = []
        for (index, request) in requests.enumerated() {
            guard let first = request.cells.first else { result.append((0, .clash)); continue }
            let n = request.label
            func fits(_ start: Int, coverOthers: Bool) -> Bool {
                let range = start..<(start + n)
                guard start >= 0, start + n <= columns else { return false }
                if placed.contains(where: { $0.row == first.row && $0.range.overlaps(range) }) { return false }
                for (other, item) in requests.enumerated() where other == index || (!item.command && !coverOthers) {
                    if item.cells.contains(where: { $0.row == first.row && ($0.start..<$0.end).overlaps(range) }) { return false }
                }
                return true
            }
            let left = first.start - n, right = first.end
            let choice: (Int, BadgeSide)
            if fits(left, coverOthers: false) { choice = (left, .left) }
            else if fits(right, coverOthers: false) { choice = (right, .right) }
            else if fits(left, coverOthers: true) { choice = (left, .leftCover) }
            else if fits(right, coverOthers: true) { choice = (right, .rightCover) }
            else { choice = (left >= 0 ? left : (right + n <= columns ? right : 0), .clash) }
            placed.append((first.row, choice.0..<(choice.0 + n)))
            result.append(choice)
        }
        return result
    }

    static func labels(count: Int) -> [String] {
        let alphabet = Array("asdfghjklqwertyuiopzxcvbnm")
        var length = 2, capacity = alphabet.count * alphabet.count
        while capacity < count { capacity *= alphabet.count; length += 1 }
        return (0..<count).map { index in
            var n = index
            var chars = Array(repeating: alphabet[0], count: length)
            for i in stride(from: length - 1, through: 0, by: -1) {
                chars[i] = alphabet[n % alphabet.count]; n /= alphabet.count
            }
            return String(chars)
        }
    }

    private static func trimmedURL(_ original: NSRange, in text: NSString) -> NSRange {
        var range = original
        while range.length > 0 {
            let value = text.substring(with: range)
            let last = value.last!
            var trim = ".,;:!?".contains(last)
            for (open, close): (Character, Character) in [("(", ")"), ("[", "]"), ("{", "}")] where last == close {
                trim = value.filter { $0 == close }.count > value.filter { $0 == open }.count
            }
            guard trim else { break }
            range.length -= 1
        }
        return range
    }

    /// Output punctuation is not part of a path. Keep quoted/escaped names
    /// and balanced filename punctuation intact, including literal trailing
    /// punctuation when that exact local file exists.
    private static func pathRange(_ original: NSRange, in text: NSString, cwd: String?, local: Bool) -> NSRange {
        var range = original
        var value = text.substring(with: range)
        if let start = pathStart.firstMatch(in: value, range: NSRange(location: 0, length: (value as NSString).length)),
           NSMaxRange(start.range) > 0 {
            let skip = NSMaxRange(start.range)
            range.location += skip
            range.length -= skip
            value = text.substring(with: range)
        } else {
            while let first = value.first, "([{".contains(first) {
                value.removeFirst()
                range.location += 1
                range.length -= 1
            }
        }
        while let last = value.last, ".,:!?)]}".contains(last) {
            let escaped = value.dropLast().reversed().prefix(while: { $0 == "\\" }).count % 2 == 1
            if escaped { break }
            if local, let url = resolvedPath(unescapePath(value), cwd: cwd, local: true),
               FileManager.default.fileExists(atPath: url.path) { break }
            var trim = ".,:!?".contains(last)
            for (open, close): (Character, Character) in [("(", ")"), ("[", "]"), ("{", "}")] where last == close {
                trim = value.filter { $0 == close }.count > value.filter { $0 == open }.count
            }
            guard trim else { break }
            value.removeLast()
            range.length -= 1
        }
        return range
    }

    private static func unescapePath(_ text: String) -> String {
        var result = "", escaped = false
        for ch in text {
            if escaped { result.append(ch); escaped = false }
            else if ch == "\\" { escaped = true }
            else { result.append(ch) }
        }
        if escaped { result.append("\\") }
        return result
    }

    private static func resolvedPath(_ path: String, cwd: String?, local: Bool) -> URL? {
        guard local else { return nil }
        let expanded = (path as NSString).expandingTildeInPath
        if expanded.hasPrefix("/") { return URL(fileURLWithPath: expanded).standardizedFileURL }
        guard let cwd else { return nil }
        return URL(fileURLWithPath: cwd, isDirectory: true).appendingPathComponent(expanded).standardizedFileURL
    }
}
