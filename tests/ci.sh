#!/bin/sh
# Tests of the ci/ scripts that need no build: release notes, version and
# tag checks, Debian versions. Run by ci/test.sh.

set -eu

cd "$(dirname -- "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
failures=0

ok() {
    printf 'ok   %s\n' "$1"
}

bad() {
    printf 'FAIL %s\n' "$1"
    failures=$((failures + 1))
}

# expect NAME WANT COMMAND...: the command succeeds and prints WANT.
expect() {
    name=$1 want=$2
    shift 2
    if have=$("$@" 2>"$tmp/err"); then
        if [ "$have" = "$want" ]; then ok "$name"; else
            bad "$name: got '$have', want '$want'"
        fi
    else
        bad "$name: failed: $(cat "$tmp/err")"
    fi
}

# refuse NAME TEXT COMMAND...: the command fails, and says TEXT.
refuse() {
    name=$1 text=$2
    shift 2
    if "$@" >/dev/null 2>"$tmp/err"; then
        bad "$name: should fail"
    elif grep -qF -- "$text" "$tmp/err"; then
        ok "$name"
    else
        bad "$name: error lacks '$text': $(cat "$tmp/err")"
    fi
}

cat >"$tmp/CHANGELOG.md" <<'EOF'
# Changelog

## Unreleased

- Next thing.

## 0.2.0 - 2026-11-20

- Debian packages.
- Static builds.

## 0.2.0-rc.1 - 2026-11-15

- A candidate.

## 0.1.0 - 2026-10-01

## 0.0.9

- Old.
EOF

expect "notes of a release" "- Debian packages.
- Static builds." sh ci/notes.sh 0.2.0 "$tmp/CHANGELOG.md"
expect "notes of a pre-release" "- A candidate." sh ci/notes.sh 0.2.0-rc.1 "$tmp/CHANGELOG.md"
expect "notes without a date" "- Old." sh ci/notes.sh 0.0.9 "$tmp/CHANGELOG.md"
refuse "notes of an empty section" "no notes for 0.1.0" sh ci/notes.sh 0.1.0 "$tmp/CHANGELOG.md"
refuse "notes of a missing version" "no notes for 9.9.9" sh ci/notes.sh 9.9.9 "$tmp/CHANGELOG.md"
refuse "a version is not a prefix" "no notes for 0.2" sh ci/notes.sh 0.2 "$tmp/CHANGELOG.md"

printf '[package]\nname = "clai"\nversion = "0.2.0"\n\n[dependencies]\nversion = "1"\n' >"$tmp/Cargo.toml"
expect "tag matches" "0.2.0" env CLAI_CARGO_TOML="$tmp/Cargo.toml" sh ci/version.sh tag v0.2.0
refuse "tag without v" "expected v0.2.0" env CLAI_CARGO_TOML="$tmp/Cargo.toml" sh ci/version.sh tag 0.2.0
refuse "tag of another version" "does not match" env CLAI_CARGO_TOML="$tmp/Cargo.toml" sh ci/version.sh tag v0.2.1

expect "deb version of a release" "0.2.0" sh ci/version.sh deb 0.2.0
expect "deb version of a pre-release" "0.2.0~rc.1" sh ci/version.sh deb 0.2.0-rc.1
if command -v dpkg >/dev/null 2>&1; then
    if dpkg --compare-versions 0.2.0~rc.1-1 lt 0.2.0-1; then ok "pre-release sorts first"; else
        bad "0.2.0~rc.1-1 does not sort before 0.2.0-1"
    fi
fi
if sh ci/version.sh prerelease 0.2.0-rc.1; then ok "rc is a pre-release"; else bad "rc is a pre-release"; fi
if sh ci/version.sh prerelease 0.2.0; then bad "0.2.0 is a release"; else ok "0.2.0 is a release"; fi

printf '[toolchain]\nchannel = "1.99.1"\n' >"$tmp/rust-toolchain.toml"
printf 'variables:\n  RUST_VERSION: "1.99.1"\n' >"$tmp/same.yml"
printf 'variables:\n  RUST_VERSION: "1.98.0"\n' >"$tmp/other.yml"
expect "toolchain matches GitLab" "" env CLAI_TOOLCHAIN_TOML="$tmp/rust-toolchain.toml" \
    CLAI_GITLAB_CI="$tmp/same.yml" sh ci/version.sh toolchain
refuse "toolchain differs from GitLab" "uses 1.98.0" env CLAI_TOOLCHAIN_TOML="$tmp/rust-toolchain.toml" \
    CLAI_GITLAB_CI="$tmp/other.yml" sh ci/version.sh toolchain
expect "the repository's toolchain matches GitLab" "" sh ci/version.sh toolchain

if [ "$failures" -gt 0 ]; then
    echo "$failures ci script tests failed" >&2
    exit 1
fi
echo "ci script tests passed"
