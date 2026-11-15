//! Trains the row model on labelled screens and writes `model/rows.json`.
//!
//!   azj-hints-train --train <corpus.txt> [--dev <corpus.txt>] --out <rows.json>
//!                   [--buckets 16384] [--epochs 12] [--seed 7]
//!
//! An averaged structured perceptron over the hashed features in
//! `features.rs`, with first-order label transitions (a linear-chain model,
//! decoded with Viterbi). `--dev` only reports accuracy per epoch; use a
//! synthetic split there, never the held-out screens. `just hints-model`
//! runs this on `tools/generate.py`'s output.

use std::process::ExitCode;

use azulejo_hints::model::{Model, SCORED};
use azulejo_hints::{RowLabel, corpus, features};

const L: usize = RowLabel::COUNT;

struct Example {
    feats: Vec<Vec<u32>>,
    gold: Vec<usize>,
    blank: Vec<bool>,
}

fn examples(screens: &[corpus::Screen], buckets: usize) -> Vec<Example> {
    screens
        .iter()
        .map(|s| {
            let rows = s.row_refs();
            let feats = features::screen(&rows, &s.join_list(), Some(s.columns))
                .into_iter()
                .map(|f| {
                    f.into_iter()
                        .map(|h| (h as usize % buckets) as u32)
                        .collect()
                })
                .collect();
            Example {
                feats,
                gold: s.labels.iter().map(|l| l.index()).collect(),
                blank: rows.iter().map(|r| features::is_blank(r)).collect(),
            }
        })
        .collect()
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn emissions(model: &Model, ex: &Example) -> Vec<[f32; SCORED]> {
    ex.feats
        .iter()
        .map(|f| {
            let mut s = [0.0; SCORED];
            for &b in f {
                let b = b as usize * SCORED;
                for (k, v) in s.iter_mut().enumerate() {
                    *v += model.emission[b + k];
                }
            }
            s
        })
        .collect()
}

fn accuracy(model: &Model, set: &[Example]) -> (usize, usize) {
    let mut right = 0;
    let mut total = 0;
    for ex in set {
        let pred = model.decode(&emissions(model, ex), &ex.blank);
        for (i, (&p, &g)) in pred.iter().zip(&ex.gold).enumerate() {
            if ex.blank[i] {
                continue;
            }
            total += 1;
            if p == g {
                right += 1;
            }
        }
    }
    (right, total)
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut train = None;
    let mut dev = None;
    let mut out = None;
    let mut buckets = 16384usize;
    let mut epochs = 12usize;
    let mut seed = 7u64;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--train" => train = args.next(),
            "--dev" => dev = args.next(),
            "--out" => out = args.next(),
            "--buckets" => buckets = args.next().and_then(|v| v.parse().ok()).unwrap_or(buckets),
            "--epochs" => epochs = args.next().and_then(|v| v.parse().ok()).unwrap_or(epochs),
            "--seed" => seed = args.next().and_then(|v| v.parse().ok()).unwrap_or(seed),
            other => {
                eprintln!("azj-hints-train: unknown argument {other}");
                return ExitCode::from(2);
            }
        }
    }
    let (Some(train), Some(out)) = (train, out) else {
        eprintln!(
            "usage: azj-hints-train --train <corpus> [--dev <corpus>] --out <rows.json> [--buckets N] [--epochs N] [--seed N]"
        );
        return ExitCode::from(2);
    };
    let load = |path: &str| -> Result<Vec<corpus::Screen>, String> {
        corpus::parse(&std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?)
    };
    let train_screens = match load(&train) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("azj-hints-train: {e}");
            return ExitCode::FAILURE;
        }
    };
    let dev_screens = match dev.as_deref().map(load).transpose() {
        Ok(s) => s.unwrap_or_default(),
        Err(e) => {
            eprintln!("azj-hints-train: {e}");
            return ExitCode::FAILURE;
        }
    };
    let rows: usize = train_screens.iter().map(|s| s.rows.len()).sum();
    eprintln!(
        "train: {} screens, {rows} rows; {buckets} buckets",
        train_screens.len()
    );
    let train_set = examples(&train_screens, buckets);
    let dev_set = examples(&dev_screens, buckets);

    let mut model = Model {
        buckets,
        emission: vec![0.0; buckets * SCORED],
        transitions: [[0.0; L]; L],
        start: [0.0; L],
    };
    // Averaging: sum of c·update, subtracted at the end (Daumé's trick).
    let mut acc_e = vec![0.0f32; buckets * SCORED];
    let mut acc_t = [[0.0f32; L]; L];
    let mut acc_s = [0.0f32; L];
    let mut c = 1.0f32;
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut order: Vec<usize> = (0..train_set.len()).collect();
    for epoch in 0..epochs {
        for i in (1..order.len()).rev() {
            let j = (rng.next() % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
        let mut mistakes = 0;
        for &k in &order {
            let ex = &train_set[k];
            let pred = model.decode(&emissions(&model, ex), &ex.blank);
            if pred == ex.gold {
                c += 1.0;
                continue;
            }
            for i in 0..pred.len() {
                let (g, p) = (ex.gold[i], pred[i]);
                if g != p && g < SCORED && p < SCORED {
                    mistakes += 1;
                    for &b in &ex.feats[i] {
                        let b = b as usize * SCORED;
                        model.emission[b + g] += 1.0;
                        model.emission[b + p] -= 1.0;
                        acc_e[b + g] += c;
                        acc_e[b + p] -= c;
                    }
                }
                if i == 0 {
                    model.start[g] += 1.0;
                    model.start[p] -= 1.0;
                    acc_s[g] += c;
                    acc_s[p] -= c;
                } else {
                    let (gp, pp) = (ex.gold[i - 1], pred[i - 1]);
                    model.transitions[gp][g] += 1.0;
                    model.transitions[pp][p] -= 1.0;
                    acc_t[gp][g] += c;
                    acc_t[pp][p] -= c;
                }
            }
            c += 1.0;
        }
        let averaged = average(&model, &acc_e, &acc_t, &acc_s, c);
        let (r, t) = accuracy(&averaged, &dev_set);
        if t > 0 {
            eprintln!(
                "epoch {:2}: {mistakes} row mistakes; dev {:.4} ({r}/{t})",
                epoch + 1,
                r as f64 / t as f64
            );
        } else {
            eprintln!("epoch {:2}: {mistakes} row mistakes", epoch + 1);
        }
    }
    let model = average(&model, &acc_e, &acc_t, &acc_s, c);
    let trained = serde_json::json!({
        "algorithm": "averaged structured perceptron, first-order transitions",
        "screens": train_screens.len(),
        "rows": rows,
        "epochs": epochs,
        "seed": seed,
    });
    if let Err(e) = std::fs::write(&out, model.to_json(trained)) {
        eprintln!("azj-hints-train: {out}: {e}");
        return ExitCode::FAILURE;
    }
    eprintln!("wrote {out}");
    ExitCode::SUCCESS
}

#[allow(clippy::needless_range_loop)] // Label indices address several tables.
fn average(model: &Model, acc_e: &[f32], acc_t: &[[f32; L]; L], acc_s: &[f32; L], c: f32) -> Model {
    let mut avg = Model {
        buckets: model.buckets,
        emission: model
            .emission
            .iter()
            .zip(acc_e)
            .map(|(w, a)| w - a / c)
            .collect(),
        transitions: model.transitions,
        start: model.start,
    };
    for x in 0..L {
        avg.start[x] -= acc_s[x] / c;
        for y in 0..L {
            avg.transitions[x][y] -= acc_t[x][y] / c;
        }
    }
    avg
}
