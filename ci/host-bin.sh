#!/bin/sh
# Prints the path of a codeagent that runs on this host, for TARGET's files:
# $CODEAGENT_HOST_BIN if set, the target's own binary when it runs here, or a
# debug build for the host (completion scripts are the same for every
# target).
#
#   ci/host-bin.sh TARGET

. "$(dirname -- "$0")/common.sh"

[ $# -eq 1 ] || fail "usage: ci/host-bin.sh TARGET"
if [ -n "${CODEAGENT_HOST_BIN:-}" ]; then
    [ -x "$CODEAGENT_HOST_BIN" ] || fail "CODEAGENT_HOST_BIN=$CODEAGENT_HOST_BIN is not a program"
    printf '%s\n' "$CODEAGENT_HOST_BIN"
elif runs_here "$1" && [ -x "target/$1/release/codeagent" ]; then
    printf '%s\n' "target/$1/release/codeagent"
else
    cargo build --locked --quiet >&2
    printf '%s\n' "target/debug/codeagent"
fi
