#!/usr/bin/env bash
set -euo pipefail

export MLN_PRECOMPILE=${MLN_PRECOMPILE:-1}
./scripts/vendor-rlottie.sh
cargo test --locked --workspace --no-fail-fast
cargo clippy --locked --workspace --all-targets -- -D warnings
