//! Row text helpers shared by the features and the span builder.

/// Recognition vocabulary, not an execution allowlist: a feature for the
/// model and a tie-breaker when finding where a prompt ends. Unknown
/// commands still work, through the other features.
pub fn is_command_word(word: &str) -> bool {
    let word = word.strip_suffix(".exe").unwrap_or(word);
    if word.is_empty() || word.len() > 24 {
        return false;
    }
    let lower;
    let word = if word.bytes().any(|b| b.is_ascii_uppercase()) {
        lower = word.to_ascii_lowercase();
        lower.as_str()
    } else {
        word
    };
    VOCAB.binary_search(&word).is_ok()
}

/// Sorted; `vocab_is_sorted` checks it.
const VOCAB: &[&str] = &[
    "act",
    "adb",
    "add-content",
    "ag",
    "alias",
    "ansible",
    "ansible-playbook",
    "apk",
    "apt",
    "apt-cache",
    "apt-get",
    "asdf",
    "attrib",
    "awk",
    "aws",
    "az",
    "base64",
    "bash",
    "bat",
    "bazel",
    "bc",
    "black",
    "blkid",
    "borg",
    "brew",
    "bun",
    "bundle",
    "bunx",
    "bzip2",
    "c++",
    "cabal",
    "call",
    "cargo",
    "cat",
    "cc",
    "cd",
    "certbot",
    "chdir",
    "chgrp",
    "chmod",
    "choco",
    "chown",
    "clang",
    "clear",
    "cls",
    "cmake",
    "cmd",
    "code",
    "command",
    "composer",
    "conda",
    "convert",
    "copy",
    "copy-item",
    "corepack",
    "cp",
    "crontab",
    "csh",
    "curl",
    "cut",
    "dart",
    "dash",
    "date",
    "dd",
    "defaults",
    "del",
    "deno",
    "df",
    "diff",
    "dig",
    "dir",
    "direnv",
    "diskutil",
    "dmesg",
    "dnf",
    "docker",
    "docker-compose",
    "doctl",
    "dotnet",
    "dpkg",
    "du",
    "echo",
    "emacs",
    "env",
    "eslint",
    "exa",
    "exec",
    "exit",
    "expand-archive",
    "export",
    "eza",
    "fd",
    "ffmpeg",
    "ffprobe",
    "file",
    "find",
    "findstr",
    "fish",
    "flutter",
    "fly",
    "free",
    "fzf",
    "g++",
    "gcc",
    "gcloud",
    "gdb",
    "gem",
    "get-childitem",
    "get-content",
    "get-process",
    "get-service",
    "gh",
    "git",
    "glab",
    "go",
    "gpg",
    "gradle",
    "grep",
    "gunzip",
    "gzip",
    "hdiutil",
    "head",
    "helm",
    "heroku",
    "hexdump",
    "hg",
    "history",
    "host",
    "hostname",
    "htop",
    "http",
    "id",
    "ifconfig",
    "invoke-webrequest",
    "ip",
    "ipconfig",
    "iptables",
    "java",
    "javac",
    "jest",
    "journalctl",
    "jq",
    "just",
    "k9s",
    "kill",
    "killall",
    "kind",
    "kotlin",
    "ksh",
    "kubectl",
    "launchctl",
    "ldd",
    "less",
    "lldb",
    "ln",
    "ls",
    "lsblk",
    "lsof",
    "make",
    "man",
    "md5sum",
    "meson",
    "minikube",
    "mise",
    "mkdir",
    "more",
    "mount",
    "move",
    "mv",
    "mvn",
    "mypy",
    "mysql",
    "nano",
    "nc",
    "net",
    "netsh",
    "netstat",
    "nginx",
    "ninja",
    "nix",
    "nix-shell",
    "nmap",
    "node",
    "nohup",
    "npm",
    "npx",
    "nslookup",
    "nvim",
    "nvm",
    "objdump",
    "od",
    "open",
    "openssl",
    "osascript",
    "otool",
    "pacman",
    "patch",
    "pbcopy",
    "pbpaste",
    "perl",
    "php",
    "pip",
    "pip3",
    "pipx",
    "pkill",
    "pnpm",
    "podman",
    "poetry",
    "popd",
    "port",
    "powershell",
    "printf",
    "ps",
    "psql",
    "pulumi",
    "pushd",
    "pwd",
    "pwsh",
    "pyenv",
    "pytest",
    "python",
    "python3",
    "rails",
    "rake",
    "rclone",
    "read",
    "readlink",
    "realpath",
    "redis-cli",
    "remove-item",
    "rg",
    "rm",
    "rmdir",
    "robocopy",
    "rpm",
    "rspec",
    "rsync",
    "ruby",
    "ruff",
    "rustc",
    "rustup",
    "say",
    "sbt",
    "scp",
    "screen",
    "sed",
    "select-string",
    "service",
    "set",
    "set-location",
    "setx",
    "sftp",
    "sh",
    "sha256sum",
    "shasum",
    "sleep",
    "softwareupdate",
    "sort",
    "source",
    "sqlite3",
    "ss",
    "ssh",
    "ssh-keygen",
    "start",
    "stat",
    "strace",
    "strings",
    "su",
    "sudo",
    "svn",
    "swift",
    "swiftc",
    "sysctl",
    "systemctl",
    "tail",
    "tailscale",
    "tar",
    "task",
    "taskkill",
    "tasklist",
    "tcsh",
    "tee",
    "telnet",
    "terraform",
    "test",
    "time",
    "timeout",
    "tmux",
    "tofu",
    "top",
    "touch",
    "tr",
    "traceroute",
    "tree",
    "tsc",
    "type",
    "umount",
    "uname",
    "uniq",
    "unzip",
    "uptime",
    "uv",
    "uvx",
    "vagrant",
    "valgrind",
    "vault",
    "vim",
    "vitest",
    "vmstat",
    "watch",
    "wc",
    "wget",
    "where",
    "which",
    "whoami",
    "winget",
    "wsl",
    "xargs",
    "xcodebuild",
    "xcrun",
    "xxd",
    "xz",
    "yarn",
    "yq",
    "yum",
    "zig",
    "zip",
    "zsh",
    "zypper",
];

