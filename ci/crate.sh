#!/bin/sh
# Packs the crate as crates.io would get it, and checks it: it builds, its
# tests pass, it holds every source file and nothing of the project's
# notes, and it is under 10 MiB.
#
#   ci/crate.sh [--allow-dirty] [--no-test]

. "$(dirname -- "$0")/common.sh"

package_args=""
run_tests=1
for arg in "$@"; do
    case "$arg" in
        --allow-dirty) package_args="--allow-dirty" ;;
        --no-test) run_tests=0 ;;
        *) fail "usage: ci/crate.sh [--allow-dirty] [--no-test]" ;;
    esac
done

version=$(crate_version)
list=target/crate-files.txt
# shellcheck disable=SC2086 # package_args is one word or empty.
cargo package --locked --list $package_args >"$list"

# Every tracked file the build or the tests read must be in the crate.
missing=$(git ls-files src themes tests | while read -r f; do
    grep -qxF "$f" "$list" || printf '%s\n' "$f"
done)
[ -z "$missing" ] || fail "not in the crate (add to include in Cargo.toml):
$missing"

# Nothing of the project's notes, pipelines or build output.
extra=$(grep -E '^(rfcs|spec|ci|debian|target|\.github)/|^(todo|agent)\.md$|^\.gitlab-ci\.yml$' "$list" || true)
[ -z "$extra" ] || fail "in the crate but should not be:
$extra"
for f in Cargo.toml Cargo.lock readme.md LICENSE-MIT LICENSE-APACHE CHANGELOG.md; do
    grep -qxF "$f" "$list" || fail "$f is not in the crate"
done

say "cargo package (builds the packed crate)"
# shellcheck disable=SC2086
cargo package --locked $package_args >&2

crate="target/package/clai-$version.crate"
[ -f "$crate" ] || fail "$crate was not written"
size=$(wc -c <"$crate" | tr -d ' ')
[ "$size" -lt 10485760 ] || fail "$crate is $size bytes; crates.io takes at most 10 MiB"
say "$crate: $size bytes, $(wc -l <"$list" | tr -d ' ') files"

if [ "$run_tests" = 1 ]; then
    say "cargo test in the packed crate"
    (cd "target/package/clai-$version" && cargo test --locked --quiet) >&2
fi
