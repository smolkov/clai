#!/bin/sh
# Prints the release notes of VERSION: its section of CHANGELOG.md, without
# the heading. Fails when there is no such section or it is empty.
#
#   ci/notes.sh VERSION [CHANGELOG]

. "$(dirname -- "$0")/common.sh"

[ $# -ge 1 ] && [ $# -le 2 ] || fail "usage: ci/notes.sh VERSION [CHANGELOG]"
version=$1
file=${2:-CHANGELOG.md}
[ -f "$file" ] || fail "$file not found"

notes=$(awk -v v="$version" '
    /^## / {
        if (found) exit
        h = substr($0, 4)
        if (h == v || index(h, v " ") == 1) { found = 1; next }
    }
    found { print }
' "$file" | sed -e '/./,$!d')

[ -n "$(printf '%s' "$notes" | tr -d '[:space:]')" ] \
    || fail "no notes for $version in $file; add a section \"## $version - YYYY-MM-DD\""
printf '%s\n' "$notes"
