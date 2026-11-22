//! The rules on the model spike's corpus (data/model-eval.txt), as the
//! overlay runs them (row classifier on). `just hints-model-bench` scores
//! Apple's on-device model on the same screens.
//!
//! `cargo test -p azulejo-hints --test model_eval -- --nocapture` prints
//! the misses.

use azulejo_hints::detect::fixtures::load;
use azulejo_hints::detect::{Filesystem, Input, Kind, detect};

struct NoFiles;

impl Filesystem for NoFiles {
    fn kind(&self, _: &str) -> Option<bool> {
        None
    }
    fn home(&self) -> Option<String> {
        None
    }
}

#[test]
fn rules_on_the_model_corpus() {
    let fixtures = load(include_str!("../data/model-eval.txt")).expect("corpus parses");
    assert!(
        (40..=80).contains(&fixtures.len()),
        "40 to 80 screens: {}",
        fixtures.len()
    );
    let (mut found, mut missed, mut extra) = (0, 0, 0);
    for fixture in &fixtures {
        let screen = fixture.screen();
        let rows: Vec<&str> = screen.rows.iter().map(String::as_str).collect();
        let mut got: Vec<String> = detect(
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
        for expected in &fixture.commands {
            if let Some(i) = got.iter().position(|g| g == expected) {
                got.remove(i);
                found += 1;
            } else {
                missed += 1;
                println!("{}: missed {expected:?}", fixture.name);
            }
        }
        for g in got {
            extra += 1;
            println!("{}: extra {g:?}", fixture.name);
        }
    }
    println!("rules: {found} found, {missed} missed, {extra} extra");
    // Known: a hint in parentheses ("use \"git push\"") is missed, and an
    // npm script banner under a shown command joins it.
    assert_eq!((found, missed, extra), (39, 1, 1));
}
