//! Items on logical lines: commands (decided while joining rows, inline in
//! backticks or a colour run, or named in a sentence) and the atoms around
//! them (URLs, paths, IPs, domains, hashes and IDs, branches).

use std::collections::HashSet;
use std::net::Ipv6Addr;
use std::ops::Range;
use std::str::FromStr;
use std::sync::LazyLock;

use fancy_regex::Regex;

use super::Kind;
use super::files::Files;
use super::lines::{SHOWN_CALL, TOOL_CALL};
use super::unicode::{char_at, char_before, is_control, trim_hspace};
use super::words::{
    CUES, Context, accepts, matches, matches_graphemes, prose_commands, re, script, tokens,
};

static IPV4: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?<![\w.:-])(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(?::(\d{1,5}))?(?![\w-]|\.\d|:\d)",
    )
});
static IPV6: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?<![\w:.\[])(\[)?([0-9A-Fa-f]{0,4}(?::[0-9A-Fa-f]{0,4}){2,7}(?:\.\d{1,3}){0,3})(\](?::(\d{1,5}))?)?(?![\w.]*[\w])",
    )
});
static DOMAIN: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?<![\w.@/:\\-])(?:[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?\.)+([A-Za-z]{2,24})(?::(\d{1,5}))?(?![\w@-]|\.[A-Za-z0-9])",
    )
});
static UUID: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?<![\w-])[0-9A-Fa-f]{8}(?:-[0-9A-Fa-f]{4}){3}-[0-9A-Fa-f]{12}(?![\w-])")
});
static HASH: LazyLock<Regex> = LazyLock::new(|| re(r"(?<![\w./:-])[0-9a-f]{7,40}(?![\w-]|\.\w)"));
static URLS: LazyLock<Regex> = LazyLock::new(|| re(r#"(?:https?://|mailto:|file://)[^\s<>"'`]+"#));
static PATHS: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?:"[^"\n]+"|'[^'\n]+'|(?:\\[^\n]|[^\s<>"'`|;,])+)"#));
static BRANCH_LINE: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*[*+] ([^\s]+)\s*$"));
static OTHER_BRANCH_LINE: LazyLock<Regex> = LazyLock::new(|| re(r"^  ([^\s]+)\s*$"));
static LOCATION_SUFFIX: LazyLock<Regex> = LazyLock::new(|| re(r":\d+(?::\d+)?$"));
static PATH_START: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^|[\(\[\{:=])(?=/|~/|\.\.?/)"));
static CODE: LazyLock<Regex> = LazyLock::new(|| re(r"`([^`\r\n]+)`"));
static FILENAME: LazyLock<Regex> = LazyLock::new(|| re(r"^[^/]*\.[A-Za-z0-9_-]{1,12}$"));
/// A tool's own update notice in its status area ("Update available! Run:
/// brew upgrade …"): advice about the tool, not a command on the screen.
static UPDATE_NOTICE: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)^[ \t]*(?:[^\w\s]+[ \t]*)?(?:an? )?(?:new )?update (?:is )?available\b")
});

/// Deliberately small: TLDs that double as common file extensions
/// (md, sh, py, rs, pl, in, so, cc, app, zip, ...) are left out so
/// filenames never look like hosts.
static TOP_LEVEL_DOMAINS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    "
    com org net edu gov mil int info biz name pro io dev ai co me tv xyz
    tech cloud site online page wiki blog shop news email link club
    design studio digital network software services company group team
    world today life media tools works zone systems host
    example invalid test localhost local internal lan home arpa onion
    uk de fr nl be eu us ca au nz jp cn kr tw hk sg ru ua br ar mx es pt
    it ch at se dk fi no ie is cz sk hu ro bg gr tr il za ly gg je fm
    "
    .split_whitespace()
    .collect()
});

/// An item on a logical line, in bytes of that line.
#[derive(Debug, Clone)]
pub struct Found {
    pub kind: Kind,
    pub line: usize,
    pub range: Range<usize>,
    pub value: String,
    pub open: Option<String>,
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start.max(b.start) < a.end.min(b.end)
}

/// An atom competing for its text: kind, range, value (when not the text
/// itself) and open target.
type Candidate = (Kind, Range<usize>, Option<String>, Option<String>);

fn port_ok(text: &str, group: Option<fancy_regex::Match>) -> bool {
    group.is_none_or(|m| text[m.range()].parse::<u32>().is_ok_and(|p| p <= 65535))
}

