//! The linear-chain model: per-row label scores from hashed features, plus
//! label-to-label transition scores, decoded with Viterbi. Blank rows are
//! fixed to [`RowLabel::Blank`]; the other six labels compete everywhere
//! else.
//!
//! The weights file (`model/rows.json`) holds the bucket count, a scale and
//! the emission weights as base64 int8 (`buckets × 6`, row-major by bucket),
//! plus float transition and start scores in the same units.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::{RowLabel, features};

/// Labels the model scores (all but Blank), in `RowLabel` order.
pub const SCORED: usize = 6;
const BLANK: usize = RowLabel::Blank as usize;
const L: usize = RowLabel::COUNT;

/// A screen's row labels and how sure the model is of each.
#[derive(Debug, Clone, PartialEq)]
pub struct Labelling {
    pub labels: Vec<RowLabel>,
    pub margins: Vec<f32>,
}

pub struct Model {
    pub buckets: usize,
    /// `buckets × SCORED`.
    pub emission: Vec<f32>,
    /// `transitions[from][to]`.
    pub transitions: [[f32; L]; L],
    pub start: [f32; L],
}

#[derive(Serialize, Deserialize)]
pub struct File {
    pub version: u32,
    pub buckets: usize,
    pub labels: String,
    pub scale: f32,
    pub transitions: Vec<Vec<f32>>,
    pub start: Vec<f32>,
    /// What the weights were trained on, for people reading the file.
    #[serde(default)]
    pub trained: serde_json::Value,
    pub weights: String,
}

static EMBEDDED: OnceLock<Model> = OnceLock::new();

