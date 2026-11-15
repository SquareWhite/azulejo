//! What a shell would make of a command so far: a small lexer for quotes,
//! `$(…)` and `(…)` nesting, comments, heredocs and continuations.

/// Whether a command is still open (a quote, heredoc or bracket) or asks
/// for another line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShellState {
    pub quote: bool,
    pub heredoc: bool,
    pub depth: usize,
    pub continues: bool,
    /// The command asks for another line with a trailing backslash.
    pub backslash: bool,
    /// Offset (in the lexed bytes) of a tool call's closing parenthesis.
    pub close: Option<usize>,
}

impl ShellState {
    pub fn open(&self) -> bool {
        self.quote || self.heredoc || self.depth > 0
    }
}

fn blank(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

fn word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The byte length of the UTF-8 character starting with `lead`.
fn char_len(lead: u8) -> usize {
    match lead {
        0xF0.. => 4,
        0xE0.. => 3,
        0xC0.. => 2,
        _ => 1,
    }
}

/// Lexes a command's bytes (line breaks as `\n`). A tool call's wrapper
/// (`wrapper`) counts as one more bracket; its closing parenthesis ends
/// the lexing.
pub fn lex(u: &[u8], wrapper: bool) -> ShellState {
    let mut state = ShellState::default();
    // The quote in force at each nesting level; 0 when unquoted.
    let mut levels: Vec<u8> = if wrapper { vec![0, 0] } else { vec![0] };
    let mut pending: Vec<&[u8]> = Vec::new();
    let mut body: Vec<&[u8]> = Vec::new();
    // Open `do`/`then`/`case`/`{` blocks, counted at command position.
    let mut blocks = 0usize;
    let mut command_position = true;
    let n = u.len();
    let mut i = 0;
    'scan: while i < n {
        if let Some(&delimiter) = body.first() {
            // A heredoc body runs line by line up to its delimiter. A
            // tool call may close its wrapper right after it: `EOF)`.
            let mut j = i;
            while j < n && u[j] != b'\n' {
                j += 1;
            }
            let mut k = i;
            while k < j && blank(u[k]) {
                k += 1;
            }
            let d = delimiter;
            if j - k >= d.len()
                && &u[k..k + d.len()] == d
                && u[k + d.len()..j]
                    .iter()
                    .all(|&c| blank(c) || c == b')' || c == b'"')
            {
                body.remove(0);
                i = k + d.len();
                continue;
            }
            if j >= n {
                break;
            }
            i = j + 1;
            continue;
        }
        let c = u[i];
        let quote = *levels.last().unwrap();
        if quote == b'\'' || quote == b'`' {
            if c == quote {
                *levels.last_mut().unwrap() = 0;
            }
            i += 1;
            continue;
        }
        if c == b'\\' {
            i += 1 + if i + 1 < n { char_len(u[i + 1]) } else { 1 };
            continue;
        }
        if quote == b'"' {
            if c == b'"' {
                *levels.last_mut().unwrap() = 0;
            } else if c == b'$' && i + 1 < n && u[i + 1] == b'(' {
                levels.push(0);
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if command_position && !blank(c) {
            let mut j = i;
            while j < n && (word(u[j]) || u[j] == b'{' || u[j] == b'}') {
                j += 1;
            }
            let bounded = j > i && (j == n || blank(u[j]) || u[j] == b'\n' || u[j] == b';');
            let name: &[u8] = if bounded { &u[i..j] } else { b"" };
            match name {
                b"do" | b"then" | b"case" | b"{" => blocks += 1,
                b"done" | b"fi" | b"esac" | b"}" => blocks = blocks.saturating_sub(1),
                _ => {}
            }
            // After these a command follows; skip the keyword itself.
            if matches!(
                name,
                b"do" | b"then" | b"else" | b"elif" | b"if" | b"while" | b"until" | b"{"
            ) {
                i = j;
                continue;
            }
            command_position = false;
            if matches!(name, b"case" | b"done" | b"fi" | b"esac" | b"}") {
                i = j;
                continue;
            }
        }
        match c {
            b';' | b'&' | b'|' => command_position = true,
            b'\'' | b'"' | b'`' => *levels.last_mut().unwrap() = c,
            b'#' if i == 0 || blank(u[i - 1]) || u[i - 1] == b'\n' => {
                while i < n && u[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'$' if i + 1 < n && u[i + 1] == b'(' => {
                levels.push(0);
                i += 2;
                continue;
            }
            b'(' => levels.push(0),
            b')' => {
                if levels.len() > 1 {
                    levels.pop();
                    if wrapper && levels.len() == 1 {
                        state.close = Some(i);
                        break 'scan;
                    }
                }
            }
            b'<' if i + 1 < n && u[i + 1] == b'<' && !(i + 2 < n && u[i + 2] == b'<') => {
                let mut j = i + 2;
                if j < n && u[j] == b'-' {
                    j += 1;
                }
                while j < n && blank(u[j]) {
                    j += 1;
                }
                let mut quote_mark = None;
                if j < n && (u[j] == b'\'' || u[j] == b'"') {
                    quote_mark = Some(u[j]);
                    j += 1;
                }
                let s = j;
                while j < n && word(u[j]) {
                    j += 1;
                }
                if j > s {
                    pending.push(&u[s..j]);
                    if let Some(mark) = quote_mark
                        && j < n
                        && u[j] == mark
                    {
                        j += 1;
                    }
                    i = j;
                    continue;
                }
            }
            b'\n' => {
                command_position = true;
                if !pending.is_empty() {
                    body = std::mem::take(&mut pending);
                }
            }
            _ => {}
        }
        i += 1;
    }
    state.quote = levels.iter().any(|&q| q != 0);
    state.heredoc = !pending.is_empty() || !body.is_empty();
    state.depth = levels.len() - 1 + blocks;
    if state.close.is_none() && !state.quote && body.is_empty() {
        let mut end = n.min(u.len());
        while end > 0 && blank(u[end - 1]) {
            end -= 1;
        }
        if end > 0 {
            let last = u[end - 1];
            if last == b'|' || (last == b'&' && end > 1 && u[end - 2] == b'&') {
                state.continues = true;
            } else if last == b'\\' {
                let count = u[..end].iter().rev().take_while(|&&c| c == b'\\').count();
                state.continues = count % 2 == 1;
                state.backslash = state.continues;
            }
        }
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states() {
        assert!(lex(b"echo 'open", false).quote);
        assert!(lex(b"cat <<EOF\nbody", false).heredoc);
        assert!(!lex(b"cat <<EOF\nbody\nEOF", false).open());
        assert!(lex(b"ls |", false).continues);
        let s = lex(b"docker run \\", false);
        assert!(s.continues && s.backslash);
        assert_eq!(lex(b"git log)", true).close, Some(7));
        assert_eq!(lex(b"for f in *; do", false).depth, 1);
    }
}
