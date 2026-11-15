//! Shell words and whether they read as a command: the vocabulary, the
//! shape of a command line and the prose cues around commands named in a
//! sentence.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::LazyLock;

use fancy_regex::Regex;

use super::unicode::{graphemes, is_hspace, path_extension};

fn set(words: &'static str) -> HashSet<&'static str> {
    words.split_whitespace().collect()
}

// Recognition vocabulary, not an execution allowlist. Never consult PATH:
// visible commands may target a different OS or a remote machine.
// These names rarely open an English sentence, so a line that starts
// with one reads as a command unless the rest reads as prose.
static COMMANDS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    set("
    sudo doas su env xargs sh bash zsh fish dash ksh csh tcsh nu pwsh
    ls cd pwd cat grep egrep fgrep rg ag fd awk gawk sed uniq wc tr tee printf echo
    uname whoami hostname uptime stat diff strings od hexdump xxd base32 base64
    md5 md5sum shasum sha1sum sha256sum sha512sum b2sum cksum unset alias unalias exec tput
    cp mv rm mkdir rmdir chmod chown chgrp chattr lsattr ln readlink realpath basename dirname
    mktemp envsubst printenv nohup timeout renice ionice taskset parallel fmt nl rev shuf tac seq
    expr bc numfmt comm cmp csplit iconv dos2unix truncate fallocate mkfifo shred wipefs dd pv
    du df umount ps htop atop iotop btop glances killall pkill pgrep pidof fg bg disown
    vmstat mpstat iostat sar lsblk blkid findmnt lscpu nproc dmesg nvidia-smi w who lastlog
    tar gzip gunzip zcat zip unzip xz unxz bzip2 bunzip2 zstd lz4 7z 7za unrar cpio
    curl wget aria2c ssh scp sftp rsync mosh autossh sshfs ssh-keygen ssh-add ssh-copy-id ssh-agent
    ping ping6 traceroute tracepath mtr dig nslookup whois ifconfig ss netstat lsof route arp
    nc ncat netcat socat telnet nmap tcpdump tshark iperf3 openssl gpg gpg2 xh httpie grpcurl websocat
    git hg svn fossil git-lfs gh glab tig jj lazygit
    make cmake ninja meson bazel bazelisk scons autoreconf autoconf automake libtool pkg-config
    gcc clang cc c++ g++ ld ldd otool nm objdump readelf addr2line ranlib ccache sccache
    clang-format clang-tidy cppcheck gdb lldb valgrind strace ltrace perf dtrace dtruss
    rustc cargo rustup rustfmt wasm-pack miri
    python python3 python2 pip pip3 pipx uv uvx poetry pdm hatch conda mamba micromamba pyenv
    ipython jupyter pytest tox nox ruff mypy isort flake8 pylint pre-commit
    node npm npx pnpm yarn bun bunx deno corepack nvm fnm volta tsc ts-node tsx vite webpack
    esbuild rollup parcel nuxt astro jest vitest mocha playwright cypress eslint prettier biome
    turbo nx lerna nodemon pm2
    ruby gem irb rails rake rspec bundler rbenv php composer artisan perl cpan lua luarocks
    java javac jar gradle gradlew mvn kotlin kotlinc scala sbt lein clj elixir iex erl rebar3
    ghc ghci cabal opam dune ocaml julia Rscript zig nim dart flutter gofmt golangci-lint
    dotnet nuget msbuild asdf mise direnv
    docker docker-compose podman podman-compose buildah skopeo crictl ctr nerdctl colima limactl
    kubectl kubectx kubens k9s minikube k3d k3s helm helmfile kustomize skaffold istioctl argocd
    terraform tofu pulumi packer vagrant consul vault nomad ansible ansible-playbook ansible-galaxy
    aws gcloud gsutil az doctl flyctl heroku vercel netlify wrangler firebase supabase
    psql pg_dump pg_restore createdb dropdb mysql mysqldump redis-cli mongosh sqlite3 duckdb etcdctl
    jq yq fx gron xmllint mlr sd fzf zoxide bat batcat eza exa lsd ncdu dust duf procs nnn ranger
    hyperfine entr watchexec supervisorctl tmux zellij byobu vim nvim vi nano emacs hx subl
    brew plutil launchctl diskutil hdiutil softwareupdate xcodebuild xcrun xcode-select xcodes
    swift swiftc swift-format swiftlint pod carthage fastlane tuist codesign spctl xattr
    mdfind mdls mdutil sips afplay caffeinate pmset networksetup scutil sw_vers system_profiler
    tmutil ditto pkgutil lipo install_name_tool dsymutil atos osascript pbcopy pbpaste mas
    apt apt-get apt-cache aptitude dpkg yum dnf rpm pacman yay paru zypper apk emerge
    nix nix-env nix-shell nix-build nixos-rebuild home-manager guix flatpak snap
    systemctl journalctl timedatectl hostnamectl loginctl networkctl resolvectl nmcli iwconfig
    iptables nft ufw firewall-cmd sysctl modprobe lsmod insmod rmmod lspci lsusb dmidecode smartctl
    fdisk parted mkfs mkswap swapon swapoff fsck e2fsck cryptsetup zfs zpool btrfs chroot unshare nsenter
    usermod useradd userdel groupadd passwd visudo crontab reboot halt poweroff
    tailscale wg wg-quick openvpn cloudflared ngrok mkcert certbot sops gopass rclone restic borg
    ffmpeg ffprobe magick exiftool optipng pngquant yt-dlp pandoc latexmk pdflatex xelatex typst
    mdbook hugo jekyll zola mkdocs sphinx-build doxygen
    just claude codex aider ollama azj herdr ghostty
    ")
});

