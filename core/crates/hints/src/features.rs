//! Hashed row features. Training and inference share this code, so the
//! weights file is only valid for the features it was trained with:
//! `just lint` fails when this file changed after the last `just hints-model`.
//!
//! Each row gets its own features (shape, tokens, prompt, punctuation), the
//! "shared" subset of its neighbours' features (prefixed, so "the row above
//! ends with `\`" differs from "this row ends with `\`"), and a few
//! relational and screen-wide ones (same indent as above, soft wrap from
//! above, inside an open `Bash(` call, a fence or a heredoc).

use crate::text;

const SEED: u32 = 0x811c_9dc5;

fn fnv(bytes: &[u8], mut h: u32) -> u32 {
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn mix(h: u32, salt: u32) -> u32 {
    let mut x = (h ^ salt).wrapping_mul(0x85eb_ca6b);
    x ^= x >> 13;
    x = x.wrapping_mul(0xc2b2_ae35);
    x ^ (x >> 16)
}

const PREV: u32 = 0x5bd1_e995;
const NEXT: u32 = 0x1b87_3593;

/// Feature sink for one row.
#[derive(Default)]
struct Sink {
    own: Vec<u32>,
    shared: Vec<u32>,
}

impl Sink {
    fn tag(&mut self, tag: &str) {
        self.own.push(fnv(tag.as_bytes(), SEED));
    }
    fn val(&mut self, tag: &str, value: &str) {
        self.own.push(fnv(
            value.as_bytes(),
            fnv(&[0xff], fnv(tag.as_bytes(), SEED)),
        ));
    }
    fn num(&mut self, tag: &str, n: usize) {
        self.own.push(fnv(
            &(n as u32).to_le_bytes(),
            fnv(&[0xfe], fnv(tag.as_bytes(), SEED)),
        ));
    }
    /// Also exported to the rows above and below.
    fn tag_s(&mut self, tag: &str) {
        let h = fnv(tag.as_bytes(), SEED);
        self.own.push(h);
        self.shared.push(h);
    }
    fn val_s(&mut self, tag: &str, value: &str) {
        let h = fnv(value.as_bytes(), fnv(&[0xff], fnv(tag.as_bytes(), SEED)));
        self.own.push(h);
        self.shared.push(h);
    }
    fn num_s(&mut self, tag: &str, n: usize) {
        let h = fnv(
            &(n as u32).to_le_bytes(),
            fnv(&[0xfe], fnv(tag.as_bytes(), SEED)),
        );
        self.own.push(h);
        self.shared.push(h);
    }
}

/// What the relational features need from a row.
#[derive(Default, Clone)]
struct Summary {
    blank: bool,
    indent: usize,
    cells: usize,
    prompt: u8,
    continues: bool,
    open_quote: bool,
    bash_open: i32,
    fence: bool,
    heredoc: Option<String>,
    agent: bool,
}

fn bucket(n: usize, edges: &[usize]) -> usize {
    edges.iter().position(|&e| n <= e).unwrap_or(edges.len())
}

fn char_class(c: char) -> &'static str {
    if c.is_ascii_uppercase() {
        "A"
    } else if c.is_ascii_lowercase() {
        "a"
    } else if c.is_ascii_digit() {
        "0"
    } else if c.is_alphabetic() {
        "u"
    } else if text::is_box_char(c) {
        "box"
    } else {
        ""
    }
}

/// Word shape: runs of character classes, at most 6 runs ("Aa", "a-a", "0.0").
fn shape(word: &str) -> String {
    let mut out = String::new();
    let mut last = '\0';
    let mut runs = 0;
    for c in word.chars() {
        let k = if c.is_ascii_uppercase() {
            'A'
        } else if c.is_ascii_lowercase() {
            'a'
        } else if c.is_ascii_digit() {
            '0'
        } else if c.is_alphabetic() {
            'u'
        } else {
            c
        };
        if k != last {
            runs += 1;
            if runs > 6 {
                out.push('…');
                break;
            }
            out.push(k);
            last = k;
        }
    }
    out
}

