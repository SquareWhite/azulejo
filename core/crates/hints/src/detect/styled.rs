//! The terminal's styled snapshot (SGR colours) as colour runs per row.

use std::sync::LazyLock;

use fancy_regex::Regex;

/// A run of one foreground colour on a row, in bytes of that row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorSpan {
    pub row: usize,
    pub start: usize,
    pub end: usize,
    /// Palette index, or RGB with bit 24 set. Default foreground is omitted.
    pub color: u32,
}

static VT_CONTROLS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\r?\n").unwrap()
});

/// Decodes the terminal formatter's SGR snapshot (never raw PTY output)
/// into plain rows and colour runs. Bold or underline changes don't split
/// a run; background and underline colours are skipped.
pub fn styled_rows(vt: &str) -> (Vec<String>, Vec<ColorSpan>) {
    let mut rows = vec![String::new()];
    let mut colors: Vec<ColorSpan> = Vec::new();
    let mut foreground: Option<u32> = None;
    let mut offset = 0;
    let append =
        |rows: &mut Vec<String>, colors: &mut Vec<ColorSpan>, text: &str, fg: Option<u32>| {
            if text.is_empty() {
                return;
            }
            let row = rows.len() - 1;
            let start = rows[row].len();
            rows[row].push_str(text);
            let end = rows[row].len();
            if let Some(color) = fg {
                if let Some(last) = colors.last_mut()
                    && last.row == row
                    && last.color == color
                    && last.end == start
                {
                    last.end = end;
                } else {
                    colors.push(ColorSpan {
                        row,
                        start,
                        end,
                        color,
                    });
                }
            }
        };
    for m in VT_CONTROLS.find_iter(vt).flatten() {
        append(&mut rows, &mut colors, &vt[offset..m.start()], foreground);
        let control = m.as_str();
        if control == "\r\n" || control == "\n" {
            rows.push(String::new());
        } else if control.starts_with("\x1b[") && control.ends_with('m') {
            let parameters: Vec<&str> = control[2..control.len() - 1].split(';').collect();
            let mut i = 0;
            while i < parameters.len() {
                let n = if parameters[i].is_empty() {
                    0
                } else if let Ok(n) = parameters[i].parse::<i64>() {
                    n
                } else {
                    i += 1;
                    continue;
                };
                match n {
                    0 | 39 => foreground = None,
                    30..=37 => foreground = Some((n - 30) as u32),
                    90..=97 => foreground = Some((n - 90 + 8) as u32),
                    38 | 48 | 58 => {
                        // Consume background/underline payloads too: their RGB
                        // components must never be mistaken for SGR operations.
                        if i + 2 < parameters.len() && parameters[i + 1] == "5" {
                            if n == 38 {
                                foreground = parameters[i + 2].parse::<u32>().ok();
                            }
                            i += 2;
                        } else if i + 4 < parameters.len() && parameters[i + 1] == "2" {
                            if n == 38
                                && let (Ok(r), Ok(g), Ok(b)) = (
                                    parameters[i + 2].parse::<u32>(),
                                    parameters[i + 3].parse::<u32>(),
                                    parameters[i + 4].parse::<u32>(),
                                )
                            {
                                foreground = Some(0x100_0000 | r << 16 | g << 8 | b);
                            }
                            i += 4;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        offset = m.end();
    }
    append(&mut rows, &mut colors, &vt[offset..], foreground);
    (rows, colors)
}

/// Colour runs from a snapshot, kept only on rows whose plain capture
/// matches it (the plain row is the styled one plus trailing spaces), so
/// output arriving between the two reads can't move a colour boundary onto
/// different text.
pub fn matching_colors(rows: &[String], vt: &str) -> Vec<ColorSpan> {
    let (styled, colors) = styled_rows(vt);
    let matching: Vec<bool> = rows
        .iter()
        .enumerate()
        .map(|(row, plain)| {
            styled.get(row).is_some_and(|styled| {
                plain.starts_with(styled.as_str())
                    && plain[styled.len()..].bytes().all(|b| b == b' ')
            })
        })
        .collect();
    colors
        .into_iter()
        .filter(|c| matching.get(c.row).copied().unwrap_or(false) && c.end <= rows[c.row].len())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_runs() {
        let (rows, colors) =
            styled_rows("a \x1b[34mgit\x1b[1m status\x1b[0m b\r\n\x1b[38;2;1;2;3mx\x1b[39m");
        assert_eq!(rows, vec!["a git status b", "x"]);
        assert_eq!(
            colors,
            vec![
                ColorSpan {
                    row: 0,
                    start: 2,
                    end: 12,
                    color: 4
                },
                ColorSpan {
                    row: 1,
                    start: 0,
                    end: 1,
                    color: 0x100_0000 | 1 << 16 | 2 << 8 | 3
                },
            ]
        );
        let kept = matching_colors(
            &["a git status b  ".into(), "y".into()],
            "a \x1b[34mgit\x1b[0m\r\n\x1b[31my",
        );
        assert_eq!(kept.len(), 1);
    }
}