/// Characters that end a shell prompt when a space follows.
pub fn is_prompt_char(c: char) -> bool {
    matches!(
        c,
        '$' | '%' | '#' | '>' | '❯' | '➜' | 'λ' | '»' | '✗' | '➤' | '→' | '❱' | '▶' | '›'
    )
}

/// A bullet that starts a coding agent's transcript entry.
pub fn is_agent_bullet(c: char) -> bool {
    matches!(c, '●' | '⏺' | '◉' | '•')
}

pub fn is_box_vertical(c: char) -> bool {
    matches!(c, '│' | '┃' | '║' | '▏' | '▕' | '|')
}

pub fn is_box_char(c: char) -> bool {
    ('\u{2500}'..='\u{257F}').contains(&c) || ('\u{2580}'..='\u{259F}').contains(&c)
}

/// Where a prompt ends, if the row has one. `strength`: 3 when the word
/// after the prompt is a known command or a path, 2 when the text before
/// the prompt marker looks like a prompt (user@host, a directory, a git
/// segment), 1 otherwise, 0 for a bare prompt with nothing after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prompt {
    /// The marker character.
    pub marker: char,
    /// Byte offset of the marker.
    pub at: usize,
    /// Byte offset of the command text (the row's end for a bare prompt).
    pub start: usize,
    pub strength: u8,
}

/// Finds the prompt that best explains the row: the first marker followed
/// by a known command, else the first after prompt-like text, else the
/// first at all. Only the first 100 characters can hold a prompt.
pub fn prompt(row: &str) -> Option<Prompt> {
    let mut best: Option<Prompt> = None;
    let mut seen = 0;
    for (at, c) in row.char_indices() {
        seen += 1;
        if seen > 100 {
            break;
        }
        if !is_prompt_char(c) {
            continue;
        }
        let after = at + c.len_utf8();
        let next = row[after..].chars().next();
        if next.is_some_and(|n| n != ' ' && n != '\t') {
            continue;
        }
        let mut rest = row[after..].trim_start();
        if c == '➜' {
            rest = after_arrow_segments(rest);
        }
        let start = row.len() - rest.len();
        let prefix = row[..at].trim();
        let strength = if rest.is_empty() {
            0
        } else if word_starts_command(first_word(rest)) {
            3
        } else if prompt_like_prefix(prefix) {
            2
        } else {
            1
        };
        let candidate = Prompt {
            marker: c,
            at,
            start,
            strength,
        };
        if best.is_none_or(|b| strength > b.strength) {
            best = Some(candidate);
        }
        if strength == 3 {
            break;
        }
    }
    best
}