/// Command names that are also everyday words ("find out more", "test
/// suite passes"). On a line of their own they need the shape of a
/// command: a flag, a path, an operator, an assignment.
static AMBIGUOUS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    set("
    head tail less more find cut sort test true false id date which command man file patch
    source export history read exit clear touch mount top kill sleep watch free host ip go
    port defaults open say service task convert time look join split paste yes column fold nice
    at install link tree code screen stack mix kind next black delta bundle log security trap
    wait jobs for while if until case last users groups cal age pass step op fly http trunk cross
    ")
});

static WINDOWS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    set("
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
    ")
});

static AMBIGUOUS_WINDOWS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    set("
    copy move type start call set path help time date title color pause prompt where net ver vol
    erase rename echo exit
    ")
});

/// Log levels after a tool's name: `npm WARN …` is output, not a command.
const LOG_LEVELS: &[&str] = &[
    "WARN", "ERR!", "ERROR", "INFO", "DEBUG", "TRACE", "WARNING", "FATAL", "notice", "warn",
    "error", "info", "verb", "sill", "timing",
];

/// Common English function words; shell commands rarely use them bare.
static STOPWORDS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    set("
    the a an is are was were be been you your we our they their it its this that these those
    should would could will can may might must please also not but so because when what why how
    there here has have had does did than into about to of and or with from on at by as
    it's i'm don't let's you're i'll we'll that's there's
    ")
});

/// Words that introduce a command in a sentence: "run it with npx vitest".
pub const CUES: &[&str] = &[
    "run", "use", "using", "with", "try", "type", "execute", "via", "running",
];
/// Weaker cues: the command must also end the sentence ("then just lint.").
const LATE_CUES: &[&str] = &["then", "and"];

/// Words that end a command named in a sentence.
static ENDERS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    set("
    and or to then which so but if before after first again instead from when while until because
    the a an is are it this that you we in on for at
    ")
});

/// Code fences whose every line is a command. Session fences
/// (`console`) mix prompts and output, so they read like a terminal.
pub const SHELL_FENCES: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "fish",
    "shell",
    "ps1",
    "powershell",
    "pwsh",
    "cmd",
    "bat",
    "batch",
];

const SCRIPT_SUFFIXES: &[&str] = &[
    "sh", "bash", "zsh", "py", "rb", "pl", "js", "ts", "command", "ps1", "bat", "cmd",
];

pub(crate) fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap()
}

/// Whether the pattern matches somewhere in `text`.
pub(crate) fn matches(regex: &Regex, text: &str) -> bool {
    regex.is_match(text).unwrap_or(false)
}