/// Items on the logical lines. `commands`: each line's command, decided
/// while joining rows; `colors`: each line's colour runs.
pub fn scan(
    rows: &[&str],
    commands: &[Option<Range<usize>>],
    colors: &[Vec<Range<usize>>],
    branch: Option<&str>,
    files: &mut Files,
) -> Vec<Found> {
    let mut result: Vec<Found> = Vec::new();
    // Recognise the rest of a plain `git branch` listing only when its
    // contiguous block includes an explicit current/worktree marker.
    let mut branch_rows: HashSet<usize> = HashSet::new();
    for (row, text) in rows.iter().enumerate() {
        if !matches(&BRANCH_LINE, text) {
            continue;
        }
        branch_rows.insert(row);
        for step in [-1i64, 1] {
            let mut next = row as i64 + step;
            while next >= 0
                && (next as usize) < rows.len()
                && matches(&OTHER_BRANCH_LINE, rows[next as usize])
            {
                branch_rows.insert(next as usize);
                next += step;
            }
        }
    }
    for (row, &text) in rows.iter().enumerate() {
        let listed: Option<Range<usize>> = if branch_rows.contains(&row) {
            BRANCH_LINE
                .captures(text)
                .ok()
                .flatten()
                .or_else(|| OTHER_BRANCH_LINE.captures(text).ok().flatten())
                .and_then(|c| c.get(1).map(|m| m.range()))
        } else {
            None
        };
        let listed_name = listed.clone().map(|r| &text[r]);
        let mut command_ranges: Vec<Range<usize>> = Vec::new();
        let add = |result: &mut Vec<Found>,
                   command_ranges: &mut Vec<Range<usize>>,
                   kind: Kind,
                   range: Range<usize>,
                   value: Option<String>,
                   open: Option<String>| {
            if range.is_empty() {
                return;
            }
            if kind == Kind::Command {
                command_ranges.push(range.clone());
            }
            let value = value.unwrap_or_else(|| text[range.clone()].to_owned());
            result.push(Found {
                kind,
                line: row,
                range,
                value,
                open,
            });
        };
        // A displayed command must be a single shell line. Never turn
        // terminal control characters into executable input.
        let runnable = |range: &Range<usize>| !text[range.clone()].chars().any(is_control);
        if let Some(range) = &commands[row]
            && range.end <= text.len()
            && runnable(range)
        {
            add(
                &mut result,
                &mut command_ranges,
                Kind::Command,
                range.clone(),
                None,
                None,
            );
        }
        // Code and colour boundaries keep surrounding prose out of inline
        // commands; in plain prose a cue word introduces a command. On
        // styled rows, an unrecognised coloured path or noun stays put.
        // An update notice names none.
        let notice = matches(&UPDATE_NOTICE, text);
        let mut candidates: Vec<(Range<usize>, Context)> = CODE
            .captures_iter(text)
            .flatten()
            .filter_map(|c| c.get(1).map(|m| (m.range(), Context::Backtick)))
            .collect();
        candidates.extend(colors[row].iter().map(|r| (r.clone(), Context::ColorRun)));
        for (candidate, context) in candidates {
            if notice || candidate.end > text.len() {
                continue;
            }
            let Some(range) = inline_command(&candidate, context, text) else {
                continue;
            };
            if command_ranges.iter().any(|c| overlaps(c, &range)) || !runnable(&range) {
                continue;
            }
            add(
                &mut result,
                &mut command_ranges,
                Kind::Command,
                range,
                None,
                None,
            );
        }
        // A tool call or shown command (cut short with `…`, so not
        // offered) is shell text, not prose: "gh run watch" has no cue.
        let shell = matches(&TOOL_CALL, text) || matches(&SHOWN_CALL, text);
        if !shell && !notice {
            for (range, _) in prose_commands(text) {
                if command_ranges.iter().any(|c| overlaps(c, &range)) || !runnable(&range) {
                    continue;
                }
                add(
                    &mut result,
                    &mut command_ranges,
                    Kind::Command,
                    range,
                    None,
                    None,
                );
            }
        }
        // Non-command items compete for their text: overlapping
        // candidates resolve to the longest (ip:port over ip, a URL over
        // its host). Commands keep their own hint above these.
        let mut found: Vec<Candidate> = Vec::new();
        for m in URLS.find_iter(text).flatten() {
            let range = trimmed_url(m.range(), text);
            let value = &text[range.clone()];
            let Some((scheme, host)) = parse_url(value) else {
                continue;
            };
            if (scheme == "http" || scheme == "https") && host.is_none_or(str::is_empty) {
                continue;
            }
            let can_open = scheme != "file"
                || (files.local() && host.is_none_or(|h| h.is_empty() || h == "localhost"));
            found.push((Kind::Url, range, None, can_open.then(|| encode_url(value))));
        }
        for m in PATHS.find_iter(text).flatten() {
            let mut range = m.range();
            let quoted = text[range.clone()].starts_with(['"', '\'']);
            if quoted {
                range = range.start + 1..range.end - 1;
            } else {
                range = path_range(range, text, files);
            }
            let value = &text[range.clone()];
            // `//host…` is what a URL with an unknown scheme leaves.
            if value.is_empty() || value.contains("://") || value.starts_with("//") {
                continue;
            }
            let decoded = if quoted {
                value.to_owned()
            } else {
                unescape_path(value)
            };
            if branch == Some(decoded.as_str()) || listed_name == Some(decoded.as_str()) {
                continue;
            }
            let path = match LOCATION_SUFFIX.find(&decoded) {
                Ok(Some(suffix)) => &decoded[..suffix.start()],
                _ => decoded.as_str(),
            };
            let explicit = path.starts_with('/')
                || path.starts_with("./")
                || path.starts_with("../")
                || path.starts_with("~/");
            let relative = path.contains('/') && !path.contains(':');
            // Only ask the filesystem about words that could name a file.
            let filename = matches_graphemes(&FILENAME, path);
            let in_command = command_ranges.iter().any(|c| overlaps(c, &range));
            if !(explicit || relative || filename || in_command) {
                continue;
            }
            let url = files.open_target(path);
            // An existing bare word ("docs") only counts as a path inside
            // a command or when it looks like a filename; prose is full
            // of words that happen to name a directory.
            if !(explicit || relative || (url.is_some() && (filename || in_command))) {
                continue;
            }
            found.push((Kind::Path, range, Some(decoded.clone()), url));
        }
        for c in IPV4.captures_iter(text).flatten() {
            if !port_ok(text, c.get(1)) {
                continue;
            }
            found.push((Kind::Ip, c.get(0).unwrap().range(), None, None));
        }
        for c in IPV6.captures_iter(text).flatten() {
            let bracketed = c.get(1).is_some();
            if bracketed != c.get(3).is_some() {
                continue;
            }
            let mut address = c.get(2).unwrap().range();
            // Sentence punctuation after an unbracketed address.
            while !bracketed
                && text[address.clone()].ends_with(':')
                && !text[address.clone()].ends_with("::")
            {
                address.end -= 1;
            }
            let value = &text[address.clone()];
            if !value.chars().any(|c| c.is_numeric())
                || value.matches(':').count() < 2
                || Ipv6Addr::from_str(value).is_err()
            {
                continue;
            }
            if !port_ok(text, c.get(4)) {
                continue;
            }
            found.push((
                Kind::Ip,
                if bracketed {
                    c.get(0).unwrap().range()
                } else {
                    address
                },
                None,
                None,
            ));
        }
        for c in DOMAIN.captures_iter(text).flatten() {
            if !TOP_LEVEL_DOMAINS.contains(text[c.get(1).unwrap().range()].to_lowercase().as_str())
            {
                continue;
            }
            if !port_ok(text, c.get(2)) {
                continue;
            }
            found.push((Kind::Domain, c.get(0).unwrap().range(), None, None));
        }
        for m in UUID.find_iter(text).flatten() {
            found.push((Kind::Hash, m.range(), None, None));
        }
        for m in HASH.find_iter(text).flatten() {
            // Plain words and plain numbers aren't hashes.
            let value = m.as_str();
            if !value.bytes().any(|b| b.is_ascii_digit())
                || !value.bytes().any(|b| b.is_ascii_alphabetic())
            {
                continue;
            }
            found.push((Kind::Hash, m.range(), None, None));
        }
        if let Some(range) = listed {
            found.push((Kind::Branch, range, None, None));
        }
        if let Some(branch) = branch.filter(|b| !b.is_empty()) {
            let boundary = |c: Option<char>| {
                c.is_none_or(|c| c.is_whitespace() || "()[]{}<>\"'`,;:".contains(c))
            };
            let mut search = 0;
            while let Some(at) = text[search..].find(branch) {
                let start = search + at;
                let end = start + branch.len();
                // Sentence punctuation ends a branch name, but `main.rs` isn't `main`.
                let after = char_at(text, end);
                let ends = boundary(after)
                    || (after.is_some_and(|c| ".!?".contains(c))
                        && boundary(char_at(text, end + 1)));
                if boundary(char_before(text, start)) && ends {
                    found.push((Kind::Branch, start..end, None, None));
                }
                search = end;
            }
        }
        found.sort_by(|a, b| {
            b.1.len()
                .cmp(&a.1.len())
                .then_with(|| a.0.priority().cmp(&b.0.priority()))
                .then_with(|| a.1.start.cmp(&b.1.start))
        });
        let mut taken: Vec<Range<usize>> = Vec::new();
        for (kind, range, value, open) in found {
            if taken.iter().any(|t| overlaps(t, &range)) {
                continue;
            }
            taken.push(range.clone());
            add(&mut result, &mut command_ranges, kind, range, value, open);
        }
    }
    result
}

