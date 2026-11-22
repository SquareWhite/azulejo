//! Exports a fixture corpus for the on-device model bench
//! (apps/macos/tools/HintsModelBench): each screen's rows, the commands it
//! should yield, what the rules find (the row classifier on, as by
//! default), and the candidates a model may pick from, one per logical
//! line plus the rules' own commands.
//!
//!   azj-hints-candidates data/model-eval.txt > candidates.json

use std::collections::HashMap;
use std::process::ExitCode;

use azulejo_hints::detect::fixtures::load;
use azulejo_hints::detect::lines::{join_lines, soft_lines};
use azulejo_hints::detect::{Filesystem, Input, Kind, detect};
use serde_json::json;

/// No path exists: the bench must not depend on this machine's files.
struct NoFiles;

impl Filesystem for NoFiles {
    fn kind(&self, _: &str) -> Option<bool> {
        None
    }
    fn home(&self) -> Option<String> {
        None
    }
}

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: azj-hints-candidates <corpus>");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::from(2);
        }
    };
    let fixtures = match load(&text) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let mut out = Vec::new();
    for fixture in &fixtures {
        let screen = fixture.screen();
        let rows: Vec<&str> = screen.rows.iter().map(String::as_str).collect();
        let rules: Vec<String> = detect(
            &Input {
                rows: &rows,
                joins: &screen.joins,
                columns: Some(fixture.columns),
                colors: &screen.colors,
                row_classifier: true,
                ..Input::default()
            },
            &NoFiles,
        )
        .into_iter()
        .filter(|i| i.kind == Kind::Command)
        .map(|i| i.value)
        .collect();
        let mut joined = vec![false; rows.len()];
        for &j in &screen.joins {
            joined[j] = true;
        }
        let lines = join_lines(
            &soft_lines(&rows, &joined),
            Some(fixture.columns),
            &HashMap::new(),
        );
        let mut candidates: Vec<String> = lines
            .iter()
            .map(|l| l.text.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect();
        for command in &rules {
            if !candidates.contains(command) {
                candidates.push(command.clone());
            }
        }
        out.push(json!({
            "name": fixture.name,
            "category": fixture.category,
            "rows": screen.rows,
            "expected": fixture.commands,
            "rules": rules,
            "candidates": candidates,
        }));
    }
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
    ExitCode::SUCCESS
}
