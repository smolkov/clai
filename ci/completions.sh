#!/bin/sh
# Writes the completion files for bash, zsh and fish into DIR.
#
#   ci/completions.sh BIN DIR
#
# BIN is a codeagent that runs on this host (the scripts do not depend on the
# target). It is started as `codeagent` through PATH, so the files call `codeagent`
# and not the path of the build directory (see src/command/completion.rs).

. "$(dirname -- "$0")/common.sh"

[ $# -eq 2 ] || fail "usage: ci/completions.sh BIN DIR"
bin_dir=$(CDPATH='' cd -- "$(dirname -- "$1")" && pwd)
[ "$(basename -- "$1")" = codeagent ] || fail "BIN must be named codeagent: $1"
mkdir -p "$2"

PATH="$bin_dir:$PATH" codeagent completion bash >"$2/codeagent.bash"
PATH="$bin_dir:$PATH" codeagent completion zsh >"$2/_codeagent"
PATH="$bin_dir:$PATH" codeagent completion fish >"$2/codeagent.fish"

if grep -q "$bin_dir" "$2/codeagent.bash" "$2/_codeagent" "$2/codeagent.fish"; then
    fail "completion files name the build directory $bin_dir"
fi
