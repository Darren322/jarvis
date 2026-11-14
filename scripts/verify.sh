#!/usr/bin/env bash
# The whole Verification list as one command. Run it before calling a change done.
#
#   scripts/verify.sh                     every check, whole workspace
#   scripts/verify.sh --repeat PATTERN    also run the tests matching PATTERN 20 times
#   scripts/verify.sh --repeat PATTERN N  ... N times
#
# Use --repeat for any new test of a select! race, cancellation or timing: a race
# that passes once proves nothing.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

repeat_pattern=""
repeat_count=20
if [[ "${1:-}" == "--repeat" ]]; then
    repeat_pattern="${2:?--repeat needs a test name pattern}"
    repeat_count="${3:-20}"
fi

step() { printf '\n== %s\n' "$*"; "$@"; }

step cargo fmt --all --check
step cargo check --locked --offline --all-targets --workspace
step cargo clippy --locked --offline --all-targets --workspace -- -D warnings
step cargo test --locked --offline --workspace
step git diff --check

printf '\n== banned patterns in production code\n'
# unsafe is allowed only in the sqlite-vec FFI module; unwrap() and drawn text nowhere.
if grep -rnE '\.unwrap\(\)|\bunsafe\b|draw_text' --include='*.rs' src crates/*/src \
    | grep -v '^src/storage/sqlite_vec.rs:' \
    | grep -vE '^[^:]+:[0-9]+:\s*//'; then
    echo "error: banned pattern found above (unwrap(), unsafe or draw_text)" >&2
    exit 1
fi
echo "none"

if [[ -n "$repeat_pattern" ]]; then
    printf '\n== %s x%s\n' "$repeat_pattern" "$repeat_count"
    for run in $(seq 1 "$repeat_count"); do
        cargo test --locked --offline --workspace -- "$repeat_pattern" >/dev/null 2>&1 \
            || { echo "error: run $run of $repeat_count failed for $repeat_pattern" >&2; exit 1; }
    done
    echo "$repeat_count of $repeat_count runs passed"
fi

printf '\nAll checks passed. If speech changed, also run:\n  python3 -B tests/unit/features/speech/supertonic_worker/test_supertonic_worker.py\n'
