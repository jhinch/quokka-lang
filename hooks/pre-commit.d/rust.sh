#!/bin/sh
set -e

echo "[pre-commit] Running Rust checks"

set -x

cargo fmt --all --check
cargo test --all --locked
