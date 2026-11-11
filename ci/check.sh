#!/bin/sh
# Format and lints, as agent.md "Before committing" asks.
#
#   ci/check.sh

. "$(dirname -- "$0")/common.sh"

say "cargo fmt"
cargo fmt --all -- --check
say "cargo clippy"
cargo clippy --all-targets --locked -- -D warnings
