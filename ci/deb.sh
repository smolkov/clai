#!/bin/sh
# Builds the Debian package of TARGET from its built binary, and checks it
# with lintian.
#
#   ci/deb.sh TARGET [VERSION]
#
# Needs target/TARGET/release/clai (ci/build.sh) and cargo-deb. lintian is
# required when CLAI_REQUIRE_LINTIAN=1 (the pipelines); otherwise skipped
# when missing. Prints the package's path.

. "$(dirname -- "$0")/common.sh"

[ $# -ge 1 ] && [ $# -le 2 ] || fail "usage: ci/deb.sh TARGET [VERSION]"
target=$1
version=${2:-$(crate_version)}
case "$target" in
    *-linux-musl) ;;
    *) fail "Debian packages are built from the static musl binaries, not $target" ;;
esac
bin="target/$target/release/clai"
[ -x "$bin" ] || fail "$bin missing; run ci/build.sh $target first"

assets=target/deb-assets
rm -rf "$assets"
mkdir -p "$assets" dist
sh ci/completions.sh "$(sh ci/host-bin.sh "$target")" "$assets"
debversion="$(deb_version "$version")-1"

# The project's changelog, and the short Debian changelog Debian policy
# asks for, pointing to it.
gzip -9n <CHANGELOG.md >"$assets/changelog.gz"
maintainer=$(sed -n 's/^maintainer = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
[ -n "$maintainer" ] || fail "no maintainer in [package.metadata.deb]"
epoch=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct HEAD 2>/dev/null || date +%s)}
when=$(LC_ALL=C date -u -R -d "@$epoch" 2>/dev/null || LC_ALL=C date -u -R -r "$epoch")
{
    printf 'clai (%s) unstable; urgency=medium\n\n' "$debversion"
    printf '  * Release %s; the changes are in\n' "$version"
    printf '    /usr/share/doc/clai/changelog.gz.\n\n'
    printf ' -- %s  %s\n' "$maintainer" "$when"
} | gzip -9n >"$assets/changelog.Debian.gz"
arch=$(deb_arch "$target")
deb="dist/clai_${debversion}_${arch}.deb"
cargo deb --no-build --no-strip --target "$target" --deb-version "$debversion" \
    --output "$deb" >&2

if command -v lintian >/dev/null 2>&1; then
    say "lintian $deb"
    lintian --fail-on error,warning "$deb" >&2
elif [ "${CLAI_REQUIRE_LINTIAN:-0}" = 1 ]; then
    fail "lintian is required (CLAI_REQUIRE_LINTIAN=1) but not installed"
else
    say "lintian not installed; not checked"
fi

say "built $deb"
printf '%s\n' "$deb"
