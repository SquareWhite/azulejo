import Foundation

@main
struct HintDetectorChecks {
    static func main() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory.appendingPathComponent("src"), withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        try Data().write(to: directory.appendingPathComponent("src/example.swift"))
        try Data().write(to: directory.appendingPathComponent("two words.txt"))
        try Data().write(to: directory.appendingPathComponent("round(name)"))
        func detect(_ rows: [String], local: Bool = true, branch: String? = "feature/hints") -> [TerminalHintItem] {
            TerminalHints.detect(rows: rows, cwd: directory.path, local: local, branch: branch)
        }
        func check(_ condition: Bool, _ name: String) {
            guard condition else { fatalError("hint detector: \(name)") }
        }
        let links = detect(["(https://example.invalid/a_(b)). https://example.invalid/x?q=1&v=2,"])
        check(links.filter { $0.kind == .url }.map(\.value) == ["https://example.invalid/a_(b)", "https://example.invalid/x?q=1&v=2"], "balanced URL punctuation")
        check(detect(["javascript:alert(1) data:text/html,test"]).isEmpty, "non-openable schemes")
        let files = detect(["./src/example.swift:12:3 'two words.txt' two\\ words.txt"])
        check(files.map(\.value) == ["./src/example.swift:12:3", "two words.txt", "two words.txt"], "quoted and escaped paths")
        check(files.map(\.openURL) == [directory.appendingPathComponent("src/example.swift"), directory.appendingPathComponent("two words.txt"), directory.appendingPathComponent("two words.txt")], "cwd and line-number resolution")
        let remote = detect(["./src/example.swift file:///tmp/example.txt https://example.invalid"], local: false)
        check(remote.map(\.openURL) == [nil, nil, URL(string: "https://example.invalid")], "remote files never become local open targets")
        check(detect(["./missing.txt"]).first?.openURL == nil, "missing path remains copy-only")
        let compileLine = "Compiling sample v0.1.0 (\(directory.path)/src)"
        let buildLines = [compileLine,
                          "--prepared:\(directory.path)/src/example.swift",
                          "output=\(directory.path)/src/example.swift",
                          "See (./src/example.swift:12:3).",
                          "[./missing.txt]", "target/release/example.app",
                          "\(directory.path)/round(name)"]
        let buildPaths = detect(buildLines).filter { $0.kind == .path }
        check(buildPaths.map(\.value) == [directory.path + "/src", directory.path + "/src/example.swift",
                                         directory.path + "/src/example.swift", "./src/example.swift:12:3",
                                         "./missing.txt", "target/release/example.app",
                                         directory.path + "/round(name)"], "formatted build-output paths")
        check(buildPaths.first?.spans == [TerminalHintSpan(row: 0, range: (compileLine as NSString).range(of: directory.path + "/src"))],
              "path highlights exclude surrounding output punctuation")
        check(buildPaths[4].openURL == nil && buildPaths[5].openURL == nil, "missing relative paths remain copy-only")
        check(detect(["  feature/other", "* feature/hints", "  main"]).filter { $0.kind == .branch }.map(\.value) == ["feature/other", "feature/hints", "main"], "branch listing context")
        let unicode = "界👩‍💻e\u{301} (feature/hints) feature/hints-extra"
        let branches = detect([unicode]).filter { $0.kind == .branch }
        check(branches.map(\.value) == ["feature/hints"], "branch token boundaries")
        check(branches.first?.spans.first?.range == (unicode as NSString).range(of: "feature/hints"), "UTF-16 range after wide and combining glyphs")
        let commands = detect(["$ printf example > ran.txt", "docs mention printf example", "$ printf\tunsafe",
                               "100% complete", "price$ 42", "dev@example.invalid:~/src$ printf qualified"])
        check(commands.filter { $0.kind == .command }.map(\.value) == ["printf example > ran.txt", "printf qualified"],
              "explicit prompt and control-character boundary")
        let agentRows = ["!printf agent", "  ! printf spaced", "!./check --fast", "docs mention !printf",
                         "!!", "!123", "!=", "!", "! printf\tunsafe", "$ ! grep absent"]
        let agentCommands = detect(agentRows).filter { $0.kind == .command }
        check(agentCommands.map(\.value) == ["printf agent", "printf spaced", "./check --fast", "! grep absent"],
              "agent command markers and shell-negation precedence")
        check(agentCommands.first?.spans == [TerminalHintSpan(row: 0, range: NSRange(location: 1, length: 12))],
              "agent command selection excludes its marker")
        check(detect(["sudo bash /tmp/setup.sh", "git status --short", "brew install example",
                      "DIR /b", "Get-ChildItem -Force"]).filter { $0.kind == .command }.map(\.value)
              == ["sudo bash /tmp/setup.sh", "git status --short", "brew install example",
                  "DIR /b", "Get-ChildItem -Force"], "unprompted cross-platform commands")
        check(detect(["Use `git status --short`, then continue.", "echo `pwd`",
                      "git.exe status", "notgit status", "docs mention sudo bash /tmp/setup.sh"])
              .filter { $0.kind == .command }.map(\.value)
              == ["git status --short", "echo `pwd`", "git.exe status"], "code boundaries, shell substitutions and executable names")
        let styled = TerminalHints.styledRows(
            "界👩‍💻e\u{301} run once: \u{1b}[94msudo bash /home/example/setup.sh\u{1b}[0m's folder\r\n"
            + "or add \u{1b}[38;5;117msudo usermod -aG wheel example\u{1b}[39m, and continue\r\n"
            + "read \u{1b}[38;2;100;170;230m/home/example/setup.sh\u{1b}[0m first\r\n"
            + "then \u{1b}[34mprintf \u{1b}[0m\u{1b}[1m\u{1b}[34m'%s' \u{1b}[4:3mexample\u{1b}[0m, not the prose")
        let styledItems = TerminalHints.detect(rows: styled.rows, cwd: directory.path, local: true, branch: nil, colors: styled.colors)
        let styledCommands = styledItems.filter { $0.kind == .command }
        check(styledCommands.map(\.value) == ["sudo bash /home/example/setup.sh", "sudo usermod -aG wheel example", "printf '%s' example"],
              "color boundaries select full inline commands and exclude adjoining prose")
        check(styledCommands[0].spans[0].range == (styled.rows[0] as NSString).range(of: "sudo bash /home/example/setup.sh"),
              "colored command UTF-16 boundaries after wide and combining characters")
        check(styledItems.contains { $0.kind == .path && $0.value == "/home/example/setup.sh" },
              "paths inside a full command remain selectable")
        check(styledCommands.allSatisfy { $0.spans[0].row != 2 }, "color alone does not make a path runnable")
        let twoCommands = TerminalHints.styledRows("Use \u{1b}[34mgit status\u{1b}[0m or \u{1b}[34mbrew update\u{1b}[0m.")
        check(TerminalHints.detect(rows: twoCommands.rows, cwd: nil, local: false, branch: nil, colors: twoCommands.colors)
              .filter { $0.kind == .command }.map(\.value) == ["git status", "brew update"],
              "separate same-colored commands never absorb intervening prose")
        let wrappedCommand = TerminalHints.styledRows(
            "Run \u{1b}[38;2;100;170;230msudo bash /tmp/a\r\nlong-script.sh\u{1b}[0m then continue")
        let wrappedCommands = TerminalHints.detectWrapped(rows: wrappedCommand.rows, joins: [0], cwd: nil, local: false,
                                                         branch: nil, colors: wrappedCommand.colors).filter { $0.kind == .command }
        check(wrappedCommands.map(\.value) == ["sudo bash /tmp/along-script.sh"]
              && wrappedCommands[0].spans.count == 2, "full colored commands across physical row wraps")
        let wrapped = TerminalHints.detectWrapped(rows: ["https://example.invalid/", "long/path"], joins: [0], cwd: directory.path, local: true, branch: nil)
        check(wrapped.first?.value == "https://example.invalid/long/path", "wrapped URL reconstruction")
        check(wrapped.first?.spans == [TerminalHintSpan(row: 0, range: NSRange(location: 0, length: 24)), TerminalHintSpan(row: 1, range: NSRange(location: 0, length: 9))], "wrapped physical row spans")
        try checkSheet(directory: directory, check)
        // Labels: ASCII rows, so UTF-16 offsets are terminal columns.
        func badges(_ rows: [String], columns: Int) -> [(item: TerminalHintItem, start: Int, side: String)] {
            let items = TerminalHints.detectWrapped(rows: rows, joins: [], cwd: directory.path, local: true,
                                                    branch: "main", columns: columns)
            let placed = TerminalHints.badgeColumns(items.map { item in
                TerminalHints.BadgeRequest(cells: item.spans.map { ($0.row, $0.range.location, NSMaxRange($0.range)) },
                                           label: 2, command: item.kind == .command)
            }, columns: columns)
            return zip(items, placed).map { ($0, $1.start, $1.side.rawValue) }
        }
        func labelRules(_ placed: [(item: TerminalHintItem, start: Int, side: String)], _ name: String) {
            for (index, label) in placed.enumerated() {
                let row = label.item.spans[0].row, range = label.start..<(label.start + 2)
                func hits(_ item: TerminalHintItem) -> Bool {
                    item.spans.contains { $0.row == row && ($0.range.location..<NSMaxRange($0.range)).overlaps(range) }
                }
                check(!hits(label.item), "\(name): label never covers its own item (\(label.item.value))")
                check(!placed.enumerated().contains { $0.offset != index && $0.element.item.spans[0].row == row
                    && ($0.element.start..<($0.element.start + 2)).overlaps(range) }, "\(name): labels never overlap")
                if !label.side.hasSuffix("cover") {
                    check(!placed.contains { $0.item.kind != .command && $0.item != label.item && hits($0.item) },
                          "\(name): label stays off other items (\(label.item.value))")
                }
            }
        }
        let sheetLabels = badges(sheet, columns: 95)
        labelRules(sheetLabels, "sheet")
        check(sheetLabels.allSatisfy { $0.side == "left" }, "every sheet label fits on its item's left")
        check(sheetLabels.filter { $0.item.spans[0].row == 2 }.map { "\($0.item.value.prefix(4)) \($0.start)" }
              == ["find 2", ". 7", "./ve 45", "./.d 69"], "a command's label covers its prompt marker; `.` its left cells")
        let edge = badges(["192.0.2.1 and 198.51.100.2", "  (198.51.100.3)"], columns: 40)
        labelRules(edge, "edge")
        check(edge.map { "\($0.side) \($0.start)" } == ["right 9", "left 12", "left 1"],
              "an item at column 0 takes its right; parentheses may be covered")
        let tight = badges(["192.0.2.1 198.51.100.2"], columns: 40)
        labelRules(tight, "tight")
        check(tight.map { "\($0.side) \($0.start)" } == ["right-cover 9", "right 22"],
              "no free cell beside an item: cover a neighbour's text, never its own")
        let labels = TerminalHints.labels(count: 900)
        check(Set(labels).count == 900 && labels.allSatisfy { $0.count == 3 }, "prefix-free labels above two-key capacity")
        print("hint detector: boundary checks passed")
    }

    /// A coding-agent answer: two-space indent, hard-wrapped by the agent at
    /// the pane width (real newlines with the indent repeated).
    static let sheet = [
        "  Right, first up, the big one you'd paste into the prompt:",
        "",
        #"  ! find . -type f -name "*.swift" -not -path "./vendor/*" -not -path "./.dev/*" -exec grep -l"#,
        #"  "TerminalHint" {} + | xargs -I{} sh -c 'echo "== {}"; wc -l "{}"' | sort -t: -k2 -n | tee"#,
        "  /tmp/hints-report.txt",
        "",
        "  The docs live at",
        "  https://docs.example.invalid/azulejo/hints?section=terminal&lang=en-GB#visible-items, and you",
        "  can find the mirror if you squint hard enough.",
        "",
        "  Plain domain, no scheme: mirror.example.org",
        "",
        #"  $ cargo build --release --manifest-path shared/Cargo.toml --features "hints,mirror" && ls"#,
        "  -lah target/release",
        "",
        "  The flaky box sits at 192.0.2.47, and its noisy neighbour is at 198.51.100.12 on port 8443",
        "  (198.51.100.12:8443).",
        "",
        "  Session UUID: 3f9c2a7e-8b41-4d6f-a0c3-5e17b9d2f864",
        "",
        "  That bug got squashed in commit 7c3e9a1f4b2d8e05a6f1c9b3d7e2a4f80b5c6d19 (short form",
        "  7c3e9a1).",
        "",
        "  Check out feature/terminal-hints-v2 before you rebase onto release/2026.11, or just stay on",
        "  main if you're feeling brave.",
    ]

    static func checkSheet(directory: URL, _ check: (Bool, String) -> Void) throws {
        try FileManager.default.createDirectory(at: directory.appendingPathComponent("docs"), withIntermediateDirectories: true)
        func detect(_ rows: [String], columns: Int? = nil, branch: String? = nil) -> [TerminalHintItem] {
            TerminalHints.detectWrapped(rows: rows, joins: [], cwd: directory.path, local: true, branch: branch, columns: columns)
        }
        func spans(_ item: TerminalHintItem?) -> [String] {
            (item?.spans ?? []).map { "\($0.row):\($0.range.location)+\($0.range.length)" }
        }
        let find = #"find . -type f -name "*.swift" -not -path "./vendor/*" -not -path "./.dev/*" -exec grep -l "#
            + #""TerminalHint" {} + | xargs -I{} sh -c 'echo "== {}"; wc -l "{}"' | sort -t: -k2 -n | tee /tmp/hints-report.txt"#
        let cargo = #"cargo build --release --manifest-path shared/Cargo.toml --features "hints,mirror" && ls -lah target/release"#
        let expected: [(TerminalHintItem.Kind, String)] = [
            (.command, find), (.path, "."), (.path, "./vendor/*"), (.path, "./.dev/*"), (.path, "/tmp/hints-report.txt"),
            (.url, "https://docs.example.invalid/azulejo/hints?section=terminal&lang=en-GB#visible-items"),
            (.domain, "mirror.example.org"),
            (.command, cargo), (.path, "shared/Cargo.toml"), (.path, "target/release"),
            (.ip, "192.0.2.47"), (.ip, "198.51.100.12"), (.ip, "198.51.100.12:8443"),
            (.uuid, "3f9c2a7e-8b41-4d6f-a0c3-5e17b9d2f864"),
            (.hash, "7c3e9a1f4b2d8e05a6f1c9b3d7e2a4f80b5c6d19"), (.hash, "7c3e9a1"),
            (.path, "feature/terminal-hints-v2"), (.path, "release/2026.11"), (.branch, "main"),
        ]
        // The agent wrapped at 95 cells: exactly the pane, or a pane a
        // little wider when the wrapper keeps a right margin.
        for columns in [95, 97] {
            let items = detect(sheet, columns: columns, branch: "main")
            let got = items.map { "\($0.kind.rawValue) \($0.value)" }
            check(got == expected.map { "\($0.0.rawValue) \($0.1)" }, "sheet items at \(columns) columns: \(got)")
            check(spans(items.first { $0.value == find }) == ["2:4+90", "3:2+89", "4:2+21"], "agent command spans three rows")
            check(spans(items.first { $0.value == cargo }) == ["12:4+87", "13:2+19"], "prompted command spans two rows")
            check(spans(items.first { $0.value == "198.51.100.12:8443" }) == ["16:3+18"], "ip:port drops parentheses and period")
        }
        // Without the pane width only explicit continuations join.
        check(detect(sheet).filter { $0.kind == .command }.map(\.value)
              == [#"find . -type f -name "*.swift" -not -path "./vendor/*" -not -path "./.dev/*" -exec grep -l"#,
                  #"cargo build --release --manifest-path shared/Cargo.toml --features "hints,mirror" && ls"#],
              "hard-wrap joins need the column count")
        check(!detect(sheet, columns: 95).contains { $0.value == "docs" }, "existing bare word in prose")

        let shell = detect(["$ docker run \\", "> --rm example", "$ cat log.txt |", "  grep -c error",
                            #"$ echo "one"#, #"two" && true"#, "total 48"], columns: 30)
        check(shell.filter { $0.kind == .command }.map(\.value)
              == ["docker run --rm example", "cat log.txt | grep -c error", #"echo "one two" && true"#],
              "backslash, pipe and open-quote continuations")
        check(spans(shell.first) == ["0:2+12", "1:2+12"], "continuation highlight skips the secondary prompt")
        check(detect(["! printf it's", "output row", "another row"]).first?.value == "printf it's",
              "a quote that never closes doesn't swallow later rows")
        let edge = "$ printf '%s\\n' first second third fourth"
        check(detect([edge, "total 48"], columns: edge.count).first { $0.kind == .command }?.spans.count == 1,
              "shell rows at column zero never hard-wrap")
        check(detect(["  $ printf one", "  two"], columns: 80).first?.value == "printf one",
              "short rows don't join the next row")
        check(detect(["  $ printf one two three", "    four"], columns: 24).first?.value == "printf one two three",
              "a different indent ends the command")

        let url = "https://example.invalid/" + String(repeating: "seg/", count: 10) + "end"
        let head = "  see " + url.prefix(34), tail = "  " + url.dropFirst(34) + " for details"
        let cut = detect([head, tail], columns: 40)
        check(cut.map(\.value) == [url] && spans(cut.first) == ["0:6+34", "1:2+33"], "URL cut at the right edge")
        let shortPath = String(repeating: "x", count: 32) + " ./src/a"
        check(detect(["  " + shortPath, "  word"], columns: 42).map(\.value) == ["./src/a"],
              "a short path at the edge stays separate")

        let negatives = detect(["Version 2026.11 or v1.2.3, see file.txt, README.md, hints.sh, Cargo.toml, e.g. this.",
                                "Words deadbeef facade cafebabe and numbers 1234567 2026111 999.1.1.1 1.2.3.4.5",
                                "Times 12:34:56, MAC aa:bb:cc:dd:ee:ff, std::vector, mail dev@example.invalid",
                                "docs mention the docs folder"])
        check(negatives.isEmpty, "no domains, IPs or hashes in ordinary text: \(negatives.map(\.value))")
        let more = detect(["hosts 2001:db8::1, [2001:db8::2]:8443 and ::ffff:192.0.2.9 or mirror.example.org:8080.",
                           "https://192.0.2.10:8080/x mirror.example.com/path/x abcdef0 (feature/hints.)",
                           "ls docs", "main.rs is not main"], branch: "main")
        check(more.map { "\($0.kind.rawValue) \($0.value)" }
              == ["ip 2001:db8::1", "ip [2001:db8::2]:8443", "ip ::ffff:192.0.2.9", "domain mirror.example.org:8080",
                  "url https://192.0.2.10:8080/x", "path mirror.example.com/path/x", "hash abcdef0",
                  "path feature/hints", "command ls docs", "path docs", "branch main"],
              "IPv6, ports, longest overlap and command context: \(more.map { "\($0.kind.rawValue) \($0.value)" })")
        check(detect(["Merged into feature/hints.", "(feature/hints)"], branch: "feature/hints").map(\.value)
              == ["feature/hints", "feature/hints"], "branch names drop sentence punctuation")
    }
}
