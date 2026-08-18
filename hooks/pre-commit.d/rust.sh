#!/bin/sh
set -e

readonly TMP_STAGED_FILES="$(mktemp)"
cleanup() {
    rm -f "$TMP_STAGED_FILES"
}
trap cleanup EXIT

git diff --cached --name-only | sort > "$TMP_STAGED_FILES"
readonly DIRTY_STAGED_FILES="$(git diff --name-only | sort | comm -12 "$TMP_STAGED_FILES" -)"

echo "[pre-commit] Running Rust checks"

set -x

if [ -z "$DIRTY_STAGED_FILES" ]; then
    echo "[pre-commit] Safely fixing code first"

    cargo fix --allow-dirty --workspace --locked
    cargo clippy --fix --allow-dirty --workspace --locked
    cargo fmt --all

    git diff --name-only --cached | xargs -r git add
fi

cargo fmt --all --check
cargo test --all --locked
cargo clippy --all --locked
cargo doc --all --locked --no-deps
