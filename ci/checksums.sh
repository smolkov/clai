#!/bin/sh
# Writes DIR/SHA256SUMS for the archives and packages in DIR.
#
#   ci/checksums.sh [DIR]

. "$(dirname -- "$0")/common.sh"

dir=${1:-dist}
[ -d "$dir" ] || fail "$dir not found"
(
    cd "$dir"
    set -- *.tar.gz *.deb
    files=""
    for f in "$@"; do
        [ -f "$f" ] && files="$files $f"
    done
    [ -n "$files" ] || fail "no archives or packages in $dir"
    # shellcheck disable=SC2086 # file names have no spaces.
    sha256 $files >SHA256SUMS
)
say "wrote $dir/SHA256SUMS"
cat "$dir/SHA256SUMS" >&2
