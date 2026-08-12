#!/bin/sh
set -e

echo "[pre-commit] Running Rust checks"

set -x

cargo fmt --all --check
cargo test --all --locked
cargo clippy --all --locked
cargo doc --all --locked --no-deps
