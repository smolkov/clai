#!/bin/sh
# Version checks of a release.
#
#   ci/version.sh tag TAG     TAG is v plus the version in Cargo.toml;
#                             prints the version
#   ci/version.sh toolchain   .gitlab-ci.yml uses the image of the
#                             toolchain in rust-toolchain.toml
#   ci/version.sh tools       the pipelines install cargo-deny and
#                             cargo-auditable at the versions of
#                             ci/common.sh
#   ci/version.sh deb VERSION prints the Debian version of VERSION
#   ci/version.sh prerelease VERSION
#                             exits 0 when VERSION is a pre-release

. "$(dirname -- "$0")/common.sh"

[ $# -ge 1 ] || fail "usage: ci/version.sh tag TAG | toolchain | tools | deb VERSION | prerelease VERSION"

# Every version of tool $1 that the pipelines name equals $2: GitLab's
# variable $3, and each `$1@VERSION` in the GitHub workflows.
same_tool() {
    gitlab=${CODEAGENT_GITLAB_CI:-.gitlab-ci.yml}
    workflows=${CODEAGENT_WORKFLOWS:-.github/workflows}
    have=$(sed -n "s/^ *$3: \"\(.*\)\"\$/\1/p" "$gitlab" | head -n 1)
    [ "$have" = "$2" ] || fail "ci/common.sh pins $1 $2, $gitlab uses ${have:-none}"
    found=$(grep -ho "$1@[0-9][0-9.]*" "$workflows"/*.yml | sort -u)
    [ -n "$found" ] || fail "$workflows does not install $1"
    for use in $found; do
        [ "$use" = "$1@$2" ] || fail "ci/common.sh pins $1 $2, $workflows uses $use"
    done
}
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
        file=${CODEAGENT_GITLAB_CI:-.gitlab-ci.yml}
        have=$(sed -n 's/^ *RUST_VERSION: "\(.*\)"$/\1/p' "$file" | head -n 1)
        [ "$want" = "$have" ] || fail "rust-toolchain.toml pins $want, $file uses $have"
        ;;
    tools)
        same_tool cargo-deny "$CARGO_DENY_VERSION" CARGO_DENY_VERSION
        same_tool cargo-auditable "$CARGO_AUDITABLE_VERSION" CARGO_AUDITABLE_VERSION
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
