#!/usr/bin/env bash
# Builds and runs the GTK4 shell (apps/linux) in an Ubuntu 24.04 container.
#   run.sh check   build + clippy for azulejo-linux
#   run.sh shot    run headless under Xvfb, screenshot to target/linux-dev/
#   run.sh shell   interactive shell in the container
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
toolchain="$(sed -n 's/^channel = "\(.*\)"/\1/p' "$root/rust-toolchain.toml")"
image=azj-linux-dev

docker build -q --build-arg RUST_TOOLCHAIN="$toolchain" -t "$image" "$here" >/dev/null

# Linux build output lives in a volume so it never mixes with the host's target/.
args=(--rm -v "$root":/src -w /src
      -v azj-linux-target:/target -e CARGO_TARGET_DIR=/target
      -v azj-linux-registry:/usr/local/cargo/registry)

case "${1:-shot}" in
  check) docker run "${args[@]}" "$image" bash -c \
           "cargo build -p azulejo-linux && cargo clippy -p azulejo-linux --all-targets -- -D warnings" ;;
  shot)  docker run "${args[@]}" "$image" /src/scripts/linux-dev/screenshot.sh ;;
  shell) docker run -it "${args[@]}" "$image" bash ;;
  *)     echo "usage: $0 [check|shot|shell]" >&2; exit 2 ;;
esac