/// oh-my-zsh shows the folder and the git branch after its arrow:
/// `➜  proj git:(main) ✗ cmd`. Skips them when a command follows.
fn after_arrow_segments(rest: &str) -> &str {
    let mut words = rest.split_whitespace();
    let Some(folder) = words.next() else {
        return rest;
    };
    let mut next = rest[rest.find(folder).unwrap_or(0) + folder.len()..].trim_start();
    let mut segment = false;
    if let Some(w) = next.split_whitespace().next()
        && (w.starts_with("git:(")
            || w.starts_with("hg:(")
            || (w.starts_with('(') && w.ends_with(')')))
    {
        next = next[w.len()..].trim_start();
        segment = true;
    }
    if let Some(r) = next.strip_prefix('✗') {
        next = r.trim_start();
        segment = true;
    }
    let skip = segment || !word_starts_command(folder);
    if skip && !next.is_empty() && word_starts_command(first_word(next)) {
        next
    } else {
        rest
    }
}

fn prompt_like_prefix(prefix: &str) -> bool {
    prefix.is_empty()
        || prefix.contains(['@', '~', '/', ':', '\\', ')', ']'])
        || prefix.chars().all(|c| is_box_char(c) || c == ' ')
}

pub fn first_word(text: &str) -> &str {
    text.split_whitespace().next().unwrap_or("")
}

/// A word that can start a command: a known command, a path to a program
/// or an environment assignment.
pub fn word_starts_command(word: &str) -> bool {
    if is_command_word(word) {
        return true;
    }
    if word.starts_with("./") || word.starts_with("~/") || word.starts_with("../") {
        return word.len() > 2;
    }
    if let Some((name, _)) = word.split_once('=') {
        return !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b == b'_' || b.is_ascii_digit());
    }
    word.starts_with('/') && word.len() > 1 && word[1..].contains('/') && !word.ends_with('/')
}

/// The CC tool-call prefix (`● Bash(`), if the row starts with one, as a
/// byte offset just after the parenthesis.
pub fn bash_call(row: &str) -> Option<usize> {
    let trimmed = row.trim_start();
    let mut chars = trimmed.chars();
    let bullet = chars.next()?;
    if !is_agent_bullet(bullet) {
        return None;
    }
    let rest = chars.as_str().trim_start();
    let rest = rest.strip_prefix("Bash(")?;
    Some(row.len() - rest.len())
}

/// `● Name(`: an agent tool call; returns the tool name.
pub fn tool_call(row: &str) -> Option<&str> {
    let trimmed = row.trim_start();
    let mut chars = trimmed.chars();
    if !is_agent_bullet(chars.next()?) {
        return None;
    }
    let rest = chars.as_str().trim_start();
    let name_end = rest.find('(')?;
    let name = &rest[..name_end];
    (!name.is_empty()
        && name.len() <= 16
        && name.chars().all(|c| c.is_ascii_alphanumeric())
        && name.starts_with(|c: char| c.is_ascii_uppercase()))
    .then_some(name)
}

/// Length in bytes of a secondary prompt at the row's start (indent
/// included): `> `, zsh's `heredoc> `, `dquote> `, `pipe pipe> `.
pub fn ps2_len(row: &str) -> usize {
    let indent = row.len() - row.trim_start_matches([' ', '\t']).len();
    let mut rest = &row[indent..];
    loop {
        let before = rest;
        for word in [
            "pipe", "cmdand", "cmdor", "dquote", "quote", "bquote", "heredoc", "subsh", "cursh",
            "for", "while", "if", "then", "else", "function",
        ] {
            if let Some(r) = rest.strip_prefix(word) {
                rest = r.strip_prefix(' ').unwrap_or(r);
                break;
            }
        }
        if rest.len() == before.len() {
            break;
        }
    }
    if let Some(r) = rest.strip_prefix("> ") {
        row.len() - r.len()
    } else if rest.len() != row.len() - indent && rest.starts_with('>') {
        row.len() - rest[1..].len()
    } else {
        indent
    }
}