impl Model {
    /// The model shipped in `model/rows.json`.
    pub fn embedded() -> &'static Model {
        EMBEDDED.get_or_init(|| {
            Model::from_json(include_str!("../model/rows.json")).expect("embedded hints model")
        })
    }

    pub fn from_json(text: &str) -> Result<Model, String> {
        let file: File = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if file.version != 1 {
            return Err(format!("unknown model version {}", file.version));
        }
        let expected: String = RowLabel::ALL[..SCORED].iter().map(|l| l.code()).collect();
        if file.labels != expected {
            return Err(format!("model labels {} != {expected}", file.labels));
        }
        let bytes = base64_decode(&file.weights)?;
        if bytes.len() != file.buckets * SCORED {
            return Err(format!(
                "{} weight bytes for {} buckets",
                bytes.len(),
                file.buckets
            ));
        }
        let emission = bytes
            .iter()
            .map(|&b| (b as i8) as f32 * file.scale)
            .collect();
        let mut transitions = [[0.0; L]; L];
        for (from, row) in file.transitions.iter().enumerate().take(L) {
            for (to, &v) in row.iter().enumerate().take(L) {
                transitions[from][to] = v;
            }
        }
        let mut start = [0.0; L];
        for (i, &v) in file.start.iter().enumerate().take(L) {
            start[i] = v;
        }
        Ok(Model {
            buckets: file.buckets,
            emission,
            transitions,
            start,
        })
    }

    /// Quantises to int8 and writes the file format.
    pub fn to_json(&self, trained: serde_json::Value) -> String {
        let max = self
            .emission
            .iter()
            .fold(0f32, |m, w| m.max(w.abs()))
            .max(1e-9);
        let scale = max / 127.0;
        let bytes: Vec<u8> = self
            .emission
            .iter()
            .map(|w| ((w / scale).round().clamp(-127.0, 127.0) as i8) as u8)
            .collect();
        let file = File {
            version: 1,
            buckets: self.buckets,
            labels: RowLabel::ALL[..SCORED].iter().map(|l| l.code()).collect(),
            scale,
            transitions: self
                .transitions
                .iter()
                .map(|r| r.iter().map(|v| round(*v)).collect())
                .collect(),
            start: self.start.iter().map(|v| round(*v)).collect(),
            trained,
            weights: base64_encode(&bytes),
        };
        let mut text = serde_json::to_string_pretty(&file).expect("serialise model");
        text.push('\n');
        text
    }

    pub fn bucket(&self, hash: u32) -> usize {
        (hash as usize) % self.buckets
    }

    /// Per-row scores for the six scored labels.
    pub fn scores(&self, features: &[u32]) -> [f32; SCORED] {
        let mut s = [0.0; SCORED];
        for &h in features {
            let b = self.bucket(h) * SCORED;
            for (k, v) in s.iter_mut().enumerate() {
                *v += self.emission[b + k];
            }
        }
        s
    }

    pub fn classify(
        &self,
        rows: &[&str],
        joins: &[usize],
        columns: Option<usize>,
    ) -> Vec<RowLabel> {
        self.label(rows, joins, columns).labels
    }

    /// Labels plus, for each row, how much the best labelling of the whole
    /// screen beats the best one that gives the row another label (a
    /// max-marginal gap, in model units; infinite for blank rows).
    pub fn label(&self, rows: &[&str], joins: &[usize], columns: Option<usize>) -> Labelling {
        let feats = features::screen(rows, joins, columns);
        let blank: Vec<bool> = rows.iter().map(|r| features::is_blank(r)).collect();
        let emissions: Vec<[f32; SCORED]> = feats.iter().map(|f| self.scores(f)).collect();
        let path = self.decode(&emissions, &blank);
        let margins = self.margins(&emissions, &blank, &path);
        Labelling {
            labels: path.into_iter().map(|i| RowLabel::ALL[i]).collect(),
            margins,
        }
    }

    fn emit(emissions: &[[f32; SCORED]], blank: &[bool], i: usize, y: usize) -> f32 {
        match (blank[i], y == BLANK) {
            (true, true) => 0.0,
            (true, false) | (false, true) => f32::NEG_INFINITY,
            (false, false) => emissions[i][y],
        }
    }

    fn margins(&self, emissions: &[[f32; SCORED]], blank: &[bool], path: &[usize]) -> Vec<f32> {
        let n = emissions.len();
        let mut alpha = vec![[f32::NEG_INFINITY; L]; n];
        let mut beta = vec![[0.0f32; L]; n];
        for i in 0..n {
            for y in 0..L {
                let e = Self::emit(emissions, blank, i, y);
                alpha[i][y] = if i == 0 {
                    self.start[y] + e
                } else {
                    (0..L)
                        .map(|x| alpha[i - 1][x] + self.transitions[x][y])
                        .fold(f32::NEG_INFINITY, f32::max)
                        + e
                };
            }
        }
        for i in (0..n.saturating_sub(1)).rev() {
            for x in 0..L {
                beta[i][x] = (0..L)
                    .map(|y| {
                        self.transitions[x][y]
                            + Self::emit(emissions, blank, i + 1, y)
                            + beta[i + 1][y]
                    })
                    .fold(f32::NEG_INFINITY, f32::max);
            }
        }
        (0..n)
            .map(|i| {
                if blank[i] {
                    return f32::INFINITY;
                }
                let best = alpha[i][path[i]] + beta[i][path[i]];
                let other = (0..L)
                    .filter(|&y| y != path[i])
                    .map(|y| alpha[i][y] + beta[i][y])
                    .fold(f32::NEG_INFINITY, f32::max);
                best - other
            })
            .collect()
    }

    /// Viterbi over label indices.
    #[allow(clippy::needless_range_loop)] // Label indices address several tables.
    pub fn decode(&self, emissions: &[[f32; SCORED]], blank: &[bool]) -> Vec<usize> {
        let n = emissions.len();
        if n == 0 {
            return Vec::new();
        }
        let emit = |i: usize, y: usize| Self::emit(emissions, blank, i, y);
        let mut score = vec![[f32::NEG_INFINITY; L]; n];
        let mut back = vec![[0u8; L]; n];
        for y in 0..L {
            score[0][y] = self.start[y] + emit(0, y);
        }
        for i in 1..n {
            for y in 0..L {
                let e = emit(i, y);
                if e == f32::NEG_INFINITY {
                    continue;
                }
                let mut best = f32::NEG_INFINITY;
                let mut arg = 0;
                for x in 0..L {
                    let v = score[i - 1][x] + self.transitions[x][y];
                    if v > best {
                        best = v;
                        arg = x;
                    }
                }
                score[i][y] = best + e;
                back[i][y] = arg as u8;
            }
        }
        let mut y = (0..L)
            .max_by(|&a, &b| score[n - 1][a].total_cmp(&score[n - 1][b]))
            .unwrap_or(0);
        let mut path = vec![0; n];
        for i in (0..n).rev() {
            path[i] = y;
            y = back[i][y] as usize;
        }
        path
    }
}

fn round(v: f32) -> f32 {
    (v * 1e4).round() / 1e4
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return Err(format!("bad base64 byte {c}")),
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for len in 0..10 {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 200) as u8).collect();
            assert_eq!(base64_decode(&base64_encode(&bytes)).unwrap(), bytes);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
    }

    #[test]
    fn embedded_model_loads() {
        let m = Model::embedded();
        assert_eq!(m.emission.len(), m.buckets * SCORED);
    }

    #[test]
    fn margins_are_positive_for_the_chosen_labels() {
        let rows = [
            "user@devbox:~$ git status",
            "On branch main",
            "",
            "nothing to commit, working tree clean",
        ];
        let l = Model::embedded().label(&rows, &[], Some(80));
        assert_eq!(l.margins.len(), rows.len());
        assert!(l.margins.iter().all(|m| *m >= 0.0), "{:?}", l.margins);
        assert!(l.margins[2].is_infinite());
    }

    #[test]
    fn blank_rows_are_blank() {
        let rows = ["$ ls", "", "   ", "a.txt"];
        let labels = Model::embedded().classify(&rows, &[], Some(80));
        assert_eq!(labels[1], RowLabel::Blank);
        assert_eq!(labels[2], RowLabel::Blank);
        assert!(labels[0] != RowLabel::Blank && labels[3] != RowLabel::Blank);
    }
}
