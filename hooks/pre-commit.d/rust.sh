#!/bin/sh
set -e

echo "[pre-commit] Running Rust checks"

set -x

cargo fmt --check
cargo test --locked
