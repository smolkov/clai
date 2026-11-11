# Shared helpers of the ci/ scripts (RFC 0020). Sourced, not run.
#
# Every script runs from the repository root, works the same on a
# developer's machine and in the GitHub and GitLab pipelines, and is POSIX sh.

set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

say() {
    printf '%s\n' "ci: $*" >&2
}

fail() {
    printf '%s\n' "ci: error: $*" >&2
    exit 1
}

# The `version` of the package in Cargo.toml (or $CLAI_CARGO_TOML).
crate_version() {
    sed -n '/^\[package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' "${CLAI_CARGO_TOML:-Cargo.toml}" | head -n 1
}

# The pinned toolchain in rust-toolchain.toml (or $CLAI_TOOLCHAIN_TOML).
toolchain_version() {
    sed -n 's/^channel = "\(.*\)"$/\1/p' "${CLAI_TOOLCHAIN_TOML:-rust-toolchain.toml}" | head -n 1
}

# The host's target triple.
host_target() {
    rustc -vV | sed -n 's/^host: //p'
}

# Whether a binary for target $1 runs on this host: the same triple, or the
# same architecture on Linux (a musl binary runs on a glibc host).
runs_here() {
    host=$(host_target)
    [ "$1" = "$host" ] && return 0
    case "$1:$host" in
        *-linux-*:*-linux-*) [ "${1%%-*}" = "${host%%-*}" ] ;;
        *) return 1 ;;
    esac
}

# The Debian architecture of target $1.
deb_arch() {
    case "$1" in
        x86_64-*) echo amd64 ;;
        aarch64-*) echo arm64 ;;
        *) fail "no Debian architecture for $1" ;;
    esac
}

# The Debian version of crate version $1: the first `-` becomes `~`, so a
# pre-release sorts before its release (0.2.0~rc.1 < 0.2.0).
deb_version() {
    printf '%s\n' "$1" | sed 's/-/~/'
}

# sha256 of the files given, in `sha256sum` format.
sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$@"
    else
        shasum -a 256 "$@"
    fi
}
