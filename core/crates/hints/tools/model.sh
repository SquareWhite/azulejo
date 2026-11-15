#!/usr/bin/env bash
# The terminal row classifier's model (`just hints-model`):
#   tools/model.sh            generate synthetic screens, train, write model/rows.json, score
#   tools/model.sh --eval     score the current model only
#   tools/model.sh --check    fail if model/rows.json is stale against its inputs (just lint)
#
# Training uses synthetic screens only (tools/generate.py, fixed seed). The
# scores come from hand-written screens the generator never reads:
# data/heldout.txt and data/blind*.txt, plus the shell detector's corpus
# (data/hints-corpus.txt; commands only, row labels there are
# placeholders). The ⌘S detector (src/detect) is scored too, alone and
# refined by the row labels. Work files go to target/hints-model/.
set -euo pipefail

root="$(cd "$(dirname "$0")/../../../.." && pwd)"
cd "$root"
crate=core/crates/hints
model=$crate/model/rows.json
stamp=$crate/model/rows.sha256
work=target/hints-model
seed=20261112
screens=6000

stamp_now() {
  shasum -a 256 "$crate/tools/generate.py" "$crate/src/features.rs" "$crate/src/text.rs" "$crate/src/model.rs" \
    "$crate/src/bin/azj-hints-train.rs" "$model"
}

if [[ "${1:-}" == --check ]]; then
  if [[ ! -f "$model" || ! -f "$stamp" ]]; then
    echo "error: $model or $stamp is missing; run 'just hints-model'" >&2
    exit 1
  fi
  if ! diff -q <(stamp_now) "$stamp" >/dev/null; then
    echo "error: $model is stale against the generator, features or trainer (or was edited by hand); run 'just hints-model'" >&2
    diff <(stamp_now) "$stamp" >&2 || true
    exit 1
  fi
  exit 0
fi

mkdir -p "$work"
cargo build -q --release -p azulejo-hints
if [[ "${1:-}" != --eval ]]; then
  python3 "$crate/tools/generate.py" --screens "$screens" --seed "$seed" > "$work/train.txt"
  python3 "$crate/tools/generate.py" --screens 400 --seed 99 > "$work/dev.txt"
  target/release/azj-hints-train --train "$work/train.txt" --dev "$work/dev.txt" --out "$model" --seed 7 --epochs 10
  # The model is embedded at compile time.
  cargo build -q --release -p azulejo-hints
  stamp_now > "$stamp"
fi

# The detector's own corpus, commands only (no row labels there).
python3 "$crate/tools/convert-hints-corpus.py" "$crate/data/hints-corpus.txt" > "$work/hints-corpus.txt"
for set in heldout blind blind2 blind3 hints-corpus; do
  corpus="$crate/data/$set.txt"
  [[ $set == hints-corpus ]] && corpus="$work/hints-corpus.txt"
  echo "== $set ($corpus)"
  target/release/azj-hints-eval --corpus "$corpus" --detector ${VERBOSE:+--verbose}
  echo
done
