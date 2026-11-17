#!/bin/sh
# Release build for one target, with the commit and date in --version.
#
#   ci/build.sh TARGET
#
# Linux targets are musl and must come out static. A cross build (another
# architecture) uses `cargo zigbuild`; set CODEAGENT_BUILDER=cargo or zigbuild to
# choose. The C parts of the crypto library (aws-lc-sys) need a C compiler
# for the target: musl-gcc (Debian/Ubuntu: musl-tools) or zig.
# With cargo-auditable installed, the binary carries its dependency list
# (RFC 0037), for `cargo audit bin`; CODEAGENT_REQUIRE_AUDITABLE=1 makes it
# required, as the release pipelines do.
# Prints the path of the binary on standard output.

. "$(dirname -- "$0")/common.sh"

[ $# -eq 1 ] || fail "usage: ci/build.sh TARGET"
target=$1

if command -v rustup >/dev/null 2>&1; then
    rustup target add "$target" >&2
fi

if [ -z "${CODEAGENT_BUILD_COMMIT:-}" ] && git rev-parse --git-dir >/dev/null 2>&1; then
    CODEAGENT_BUILD_COMMIT=$(git rev-parse --short=7 HEAD)
    CODEAGENT_BUILD_DATE=$(git log -1 --format=%cs HEAD)
    export CODEAGENT_BUILD_COMMIT CODEAGENT_BUILD_DATE
fi

builder=${CODEAGENT_BUILDER:-}
if [ -z "$builder" ]; then
    case "$target" in
        *-linux-musl) if runs_here "$target"; then builder=cargo; else builder=zigbuild; fi ;;
        *) builder=cargo ;;
    esac
fi

# The C compiler of a native musl build: musl-gcc, unless one is set.
case "$builder:$target" in
    cargo:*-linux-musl)
        cc_var="CC_$(printf '%s' "$target" | tr '-' '_')"
        if eval "[ -z \"\${$cc_var:-}\" ]"; then
            command -v musl-gcc >/dev/null 2>&1 \
                || fail "musl-gcc not found; install musl-tools, or set $cc_var or CODEAGENT_BUILDER=zigbuild"
            eval "export $cc_var=musl-gcc"
        fi
        ;;
esac

# `cargo auditable ARGS` runs `cargo ARGS` with itself as the compiler
# wrapper, so it works for `zigbuild` too.
auditable=
if command -v cargo-auditable >/dev/null 2>&1; then
    auditable=auditable
elif [ "${CODEAGENT_REQUIRE_AUDITABLE:-}" = 1 ]; then
    fail "cargo-auditable is not installed: cargo install --locked cargo-auditable@$CARGO_AUDITABLE_VERSION"
fi

say "building $target with $builder${auditable:+ (auditable)} (${CODEAGENT_BUILD_COMMIT:-no commit})"
case "$builder" in
    cargo) cargo $auditable build --release --locked --target "$target" >&2 ;;
    zigbuild) cargo $auditable zigbuild --release --locked --target "$target" >&2 ;;
    *) fail "CODEAGENT_BUILDER must be cargo or zigbuild, not $builder" ;;
esac

bin="target/$target/release/codeagent"
[ -x "$bin" ] || fail "$bin was not built"

if [ -n "$auditable" ] && command -v readelf >/dev/null 2>&1; then
    case "$target" in
        *-linux-*)
            readelf -S "$bin" | grep -q 'dep-v0' || fail "$bin has no dependency list (.dep-v0)"
            say "dependency list: .dep-v0"
            ;;
    esac
fi

case "$target" in
    *-linux-*)
        kind=$(file -b "$bin")
        case "$kind" in
            *"statically linked"* | *"static-pie linked"*) say "static: $kind" ;;
            *) fail "$bin is not static: $kind" ;;
        esac
        ;;
esac

if runs_here "$target"; then
    "$bin" --version >&2
    "$bin" completion bash >/dev/null
else
    say "$target does not run here; not started"
fi

printf '%s\n' "$bin"