/// Normalised token: lowercase, digits folded, at most 16 chars.
fn norm(word: &str) -> String {
    word.chars()
        .take(16)
        .map(|c| {
            if c.is_ascii_digit() {
                '0'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect()
}

const STOP: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "do", "for", "from", "has",
    "have", "i", "if", "in", "is", "it", "its", "i'll", "let", "me", "my", "not", "now", "of",
    "on", "or", "our", "should", "so", "that", "the", "then", "there", "this", "to", "was", "we",
    "when", "which", "will", "with", "you", "your",
];

fn local(row: &str, columns: Option<usize>, sink: &mut Sink) -> Summary {
    let row = row.trim_end();
    let mut sum = Summary {
        blank: row.trim().is_empty(),
        ..Default::default()
    };
    if sum.blank {
        sink.tag_s("blank");
        return sum;
    }
    sum.indent = row.chars().take_while(|&c| c == ' ' || c == '\t').count();
    sum.cells = text::cells(row);
    sink.num_s("ind", bucket(sum.indent, &[0, 1, 2, 4, 6, 8, 12]));
    sink.num_s("len", bucket(sum.cells, &[3, 8, 16, 30, 50, 70, 90, 120]));
    if let Some(cols) = columns.filter(|&c| c > 0) {
        let fill = sum.cells * 10 / cols;
        sink.num_s("fill", fill.min(10));
        if sum.cells + 3 >= cols {
            sink.tag_s("edge");
        }
    }

    let (s, e) = text::unboxed(row);
    let trimmed = row.trim_start();
    let first = trimmed.chars().next().unwrap_or(' ');
    let boxed = s > row.len() - trimmed.len();
    if boxed {
        sink.tag_s("boxed");
    }
    let inner = &row[s..e.max(s)];
    let box_cells = row.chars().filter(|&c| text::is_box_char(c)).count();
    if box_cells * 2 >= row.chars().filter(|c| !c.is_whitespace()).count() {
        sink.tag_s("boxline");
    } else if box_cells > 0 {
        sink.tag("hasbox");
    }

    // Leading character and a coding agent's bullet.
    let lead_class = char_class(first);
    if lead_class.is_empty() {
        sink.val_s("lead", first.encode_utf8(&mut [0; 4]));
    } else {
        sink.val_s("lead", lead_class);
    }
    let mut body = inner;
    if let Some(c) = inner.chars().next()
        && (text::is_agent_bullet(c) || c == '⎿')
    {
        sum.agent = true;
        sink.val_s("bullet", if c == '⎿' { "elbow" } else { "dot" });
        body = inner[c.len_utf8()..].trim_start();
    }
    if let Some(tool) = text::tool_call(row) {
        sink.val_s("tool", tool);
        if tool == "Bash" {
            sum.bash_open = paren_balance(&row[text::bash_call(row).unwrap_or(0)..]) + 1;
        }
    }
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        sum.fence = true;
        sink.tag_s("fence");
    }

    // The prompt.
    if let Some(p) = text::prompt(row) {
        sum.prompt = p.strength.max(1);
        let marker = p.marker.to_string();
        sink.val_s("prompt", &format!("{}{}", p.strength, marker));
        sink.num_s(
            "promptat",
            bucket(text::cells(&row[..p.at]), &[0, 2, 8, 16, 30, 50]),
        );
        if p.strength == 0 {
            sink.tag_s("bareprompt");
        } else {
            let after = &row[p.start..];
            sink.val("pw0", &norm(text::first_word(after)));
            if after.contains("<<") {
                sum.heredoc = heredoc_word(after);
            }
        }
    }
    let ps2 = text::ps2_len(row);
    if ps2 > sum.indent {
        sink.tag_s("ps2");
        sink.val("ps2w", row[..ps2].trim());
    }

    // Tokens.
    let words: Vec<&str> = body.split_whitespace().collect();
    sink.num_s("nw", bucket(words.len(), &[1, 2, 3, 5, 8, 12, 20]));
    if let Some(w0) = words.first() {
        sink.val("w0", &norm(w0));
        sink.val_s("w0s", &shape(w0));
        if text::is_command_word(w0) {
            sink.tag_s("w0cmd");
        } else if text::word_starts_command(w0) {
            sink.tag_s("w0prog");
        }
        if w0.ends_with(':') {
            sink.tag_s("w0colon");
        }
        if w0.len() > 1
            && w0.chars().next().is_some_and(|c| c.is_ascii_digit())
            && (w0.ends_with('.') || w0.ends_with(')'))
        {
            sink.tag_s("listnum");
        }
        if matches!(*w0, "-" | "*" | "•" | "+") {
            sink.tag_s("listbullet");
        }
    }
    if let Some(w1) = words.get(1) {
        sink.val("w1", &norm(w1));
        sink.val("w1s", &shape(w1));
        if w1.starts_with('-') && w1.len() > 1 {
            sink.tag("w1flag");
        }
        if text::is_command_word(words[0]) {
            sink.val("w01", &format!("{} {}", norm(words[0]), norm(w1)));
        }
    }
    if let Some(wl) = words.last() {
        sink.val("wls", &shape(wl));
    }
    let last = body.chars().next_back().unwrap_or(' ');
    let last_class = char_class(last);
    let mut buf = [0; 4];
    sink.val_s(
        "last",
        if last_class.is_empty() {
            last.encode_utf8(&mut buf)
        } else {
            last_class
        },
    );

    let mut alpha_words = 0;
    let mut stops = 0;
    let mut flags = 0;
    let mut cmd_words = 0;
    let mut capital_words = 0;
    for (i, w) in words.iter().enumerate() {
        let bare = w.trim_matches(|c: char| ",.;:!?()\"'`".contains(c));
        if !bare.is_empty()
            && bare
                .chars()
                .all(|c| c.is_alphabetic() || c == '\'' || c == '-')
        {
            alpha_words += 1;
            let lower = bare.to_lowercase();
            if STOP.contains(&lower.as_str()) {
                stops += 1;
            }
            if i < 24 {
                sink.val("t", &lower);
            }
            if bare.chars().next().is_some_and(char::is_uppercase) {
                capital_words += 1;
            }
        }
        if w.starts_with('-') && w.len() > 1 && !w.starts_with("--- ") {
            flags += 1;
        }
        if text::is_command_word(w) {
            cmd_words += 1;
        }
    }
    let nw = words.len().max(1);
    sink.num_s("alpha", alpha_words * 4 / nw);
    sink.num_s("stops", bucket(stops, &[0, 1, 2, 4]));
    sink.num("stopr", stops * 4 / nw);
    sink.num_s("flags", bucket(flags, &[0, 1, 2, 4]));
    sink.num("cmdw", bucket(cmd_words, &[0, 1, 2]));
    sink.num("caps", bucket(capital_words, &[0, 1, 3]));
    if first.is_uppercase() && last == '.' && alpha_words >= 4 {
        sink.tag_s("sentence");
    } else if (last == '.' || last == ':') && stops >= 2 {
        sink.tag_s("sentenceish");
    }

    // Characters.
    let mut digits = 0;
    let mut upper = 0;
    let mut punct = 0;
    let mut total = 0;
    for c in body.chars() {
        if c.is_whitespace() {
            continue;
        }
        total += 1;
        if c.is_ascii_digit() {
            digits += 1;
        } else if c.is_uppercase() {
            upper += 1;
        } else if !c.is_alphanumeric() {
            punct += 1;
        }
    }
    let total = total.max(1);
    sink.num_s("digits", digits * 5 / total);
    sink.num("upper", upper * 5 / total);
    sink.num_s("punct", punct * 5 / total);

    for (needle, tag) in [
        (" | ", "pipe"),
        ("&&", "and"),
        ("||", "or"),
        ("; ", "semi"),
        (" > ", "redir"),
        ("2>&1", "redir2"),
        ("$(", "subst"),
        ("`", "tick"),
        ("://", "url"),
        ("~/", "home"),
        ("./", "rel"),
        ("...", "dots"),
        ("…", "ellipsis"),
        ("=", "eq"),
        ("@", "at"),
        ("::", "colons"),
        ("->", "arrow"),
        ("=>", "farrow"),
        ("[", "bracket"),
        ("(", "paren"),
        ("{", "brace"),
        ("\"", "dquote"),
        ("'", "squote"),
        ("#", "hash"),
        ("%", "percent"),
        ("✓", "check"),
        ("✔", "check"),
        ("✗", "cross"),
        ("ctrl+", "ctrl"),
        ("<<", "heredoc"),
    ] {
        if body.contains(needle) {
            sink.tag(tag);
        }
    }
    if body.contains('/') {
        sink.tag("slash");
    }
    if has_time(body) {
        sink.tag_s("time");
    }
    if has_location(body) {
        sink.tag("fileline");
    }
    if body.starts_with(['d', '-', 'l'])
        && body
            .chars()
            .skip(1)
            .take(9)
            .filter(|c| "rwxst-@+".contains(*c))
            .count()
            == 9
    {
        sink.tag_s("perms");
    }
    let gaps = body.split("  ").filter(|p| !p.trim().is_empty()).count();
    sink.num_s("cols", bucket(gaps.saturating_sub(1), &[0, 1, 2, 4]));
    if text::continues(body) {
        sum.continues = true;
        sink.tag_s("continues");
    }
    if text::open_quote(body) {
        sum.open_quote = true;
        sink.tag_s("openquote");
    }
    if body.starts_with(['+', '-']) && body.len() > 1 && !body[1..].starts_with([' ', '-', '+']) {
        sink.tag_s("diffline");
    }
    if body.starts_with("@@") || body.starts_with("diff --git") {
        sink.tag_s("diffhead");
    }
    if body.starts_with("=>") || body.starts_with("==>") {
        sink.tag_s("arrowstep");
    }
    if trimmed.starts_with(['✻', '✳', '✶', '✢', '✽', '·', '⏵', '⏸', '?']) {
        sink.val_s("status", first.encode_utf8(&mut [0; 4]));
    }
    sum
}

fn paren_balance(text: &str) -> i32 {
    let mut depth = 0;
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if q == c => quote = None,
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            _ => {}
        }
    }
    depth
}

