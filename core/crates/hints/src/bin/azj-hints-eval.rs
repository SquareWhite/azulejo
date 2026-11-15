//! Scores the row model on hand-labelled screens.
//!
//!   azj-hints-eval --corpus data/heldout.txt [--detector] [--model rows.json] [--margin N] [--no-suppress] [--no-add] [--verbose]
//!
//! Prints row accuracy per label, a confusion matrix, latency, and command
//! precision and recall: from the labels alone and, with `--detector`, for
//! the ⌘S detector ([`azulejo_hints::detect`]) alone and refined by the
//! labels.

use std::collections::{BTreeSet, HashMap};
use std::process::ExitCode;
use std::time::Instant;

use azulejo_hints::detect::{Input, Kind, StdFilesystem, detect};
use azulejo_hints::refine::{self, Command};
use azulejo_hints::{Model, RowLabel, corpus};

#[derive(Default, Clone, Copy)]
struct Score {
    tp: usize,
    fp: usize,
    fn_: usize,
}

impl Score {
    fn add(&mut self, expected: &BTreeSet<String>, got: &BTreeSet<String>) {
        self.tp += expected.intersection(got).count();
        self.fn_ += expected.difference(got).count();
        self.fp += got.difference(expected).count();
    }
    fn line(&self) -> String {
        let p = self.tp as f64 / (self.tp + self.fp).max(1) as f64;
        let r = self.tp as f64 / (self.tp + self.fn_).max(1) as f64;
        format!(
            "P {p:.2}  R {r:.2}  (tp {}, fn {}, fp {})",
            self.tp, self.fn_, self.fp
        )
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut corpus_path = None;
    let mut detector = false;
    let mut model_path = None;
    let mut verbose = false;
    let mut policy = refine::Policy::default();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--corpus" => corpus_path = args.next(),
            "--detector" => detector = true,
            "--model" => model_path = args.next(),
            "--verbose" | "-v" => verbose = true,
            "--margin" => {
                policy.min_margin = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(policy.min_margin)
            }
            "--no-suppress" => policy.suppress = false,
            "--no-add" => policy.add = false,
            other => {
                eprintln!("azj-hints-eval: unknown argument {other}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(corpus_path) = corpus_path else {
        eprintln!(
            "usage: azj-hints-eval --corpus <file> [--detector] [--model <rows.json>] [--verbose]"
        );
        return ExitCode::from(2);
    };
    let screens = match std::fs::read_to_string(&corpus_path)
        .map_err(|e| e.to_string())
        .and_then(|t| corpus::parse(&t))
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("azj-hints-eval: {corpus_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let owned;
    let model: &Model = match model_path {
        Some(p) => match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|t| Model::from_json(&t))
        {
            Ok(m) => {
                owned = m;
                &owned
            }
            Err(e) => {
                eprintln!("azj-hints-eval: {p}: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => Model::embedded(),
    };
    // The detector's commands, without the classifier: it is what the
    // labels refine below.
    let regex: HashMap<String, Vec<Command>> = if detector {
        screens
            .iter()
            .map(|s| {
                let rows = s.row_refs();
                let joins = s.join_list();
                let input = Input {
                    rows: &rows,
                    joins: &joins,
                    columns: Some(s.columns),
                    ..Input::default()
                };
                let commands = detect(&input, &StdFilesystem)
                    .into_iter()
                    .filter(|i| i.kind == Kind::Command)
                    .map(|i| Command {
                        value: i.value,
                        spans: i.spans,
                    })
                    .collect();
                (s.id.clone(), commands)
            })
            .collect()
    } else {
        HashMap::new()
    };

    const L: usize = RowLabel::COUNT;
    let mut confusion = [[0usize; L]; L];
    let mut times = Vec::new();
    let mut per_row = Vec::new();
    let mut labels_only = Score::default();
    let mut regex_only = Score::default();
    let mut combined = Score::default();
    let mut changes = refine::Changes::default();
    for s in &screens {
        let rows = s.row_refs();
        let joins = s.join_list();
        let t0 = Instant::now();
        let labelling = model.label(&rows, &joins, Some(s.columns));
        let pred = labelling.labels.clone();
        let dt = t0.elapsed().as_secs_f64() * 1e3;
        times.push(dt);
        per_row.push(dt * 1e3 / rows.len().max(1) as f64);
        for (g, p) in s.labels.iter().zip(&pred) {
            confusion[g.index()][p.index()] += 1;
        }
        if verbose {
            for (i, (g, p)) in s.labels.iter().zip(&pred).enumerate() {
                if g != p {
                    println!(
                        "  {} row {i:2}: want {} got {}  | {}",
                        s.id,
                        g.code(),
                        p.code(),
                        rows[i]
                    );
                }
            }
        }
        let expected: BTreeSet<String> = s
            .expected
            .iter()
            .filter(|e| e.kind == "command")
            .map(|e| corpus::normalise(&e.value))
            .collect();
        let ours = refine::label_commands(&rows, &joins, &pred, Some(s.columns));
        let got: BTreeSet<String> = ours.iter().map(|c| corpus::normalise(&c.value)).collect();
        labels_only.add(&expected, &got);
        if let Some(rx) = regex.get(&s.id) {
            let got_rx: BTreeSet<String> = rx.iter().map(|c| corpus::normalise(&c.value)).collect();
            regex_only.add(&expected, &got_rx);
            let (merged, ch) =
                refine::refine_with(&rows, &joins, Some(s.columns), &labelling, rx, &policy);
            changes.suppressed += ch.suppressed;
            changes.replaced += ch.replaced;
            changes.added += ch.added;
            let got_m: BTreeSet<String> =
                merged.iter().map(|c| corpus::normalise(&c.value)).collect();
            combined.add(&expected, &got_m);
            if verbose {
                let raw = |set: &BTreeSet<String>, cmds: &[Command]| -> Vec<String> {
                    cmds.iter()
                        .filter(|c| set.contains(&corpus::normalise(&c.value)))
                        .map(|c| c.value.clone())
                        .collect()
                };
                let fp_m: BTreeSet<String> = got_m.difference(&expected).cloned().collect();
                let fn_m: BTreeSet<String> = expected.difference(&got_m).cloned().collect();
                for v in raw(&fp_m, &merged) {
                    println!("  {} FP {v:?}", s.id);
                }
                for e in s
                    .expected
                    .iter()
                    .filter(|e| e.kind == "command" && fn_m.contains(&corpus::normalise(&e.value)))
                {
                    println!("  {} FN {:?}", s.id, e.value);
                }
            }
        }
    }

    let total: usize = confusion.iter().flatten().sum();
    let right: usize = (0..L).map(|i| confusion[i][i]).sum();
    let blank = confusion[RowLabel::Blank.index()][RowLabel::Blank.index()];
    let blank_rows: usize = confusion[RowLabel::Blank.index()].iter().sum();
    println!(
        "{} screens, {} rows ({} blank)",
        screens.len(),
        total,
        blank_rows
    );
    println!(
        "row accuracy: {:.3} all rows ({right}/{total}); {:.3} non-blank ({}/{})",
        right as f64 / total.max(1) as f64,
        (right - blank) as f64 / (total - blank_rows).max(1) as f64,
        right - blank,
        total - blank_rows
    );
    println!();
    println!(
        "{:<13} {:>6} {:>7} {:>9}",
        "label", "rows", "recall", "precision"
    );
    for l in RowLabel::ALL {
        let i = l.index();
        let support: usize = confusion[i].iter().sum();
        let predicted: usize = (0..L).map(|g| confusion[g][i]).sum();
        println!(
            "{:<13} {:>6} {:>7.3} {:>9.3}",
            format!("{} {}", l.code(), l.name()),
            support,
            confusion[i][i] as f64 / support.max(1) as f64,
            confusion[i][i] as f64 / predicted.max(1) as f64
        );
    }
    println!();
    print!("confusion (rows: truth, columns: predicted)\n      ");
    for l in RowLabel::ALL {
        print!("{:>5}", l.code());
    }
    println!();
    for g in RowLabel::ALL {
        print!("  {:<4}", g.code());
        for p in RowLabel::ALL {
            print!("{:>5}", confusion[g.index()][p.index()]);
        }
        println!();
    }
    println!();
    println!("commands, labels alone:      {}", labels_only.line());
    if !regex.is_empty() {
        println!("commands, detector alone:    {}", regex_only.line());
        println!("commands, detector + labels: {}", combined.line());
        println!(
            "  refine: {} suppressed, {} replaced, {} added",
            changes.suppressed, changes.replaced, changes.added
        );
    }
    // A full 50-row screen, classified and refined, many times over.
    let mut tall: Vec<&str> = Vec::new();
    let mut tall_joins = Vec::new();
    for s in &screens {
        for (i, r) in s.rows.iter().enumerate() {
            if tall.len() < 50 {
                if s.joins.contains(&i) {
                    tall_joins.push(tall.len());
                }
                tall.push(r);
            }
        }
    }
    let mut bench = Vec::new();
    for _ in 0..500 {
        let t0 = Instant::now();
        let l = model.label(&tall, &tall_joins, Some(100));
        let _ = refine::refine_with(&tall, &tall_joins, Some(100), &l, &[], &policy);
        bench.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    bench.sort_by(f64::total_cmp);
    times.sort_by(f64::total_cmp);
    per_row.sort_by(f64::total_cmp);
    println!();
    println!(
        "latency: median {:.3} ms per screen, max {:.3} ms; median {:.1} µs per row (first call loads the model)",
        times[times.len() / 2],
        times[times.len() - 1],
        per_row[per_row.len() / 2]
    );
    println!(
        "latency, {}-row screen, label + refine: median {:.3} ms, p99 {:.3} ms",
        tall.len(),
        bench[bench.len() / 2],
        bench[bench.len() * 99 / 100]
    );
    ExitCode::SUCCESS
}
