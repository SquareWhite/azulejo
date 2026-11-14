#!/usr/bin/env bash
# Spike 0.7 — reproduce from macOS (or Linux) with Docker:
#   ./run.sh            build the image and run the whole spike
#   ./run.sh shell      drop into the container
# Outputs land in ./out/ (screenshots, grid dumps, logs).
set -euo pipefail
cd "$(dirname "$0")"

IMAGE=azj-linux-terminal-spike
docker build -t "$IMAGE" . >out/build.log 2>&1 || { tail -40 out/build.log; exit 1; }

args=(--rm -v "$PWD:/spike" -v azj-spike-cargo-target:/spike/vte/target "$IMAGE")
if [[ "${1:-}" == shell ]]; then
  exec docker run -it "${args[@]}" bash
fi
docker run "${args[@]}" bash /spike/in-container.sh
