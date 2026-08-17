#!/bin/sh
set -e

readonly DIRTY_STAGED_FILES="$(comm -12 <(git diff --cached --name-only | sort) <(git diff --name-only | sort))"

echo "[pre-commit] Running Rust checks"

if [ -z "$DIRTY_STAGED_FILES" ]; then
    echo "[pre-commmit] Safely fixing code first"

    set -x
    cargo fix --allow-dirty --workspace --locked
    cargo clippy --fix --allow-dirty --workspace --locked
    cargo fmt --all
    set +x

    git diff --name-only --cached | xargs -r git add
fi

set -x
cargo fmt --all --check
cargo test --all --locked
cargo clippy --all --locked
cargo doc --all --locked --no-deps