/// Swift's `String.range(of:options: .regularExpression)`, which the
/// macOS detector used for a few whole-word tests, matches whole graphemes:
/// a letter with a combining mark is a word character but not an ASCII
/// letter. Multi-scalar graphemes stand in as one character of their class.
pub(crate) fn matches_graphemes(regex: &Regex, text: &str) -> bool {
    use unicode_segmentation::UnicodeSegmentation;
    if text.is_ascii() {
        return matches(regex, text);
    }
    let mut mapped = String::with_capacity(text.len());
    for g in text.graphemes(true) {
        let mut chars = g.chars();
        let first = chars.next().unwrap();
        mapped.push(if chars.next().is_none() {
            first
        } else if first.is_alphanumeric() || first == '_' {
            'é'
        } else if first.is_whitespace() {
            ' '
        } else {
            '¤'
        });
    }
    matches(regex, &mapped)
}

static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Za-z']{2,}[.!?]$"));
static IDENTIFIER: LazyLock<Regex> = LazyLock::new(|| re(r"^[a-z_][A-Za-z0-9_.+-]*$"));
/// Usage synopses aren't runnable: `[-v | --version]`, `<command>`.
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:\[-|<[A-Za-z][\w-]*>)"));
/// A diff's added or removed line: `+    run: npm test`.
pub(crate) static DIFF_LINE: LazyLock<Regex> = LazyLock::new(|| re(r"^[+-][ \t]{2,}"));
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Za-z_][A-Za-z0-9_]*="));

/// A shell word: a byte range of its text and whether it has quotes.
#[derive(Debug, Clone)]
pub struct Token<'a> {
    pub range: Range<usize>,
    pub text: &'a str,
    pub quoted: bool,
}

/// How sure a context makes us that its text is a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// After a shell prompt, in a shell code block or a permission box.
    Prompt,
    /// Inside backticks.
    Backtick,
    /// A run of one colour inside a row (agents style inline code).
    ColorRun,
    /// A whole plain line, optionally a list item.
    Plain,
    /// A whole line on a row that has colours: they mark the code.
    Colored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Strong,
    Weak,
}

/// Shell words: whitespace outside quotes separates them. An apostrophe
/// between letters ("it's") is prose, not a quote.
pub fn tokens(text: &str, range: Range<usize>) -> Vec<Token<'_>> {
    let b = text.as_bytes();
    let end = range.end;
    let mut result = Vec::new();
    let mut i = range.start;
    let letter = |c: u8| c.is_ascii_alphabetic();
    while i < end {
        while i < end && (b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
        if i >= end {
            break;
        }
        let start = i;
        let mut quote = 0u8;
        let mut quoted = false;
        while i < end {
            let c = b[i];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                } else if c == b'\\' && quote != b'\'' {
                    i += 1;
                }
                i += 1;
                continue;
            }
            if c == b' ' || c == b'\t' {
                break;
            }
            if c == b'\'' && i > start && i + 1 < end && letter(b[i - 1]) && letter(b[i + 1]) {
                i += 1;
                continue;
            }
            if c == b'\'' || c == b'"' || c == b'`' {
                quote = c;
                quoted = true;
            } else if c == b'\\' {
                i += 1;
            }
            i += 1;
        }
        // A backslash may have stepped into a character: finish it.
        i = i.min(end);
        while !text.is_char_boundary(i) {
            i += 1;
        }
        result.push(Token {
            range: start..i,
            text: &text[start..i],
            quoted,
        });
    }
    result
}

pub fn tier(word: &str) -> Option<Tier> {
    if COMMANDS.contains(word) {
        return Some(Tier::Strong);
    }
    if AMBIGUOUS.contains(word) {
        return Some(Tier::Weak);
    }
    let lower = word.to_lowercase();
    if let Some(name) = lower.strip_suffix(".exe") {
        return (COMMANDS.contains(name) || AMBIGUOUS.contains(name) || WINDOWS.contains(name))
            .then_some(Tier::Strong);
    }
    // Windows names match in any case, but prose capitalises words
    // ("Copy the file"): only lower case, upper case or Verb-Noun.
    if !WINDOWS.contains(lower.as_str())
        || !(word == lower || word == word.to_uppercase() || word.contains('-'))
    {
        return None;
    }
    Some(if AMBIGUOUS_WINDOWS.contains(lower.as_str()) {
        Tier::Weak
    } else {
        Tier::Strong
    })
}

