#!/bin/sh
# Packs the built binary of TARGET into dist/clai-VERSION-TARGET.tar.gz.
#
#   ci/package.sh TARGET [VERSION]
#
# Needs target/TARGET/release/clai (ci/build.sh). VERSION defaults to the
# version in Cargo.toml. Prints the archive's path.

. "$(dirname -- "$0")/common.sh"

[ $# -ge 1 ] && [ $# -le 2 ] || fail "usage: ci/package.sh TARGET [VERSION]"
target=$1
version=${2:-$(crate_version)}
bin="target/$target/release/clai"
[ -x "$bin" ] || fail "$bin missing; run ci/build.sh $target first"

name="clai-$version-$target"
stage="target/dist-stage/$name"
rm -rf "$stage"
mkdir -p "$stage/completions" dist

cp "$bin" "$stage/clai"
cp readme.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE "$stage/"
sh ci/completions.sh "$(sh ci/host-bin.sh "$target")" "$stage/completions"

archive="dist/$name.tar.gz"
if tar --version 2>/dev/null | grep -q GNU; then
    # The same files give the same archive.
    epoch=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct HEAD 2>/dev/null || echo 0)}
    tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$epoch" \
        -C target/dist-stage -cf - "$name" | gzip -9n >"$archive"
else
    tar -C target/dist-stage -cf - "$name" | gzip -9n >"$archive"
fi
say "packed $archive"
printf '%s\n' "$archive"
