#!/bin/sh
# Format and lints, as AGENTS.md "Before committing" asks.
#
#   ci/check.sh

. "$(dirname -- "$0")/common.sh"

say "cargo fmt"
cargo fmt --all -- --check
say "environment reads"
sh "$(dirname -- "$0")/env-check.sh"
say "cargo clippy"
cargo clippy --all-targets --locked -- -D warnings