pub fn assignment(word: &str) -> bool {
    matches(&ASSIGNMENT, word)
}

/// `./build.sh`, `~/bin/tool`, `/usr/bin/env`, but not `path:12:` from
/// a compiler message.
pub fn path_executable(word: &str) -> bool {
    if !(word.starts_with("./")
        || word.starts_with("../")
        || word.starts_with("~/")
        || (word.starts_with('/') && !word.starts_with("//")))
    {
        return false;
    }
    !word.ends_with(':') && !word.ends_with('/') && !word.contains("://") && !word.contains(':')
}

pub fn script(word: &str) -> bool {
    let ext = path_extension(word).to_lowercase();
    SCRIPT_SUFFIXES.contains(&ext.as_str())
}

/// The shape of a command: a flag, an operator, a path, an assignment,
/// a variable, a glob or a quoted argument.
pub fn evidence<'a>(words: impl IntoIterator<Item = &'a Token<'a>>) -> bool {
    words.into_iter().any(|word| {
        if word.quoted {
            return true;
        }
        let s = word.text;
        if graphemes(s) >= 2 && s.starts_with('-') {
            return true;
        }
        if ["|", "||", "&&", ";", ">", ">>", "<", "&", "2>&1", ".", ".."].contains(&s) {
            return true;
        }
        if s.ends_with(';') || s.starts_with("<<") || s.starts_with('>') || s.starts_with("2>") {
            return true;
        }
        if s.contains('$') || s.contains('*') || (graphemes(s) > 1 && s.contains('=')) {
            return true;
        }
        s.contains('/') && !s.contains("://")
    })
}

fn edge_punctuation(c: char) -> bool {
    ".,;:!?()\"".contains(c)
}

/// Reads as a sentence: English function words, or a final word with a
/// full stop.
fn prose(words: &[Token], shaped: bool) -> bool {
    let stops = stop_count(words);
    if let Some(last) = words.last()
        && !last.quoted
        && matches(&SENTENCE_END, last.text)
    {
        return true;
    }
    stops >= 2 || (stops == 1 && !shaped && words.len() >= 3)
}

fn stop_count(words: &[Token]) -> usize {
    words
        .iter()
        .filter(|w| {
            !w.quoted && STOPWORDS.contains(w.text.to_lowercase().trim_matches(edge_punctuation))
        })
        .count()
}

/// Whether words read as a command in a context.
pub fn accepts(words: &[Token], context: Context) -> bool {
    let mut index = 0;
    while index + 1 < words.len() && assignment(words[index].text) {
        index += 1;
    }
    if index >= words.len() {
        return false;
    }
    let first = words[index].text;
    let rest = &words[index + 1..];
    if assignment(first) || first.ends_with(':') || first.ends_with(',') {
        return false;
    }
    // Interface hints: "! for bash mode · / for commands".
    if words.iter().any(|w| w.text == "·") {
        return false;
    }
    let line = matches!(context, Context::Plain | Context::Colored);
    if line && let Some(second) = rest.first() {
        // Output: `npm WARN …`, `test result: ok`, `diff --git a/x b/x`.
        if LOG_LEVELS.contains(&second.text)
            || (second.text.ends_with(':') && !second.text.ends_with("::"))
        {
            return false;
        }
        if first == "diff" && second.text == "--git" {
            return false;
        }
    }
    if rest
        .iter()
        .any(|w| !w.quoted && matches(&PLACEHOLDER, w.text))
    {
        return false;
    }
    // A lone word on a line is a listing more often than a command.
    if rest.is_empty() && line {
        return false;
    }
    let shaped = evidence(rest);
    let wordy = prose(rest, shaped);
    match tier(first) {
        Some(Tier::Strong) => matches!(context, Context::Prompt | Context::Backtick) || !wordy,
        Some(Tier::Weak) => match context {
            Context::Prompt => !wordy || shaped,
            Context::Backtick | Context::ColorRun => !rest.is_empty() && !wordy,
            // "find the logs under ~/Library": everyday words need a pure shape.
            Context::Plain => shaped && !wordy && stop_count(rest) == 0,
            Context::Colored => false,
        },
        None => {
            if path_executable(first) {
                if matches!(context, Context::ColorRun | Context::Colored) {
                    return !rest.is_empty() && !wordy;
                }
                // `./configure --prefix=…`, `~/bin/tool -v`; not `~/proj main*`
                // or a source file named in output.
                if context == Context::Plain {
                    let named = script(first) || path_extension(first).is_empty();
                    return named
                        && (script(first)
                            || first.starts_with("./")
                            || rest.iter().any(|w| w.text.starts_with('-')))
                        && !wordy;
                }
                return (!rest.is_empty() || script(first)) && !wordy;
            }
            let name = ["!", "(", "{", "[", "[["].contains(&first) || matches(&IDENTIFIER, first);
            match context {
                Context::Prompt => name && !wordy,
                Context::Backtick | Context::ColorRun => {
                    name && rest
                        .iter()
                        .any(|w| graphemes(w.text) > 1 && w.text.starts_with('-'))
                        && !wordy
                }
                Context::Plain | Context::Colored => false,
            }
        }
    }
}

