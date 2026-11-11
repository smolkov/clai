#!/bin/sh
# Writes the completion files for bash, zsh and fish into DIR.
#
#   ci/completions.sh BIN DIR
#
# BIN is a clai that runs on this host (the scripts do not depend on the
# target). It is started as `clai` through PATH, so the files call `clai`
# and not the path of the build directory (see src/command/completion.rs).

. "$(dirname -- "$0")/common.sh"

[ $# -eq 2 ] || fail "usage: ci/completions.sh BIN DIR"
bin_dir=$(CDPATH='' cd -- "$(dirname -- "$1")" && pwd)
[ "$(basename -- "$1")" = clai ] || fail "BIN must be named clai: $1"
mkdir -p "$2"

PATH="$bin_dir:$PATH" clai completion bash >"$2/clai.bash"
PATH="$bin_dir:$PATH" clai completion zsh >"$2/_clai"
PATH="$bin_dir:$PATH" clai completion fish >"$2/clai.fish"

if grep -q "$bin_dir" "$2/clai.bash" "$2/_clai" "$2/clai.fish"; then
    fail "completion files name the build directory $bin_dir"
fi
