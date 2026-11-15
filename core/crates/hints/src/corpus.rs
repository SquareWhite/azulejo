//! Labelled screens in the corpus format (`data/heldout.txt` documents it;
//! `tools/generate.py` writes the same format).

use std::collections::BTreeSet;

use crate::RowLabel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Default)]
pub struct Screen {
    pub id: String,
    pub summary: String,
    pub columns: usize,
    pub rows: Vec<String>,
    pub labels: Vec<RowLabel>,
    /// Rows whose last cell soft-wraps into the next row.
    pub joins: BTreeSet<usize>,
    pub expected: Vec<Expected>,
}

impl Screen {
    pub fn row_refs(&self) -> Vec<&str> {
        self.rows.iter().map(String::as_str).collect()
    }
    pub fn join_list(&self) -> Vec<usize> {
        self.joins.iter().copied().collect()
    }
}

pub fn parse(text: &str) -> Result<Vec<Screen>, String> {
    let mut screens: Vec<Screen> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(head) = line.strip_prefix("=== ") {
            let (left, summary) = head.split_once('|').unwrap_or((head, ""));
            let mut words = left.split_whitespace();
            let id = words
                .next()
                .ok_or(format!("line {}: screen without id", n + 1))?;
            let columns = words
                .find_map(|w| w.strip_prefix("cols="))
                .and_then(|c| c.parse().ok())
                .ok_or(format!("line {}: screen without cols=", n + 1))?;
            screens.push(Screen {
                id: id.to_owned(),
                summary: summary.trim().to_owned(),
                columns,
                ..Default::default()
            });
            continue;
        }
        let screen = screens
            .last_mut()
            .ok_or(format!("line {}: row before any screen", n + 1))?;
        if let Some(body) = line.strip_prefix("--- ") {
            let (kind, value) = body
                .split_once(':')
                .ok_or(format!("line {}: bad expectation", n + 1))?;
            screen.expected.push(Expected {
                kind: kind.to_owned(),
                value: value.trim().to_owned(),
            });
            continue;
        }
        let bar = line
            .find('|')
            .ok_or(format!("line {}: bad row {line:?}", n + 1))?;
        let tag = &line[..bar];
        let label = tag
            .chars()
            .next()
            .and_then(RowLabel::from_code)
            .ok_or(format!("line {}: bad label {tag:?}", n + 1))?;
        let text = &line[bar + 1..];
        let text = text.strip_prefix(' ').unwrap_or(text);
        if tag.ends_with('~') {
            screen.joins.insert(screen.rows.len());
        }
        screen.rows.push(text.to_owned());
        screen.labels.push(label);
    }
    Ok(screens)
}

/// Compares command values ignoring whitespace and backslash line joins.
pub fn normalise(value: &str) -> String {
    value
        .replace("\\\n", "")
        .replace(" \\ ", " ")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_heldout() {
        let screens = parse(include_str!("../data/heldout.txt")).unwrap();
        assert!(screens.len() >= 30);
        for s in &screens {
            assert_eq!(s.rows.len(), s.labels.len());
            assert!(s.columns > 0, "{}", s.id);
        }
        let soft = screens.iter().find(|s| s.id == "curl-softwrap").unwrap();
        assert_eq!(soft.joins.iter().copied().collect::<Vec<_>>(), vec![0, 1]);
    }
}
