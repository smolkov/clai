#!/bin/sh
# Version checks of a release.
#
#   ci/version.sh tag TAG     TAG is v plus the version in Cargo.toml;
#                             prints the version
#   ci/version.sh toolchain   .gitlab-ci.yml uses the image of the
#                             toolchain in rust-toolchain.toml
#   ci/version.sh deb VERSION prints the Debian version of VERSION
#   ci/version.sh prerelease VERSION
#                             exits 0 when VERSION is a pre-release

. "$(dirname -- "$0")/common.sh"

[ $# -ge 1 ] || fail "usage: ci/version.sh tag TAG | toolchain | deb VERSION | prerelease VERSION"
case "$1" in
    tag)
        [ $# -eq 2 ] || fail "usage: ci/version.sh tag TAG"
        version=$(crate_version)
        [ -n "$version" ] || fail "no version in Cargo.toml"
        [ "$2" = "v$version" ] || fail "tag $2 does not match the version $version in Cargo.toml (expected v$version)"
        printf '%s\n' "$version"
        ;;
    toolchain)
        want=$(toolchain_version)
        file=${CLAI_GITLAB_CI:-.gitlab-ci.yml}
        have=$(sed -n 's/^ *RUST_VERSION: "\(.*\)"$/\1/p' "$file" | head -n 1)
        [ "$want" = "$have" ] || fail "rust-toolchain.toml pins $want, $file uses $have"
        ;;
    deb)
        [ $# -eq 2 ] || fail "usage: ci/version.sh deb VERSION"
        deb_version "$2"
        ;;
    prerelease)
        [ $# -eq 2 ] || fail "usage: ci/version.sh prerelease VERSION"
        case "$2" in *-*) exit 0 ;; *) exit 1 ;; esac
        ;;
    *) fail "unknown check $1" ;;
esac