/// A command inside backticks or a colour run. A lone name (`make`)
/// names a tool unless a cue word asks to run it ("run `make`").
fn inline_command(candidate: &Range<usize>, context: Context, text: &str) -> Option<Range<usize>> {
    let raw = &text[candidate.clone()];
    let value = trim_hspace(raw);
    if value.is_empty() {
        return None;
    }
    let start = candidate.start + raw.find(value)?;
    let range = start..start + value.len();
    let words = tokens(text, range.clone());
    if !accepts(&words, context) {
        return None;
    }
    if words.len() == 1 && !script(words[0].text) {
        // The words before the candidate, short of the character before it.
        let before_end =
            char_before(text, candidate.start).map_or(0, |c| candidate.start - c.len_utf8());
        let before = tokens(text, 0..before_end)
            .last()
            .map(|t| t.text.to_lowercase())?;
        if !CUES.contains(&before.as_str()) {
            return None;
        }
    }
    Some(range)
}

fn trim_closing(value: &str, last: char) -> bool {
    for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
        if last == close {
            return value.matches(close).count() > value.matches(open).count();
        }
    }
    false
}

fn trimmed_url(mut range: Range<usize>, text: &str) -> Range<usize> {
    while !range.is_empty() {
        let value = &text[range.clone()];
        let last = value.chars().next_back().unwrap();
        let trim = ".,;:!?".contains(last) || trim_closing(value, last);
        if !trim {
            break;
        }
        range.end -= last.len_utf8();
    }
    range
}