fn heredoc_word(text: &str) -> Option<String> {
    let at = text.find("<<")?;
    let rest = text[at + 2..].trim_start_matches(['-', '~']).trim_start();
    let word: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ';' && *c != '>')
        .collect();
    let word = word.trim_matches(['\'', '"']).to_string();
    (!word.is_empty()).then_some(word)
}

fn has_time(text: &str) -> bool {
    let b = text.as_bytes();
    b.windows(5).any(|w| {
        w[0].is_ascii_digit()
            && w[1].is_ascii_digit()
            && w[2] == b':'
            && w[3].is_ascii_digit()
            && w[4].is_ascii_digit()
    })
}

fn has_location(text: &str) -> bool {
    let b = text.as_bytes();
    b.windows(3).enumerate().any(|(i, w)| {
        w[0] == b':'
            && w[1].is_ascii_digit()
            && i > 0
            && (b[i - 1].is_ascii_alphanumeric())
            && text[..i].contains('.')
    })
}

/// Features for every row of a screen. Each row's list holds raw 32-bit
/// hashes; the model folds them into its bucket count.
pub fn screen(rows: &[&str], joins: &[usize], columns: Option<usize>) -> Vec<Vec<u32>> {
    let mut sinks: Vec<Sink> = Vec::with_capacity(rows.len());
    let mut sums: Vec<Summary> = Vec::with_capacity(rows.len());
    for row in rows {
        let mut sink = Sink::default();
        sums.push(local(row, columns, &mut sink));
        sinks.push(sink);
    }
    let agent_screen = sums.iter().any(|s| s.agent);
    let strong_prompts = sums.iter().filter(|s| s.prompt >= 3).count();

    // Stateful context: open `Bash(` calls, fences, heredocs and quotes.
    let mut state: Vec<Vec<&'static str>> = vec![Vec::new(); rows.len()];
    let mut bash_depth = 0i32;
    let mut in_fence = false;
    let mut heredoc: Option<String> = None;
    let mut quote_open = false;
    for i in 0..rows.len() {
        let s = &sums[i];
        if bash_depth > 0 && !s.blank {
            state[i].push("inbash");
            bash_depth += paren_balance(rows[i]);
        }
        if s.bash_open > 0 {
            bash_depth = s.bash_open;
        }
        if s.blank && bash_depth > 0 {
            bash_depth = 0;
        }
        if in_fence && !s.fence {
            state[i].push("infence");
        }
        if s.fence {
            in_fence = !in_fence;
        }
        if let Some(word) = &heredoc {
            state[i].push("inheredoc");
            let body = rows[i][text::ps2_len(rows[i])..].trim();
            if body == word {
                state[i].push("heredocend");
                heredoc = None;
            }
        } else if let Some(word) = &s.heredoc {
            heredoc = Some(word.clone());
        }
        if quote_open {
            state[i].push("inquote");
            if text::open_quote(rows[i]) {
                quote_open = false;
            }
        } else if s.prompt >= 2 && s.open_quote {
            quote_open = true;
        }
    }

    let mut out = Vec::with_capacity(rows.len());
    for i in 0..rows.len() {
        let mut f = std::mem::take(&mut sinks[i].own);
        let s = &sums[i];
        let mut add = |tag: &str| f.push(fnv(tag.as_bytes(), SEED));
        add("bias");
        if agent_screen {
            add("scr:agent");
        }
        match strong_prompts {
            0 => add("scr:noprompt"),
            1 => add("scr:oneprompt"),
            _ => add("scr:prompts"),
        }
        for tag in &state[i] {
            add(tag);
        }
        if i == 0 {
            add("top");
        }
        if i + 1 == rows.len() {
            add("bottom");
        }
        if i > 0 {
            let p = &sums[i - 1];
            if joins.contains(&(i - 1)) {
                add("softp");
            }
            if !p.blank && !s.blank {
                if p.indent == s.indent {
                    add("sameind");
                } else if p.indent < s.indent {
                    add("indgt");
                } else {
                    add("indlt");
                }
                if p.continues || (p.prompt >= 2 && p.open_quote) {
                    add("pcont");
                }
                if let Some(cols) = columns
                    && p.cells + 12 >= cols
                {
                    add("pnearedge");
                }
            }
        }
        if joins.contains(&i) {
            add("softn");
        }
        if i > 1 && sums[i - 2].blank {
            add("p2blank");
        }
        if i > 0 {
            for &h in &sinks[i - 1].shared {
                f.push(mix(h, PREV));
            }
        } else {
            f.push(mix(fnv(b"none", SEED), PREV));
        }
        if i + 1 < rows.len() {
            for &h in &sinks[i + 1].shared {
                f.push(mix(h, NEXT));
            }
        } else {
            f.push(mix(fnv(b"none", SEED), NEXT));
        }
        out.push(f);
    }
    out
}

/// True when the row is nothing but whitespace: always [`crate::RowLabel::Blank`].
pub fn is_blank(row: &str) -> bool {
    row.trim().is_empty()
}