/// Commands named in a sentence: "open a tunnel with ssh -L 8443:…".
/// A cue word introduces them; a function word or the sentence's
/// punctuation ends them. The flag says the command reached the end.
pub fn prose_commands(text: &str) -> Vec<(Range<usize>, bool)> {
    if matches(&DIFF_LINE, text) {
        return Vec::new();
    }
    let words = tokens(text, 0..text.len());
    let mut result = Vec::new();
    let mut index = 1;
    while index < words.len() {
        let cue = words[index - 1].text.to_lowercase();
        // "Clone the repo: git clone …", "- run: npm test": after a
        // label, the rest of the line, if it reads as a command.
        if graphemes(&cue) > 1
            && let Some(label) = cue.strip_suffix(':')
            && label.chars().all(char::is_alphabetic)
        {
            let start = words[index].range.start;
            let mut end = text.len();
            while end > start {
                let c = text[..end].chars().next_back().unwrap();
                if !(is_hspace(c) || c == '.') {
                    break;
                }
                end -= c.len_utf8();
            }
            let range = start..end;
            if let Some(word) = words[index..].iter().find(|w| !assignment(w.text))
                && tier(word.text) == Some(Tier::Strong)
                && !text[range.clone()].contains("   ")
                && accepts(&tokens(text, range.clone()), Context::Plain)
            {
                result.push((range, false));
                break;
            }
            index += 1;
            continue;
        }
        let late = LATE_CUES.contains(&cue.as_str());
        if !(CUES.contains(&cue.as_str()) || late) {
            index += 1;
            continue;
        }
        let mut name = index;
        while name + 1 < words.len() && assignment(words[name].text) {
            name += 1;
        }
        let first = &words[name];
        let level = match (first.quoted, tier(first.text)) {
            (false, Some(level)) => level,
            _ => {
                index += 1;
                continue;
            }
        };
        let mut end = first.range.end;
        let mut args: Vec<&Token> = Vec::new();
        let mut closed = false;
        let mut next = name + 1;
        while next < words.len() {
            let word = &words[next];
            if word.text.starts_with('`')
                || (!word.quoted && ENDERS.contains(word.text.to_lowercase().as_str()))
            {
                closed = true;
                break;
            }
            // A column gap ends it: a build step's timing (`RUN … 18.4s`).
            if word.range.start - words[next - 1].range.end >= 3 {
                closed = true;
                break;
            }
            let mut length = word.range.len();
            if let Some(last) = word.text.chars().next_back()
                && graphemes(word.text) > 1
            {
                // Sentence punctuation, or a closing parenthesis it didn't open.
                if ".,;:!?".contains(last) || (last == ')' && !word.text.contains('(')) {
                    length -= 1;
                    closed = true;
                }
            }
            end = word.range.start + length;
            args.push(word);
            next += 1;
            if closed {
                break;
            }
        }
        let ok = if level == Tier::Strong {
            !args.is_empty()
        } else {
            evidence(args.iter().copied())
        };
        if !ok || (late && next != words.len()) {
            index += 1;
            continue;
        }
        result.push((
            words[index].range.start..end,
            !closed && next == words.len(),
        ));
        index = next + 1;
    }
    result
}