/// The row without box borders: `│ text │` gives `text`'s byte range.
pub fn unboxed(row: &str) -> (usize, usize) {
    let end = row.trim_end().len();
    let mut start = row.len() - row.trim_start().len();
    let mut stop = end;
    let mut it = row[start..stop].chars();
    if let Some(c) = it.next()
        && is_box_vertical(c)
        && c != '|'
    {
        start += c.len_utf8();
        if let Some(last) = row[start..stop].chars().next_back()
            && is_box_vertical(last)
            && last != '|'
        {
            stop -= last.len_utf8();
        }
    }
    let inner = &row[start..stop];
    let lead = inner.len() - inner.trim_start().len();
    let trail = inner.len() - inner.trim_end().len();
    (start + lead, stop - trail)
}

/// Count of odd-parity unescaped quotes: true if a quote is left open.
pub fn open_quote(text: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in text.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, c) {
            (None, '\\') | (Some('"'), '\\') => escaped = true,
            (None, '"') | (None, '\'') => quote = Some(c),
            (Some(q), c) if q == c => quote = None,
            _ => {}
        }
    }
    quote.is_some()
}

/// True if the text ends in an explicit shell continuation: an odd run of
/// backslashes, `|`, `&&` or `||`.
pub fn continues(text: &str) -> bool {
    let t = text.trim_end();
    if t.ends_with('|') || t.ends_with("&&") {
        return true;
    }
    t.chars().rev().take_while(|&c| c == '\\').count() % 2 == 1
}

/// UTF-16 length of a string.
pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// Byte offset of a UTF-16 offset (clamped to the string).
pub fn byte_at_utf16(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16();
    }
    text.len()
}

/// Terminal cells, roughly: wide East Asian and emoji count two.
pub fn cells(text: &str) -> usize {
    text.chars()
        .map(|c| match c as u32 {
            0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1FAFF
            | 0x20000..=0x3FFFD => 2,
            _ => 1,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocab_is_sorted() {
        assert!(
            VOCAB.windows(2).all(|w| w[0] < w[1]),
            "{:?}",
            VOCAB.windows(2).find(|w| w[0] >= w[1])
        );
    }

    #[test]
    fn prompts() {
        let cases = [
            ("user@devbox:~/src$ git status", Some("git status")),
            (
                "➜  webapp git:(main) ✗ ls -la ./scripts",
                Some("ls -la ./scripts"),
            ),
            ("➜  dash git:(main) npm run build", Some("npm run build")),
            ("➜  ~ ls", Some("ls")),
            ("➜  make test", Some("make test")),
            (
                "user@devbox ~/w/api (feature/x)> cargo build --release",
                Some("cargo build --release"),
            ),
            ("root@web-02:/etc/nginx# nginx -t", Some("nginx -t")),
            ("╰─❯ npm test", Some("npm test")),
            ("user@laptop ~ % brew update", Some("brew update")),
            ("$ ./deploy.sh", Some("./deploy.sh")),
            ("user@devbox:~$ ", Some("")),
        ];
        for (row, want) in cases {
            let got = prompt(row).map(|p| &row[p.start..]);
            assert_eq!(got, want, "{row}");
        }
        assert_eq!(prompt("plain words here"), None);
    }

    #[test]
    fn secondary_prompts() {
        assert_eq!(&"> -p 8080 \\"[ps2_len("> -p 8080 \\")..], "-p 8080 \\");
        assert_eq!(&"heredoc> EOF"[ps2_len("heredoc> EOF")..], "EOF");
        assert_eq!(&"dquote> two\""[ps2_len("dquote> two\"")..], "two\"");
        assert_eq!(&"  plain"[ps2_len("  plain")..], "plain");
    }

    #[test]
    fn boxes_and_calls() {
        let row = "│   rm -rf target   │";
        let (s, e) = unboxed(row);
        assert_eq!(&row[s..e], "rm -rf target");
        let call = "● Bash(git log -5)";
        assert_eq!(&call[bash_call(call).unwrap()..], "git log -5)");
        assert_eq!(tool_call("● Read(src/a.ts)"), Some("Read"));
        assert_eq!(tool_call("● The plan"), None);
    }

    #[test]
    fn utf16() {
        let s = "界👩e";
        assert_eq!(utf16_len(s), 4);
        assert_eq!(byte_at_utf16(s, 1), 3);
        assert_eq!(byte_at_utf16(s, 3), 7);
    }
}
