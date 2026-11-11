#!/bin/sh
# Prints the path of a clai that runs on this host, for TARGET's files:
# $CLAI_HOST_BIN if set, the target's own binary when it runs here, or a
# debug build for the host (completion scripts are the same for every
# target).
#
#   ci/host-bin.sh TARGET

. "$(dirname -- "$0")/common.sh"

[ $# -eq 1 ] || fail "usage: ci/host-bin.sh TARGET"
if [ -n "${CLAI_HOST_BIN:-}" ]; then
    [ -x "$CLAI_HOST_BIN" ] || fail "CLAI_HOST_BIN=$CLAI_HOST_BIN is not a program"
    printf '%s\n' "$CLAI_HOST_BIN"
elif runs_here "$1" && [ -x "target/$1/release/clai" ]; then
    printf '%s\n' "target/$1/release/clai"
else
    cargo build --locked --quiet >&2
    printf '%s\n' "target/debug/clai"
fi
