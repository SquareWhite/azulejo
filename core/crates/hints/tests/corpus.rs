//! The ⌘S detector on its corpus (data/hints-corpus.txt): every fixture
//! finds its commands and items, except the known misses (`@@ xfail`).
//!
//! `cargo test -p azulejo-hints --test corpus -- --nocapture` prints the
//! per-category recall and precision; add `--release --ignored` for the
//! full-screen timing.

use std::time::Instant;

use azulejo_hints::detect::fixtures::{Fixture, evaluate, load};
use azulejo_hints::detect::{Input, StdFilesystem, detect};

fn fixtures() -> Vec<Fixture> {
    load(include_str!("../data/hints-corpus.txt")).expect("corpus parses")
}

#[test]
fn corpus() {
    let fixtures = fixtures();
    let report = evaluate(&fixtures, &StdFilesystem);
    println!(
        "hint corpus (command recognition):\n{}",
        report.describe(true)
    );
    let failing: Vec<&str> = report
        .failures
        .iter()
        .filter(|(f, _)| f.xfail.is_none())
        .map(|(f, _)| f.name.as_str())
        .collect();
    assert!(failing.is_empty(), "corpus fixtures fail: {failing:?}");
    let xfails = report
        .failures
        .iter()
        .filter(|(f, _)| f.xfail.is_some())
        .count();
    assert_eq!(xfails, 1, "one known miss");
    let total = report.total();
    assert_eq!(
        (total.found, total.missed, total.wrong),
        (173, 1, 0),
        "recall 99.4%, precision 100%"
    );
}

/// A full screen (rows of every fixture, 60 at a time at 120 columns) with
/// filesystem checks on, as the overlay runs it. Median of runs.
#[test]
#[ignore = "timing: run with --release --ignored --nocapture"]
fn full_screen_timing() {
    let fixtures = fixtures();
    let all: Vec<&str> = fixtures
        .iter()
        .flat_map(|f| f.rows.iter())
        .filter(|r| !r.contains('\u{1b}'))
        .map(String::as_str)
        .collect();
    let cwd = std::env::temp_dir();
    let cwd = cwd.to_str().unwrap();
    let mut samples = Vec::new();
    for _ in 0..15 {
        for rows in all.chunks(60) {
            let start = Instant::now();
            let input = Input {
                rows,
                columns: Some(120),
                cwd: Some(cwd),
                local: true,
                branch: Some("main"),
                ..Input::default()
            };
            std::hint::black_box(detect(&input, &StdFilesystem));
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "full screen: {:.3} ms (median of {})",
        samples[samples.len() / 2],
        samples.len()
    );
}
