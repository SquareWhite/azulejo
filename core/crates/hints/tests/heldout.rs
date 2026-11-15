//! Floors for the embedded model on the hand-written screens, so a retrain
//! that quietly gets worse fails `cargo test`. The full report comes from
//! `just hints-model` (tools/model.sh).

use azulejo_hints::refine::label_commands;
use azulejo_hints::{Model, RowLabel, corpus};

fn score(text: &str) -> (f64, f64) {
    let screens = corpus::parse(text).unwrap();
    let (mut right, mut total, mut tp, mut found) = (0, 0, 0, 0);
    for s in &screens {
        let rows = s.row_refs();
        let labels = Model::embedded().classify(&rows, &s.join_list(), Some(s.columns));
        for (g, p) in s.labels.iter().zip(&labels) {
            if *g != RowLabel::Blank {
                total += 1;
                right += usize::from(g == p);
            }
        }
        let expected: Vec<String> = s
            .expected
            .iter()
            .filter(|e| e.kind == "command")
            .map(|e| corpus::normalise(&e.value))
            .collect();
        for c in label_commands(&rows, &s.join_list(), &labels, Some(s.columns)) {
            found += 1;
            tp += usize::from(expected.contains(&corpus::normalise(&c.value)));
        }
    }
    (right as f64 / total as f64, tp as f64 / found.max(1) as f64)
}

#[test]
fn heldout_floor() {
    let (rows, precision) = score(include_str!("../data/heldout.txt"));
    assert!(rows >= 0.97, "heldout row accuracy {rows:.3}");
    assert!(
        precision >= 0.95,
        "heldout command precision {precision:.3}"
    );
}

#[test]
fn blind_floors() {
    let (rows, precision) = score(include_str!("../data/blind.txt"));
    assert!(rows >= 0.90, "blind row accuracy {rows:.3}");
    assert!(precision >= 0.85, "blind command precision {precision:.3}");
    let (rows, precision) = score(include_str!("../data/blind2.txt"));
    assert!(rows >= 0.88, "blind2 row accuracy {rows:.3}");
    assert!(precision >= 0.85, "blind2 command precision {precision:.3}");
}
