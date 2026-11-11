#!/bin/sh
# The test suite and the tests of the ci/ scripts.
#
#   ci/test.sh [CARGO TEST ARGS...]

. "$(dirname -- "$0")/common.sh"

say "cargo test"
cargo test --locked "$@"
say "ci script tests"
sh tests/ci.sh