/// Output punctuation is not part of a path. Keep quoted/escaped names
/// and balanced filename punctuation intact, including literal trailing
/// punctuation when that exact local file exists.
fn path_range(mut range: Range<usize>, text: &str, files: &mut Files) -> Range<usize> {
    let value = &text[range.clone()];
    match PATH_START.find(value) {
        Ok(Some(start)) if start.end() > 0 => range.start += start.end(),
        _ => {
            while text[range.clone()].starts_with(['(', '[', '{']) {
                range.start += 1;
            }
        }
    }
    // A mount spec, `/host/dir:/container/dir:ro`, names its host path first.
    let value = &text[range.clone()];
    if let Some(colon) = value.find(":/")
        && !value.contains("://")
        && value.chars().count() > 1
        && colon > value.chars().next().unwrap().len_utf8()
    {
        range.end = range.start + colon;
    }
    loop {
        let value = &text[range.clone()];
        let Some(last) = value.chars().next_back() else {
            break;
        };
        if !".,:!?)]}".contains(last) {
            break;
        }
        let escaped = value[..value.len() - 1]
            .bytes()
            .rev()
            .take_while(|&b| b == b'\\')
            .count()
            % 2
            == 1;
        if escaped {
            break;
        }
        if files.open_target(&unescape_path(value)).is_some() {
            break;
        }
        let trim = ".,:!?".contains(last) || trim_closing(value, last);
        if !trim {
            break;
        }
        range.end -= 1;
    }
    range
}

fn unescape_path(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut escaped = false;
    for c in text.chars() {
        if escaped {
            result.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            result.push(c);
        }
    }
    if escaped {
        result.push('\\');
    }
    result
}

/// A URL as an opener takes it: characters a URL can't hold (non-ASCII,
/// spaces, braces, stray `%`) percent-encoded, as a URL parser would.
fn encode_url(value: &str) -> String {
    let b = value.as_bytes();
    let mut url = String::with_capacity(value.len());
    for (i, &c) in b.iter().enumerate() {
        let escape = c == b'%'
            && b.get(i + 1..i + 3)
                .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit));
        if c >= 0x80 || b" \"<>\\^`{|}".contains(&c) || (c == b'%' && !escape) {
            url.push_str(&format!("%{c:02X}"));
        } else {
            url.push(c as char);
        }
    }
    url
}

/// The scheme and host of a URL the regex found, or `None` when the
/// platform's URL parser would refuse it (a malformed host).
fn parse_url(value: &str) -> Option<(&str, Option<&str>)> {
    let colon = value.find(':')?;
    let scheme = &value[..colon];
    let rest = &value[colon + 1..];
    let Some(after) = rest.strip_prefix("//") else {
        return Some((scheme, None));
    };
    let authority = &after[..after.find(['/', '?', '#']).unwrap_or(after.len())];
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host_port.starts_with('[') {
        &host_port[..host_port.find(']').map_or(host_port.len(), |i| i + 1)]
    } else {
        host_port.rsplit_once(':').map_or(host_port, |(h, _)| h)
    };
    let bad_escape = host.match_indices('%').any(|(i, _)| {
        host.get(i + 1..i + 3)
            .is_none_or(|h| !h.bytes().all(|b| b.is_ascii_hexdigit()))
    });
    if host.contains(['\\', '{', '}', '|', '^']) || bad_escape {
        return None;
    }
    if scheme == "file" && authority.is_empty() {
        return Some((scheme, None));
    }
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    Some((scheme, Some(host)))
}
