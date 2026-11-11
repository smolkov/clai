#!/bin/sh
# Installs a Debian package in this (throw-away, root) container, checks it,
# and removes it again.
#
#   ci/deb-test.sh DEB
#
# Run it in debian:stable-slim or ubuntu:24.04, never on a machine you use:
# it installs and removes packages.

set -eu

[ $# -eq 1 ] || { echo "usage: ci/deb-test.sh DEB" >&2; exit 1; }
deb=$1
[ "$(id -u)" = 0 ] || { echo "ci/deb-test.sh must run as root in a container" >&2; exit 1; }
case "$deb" in /*) ;; *) deb="$PWD/$deb" ;; esac

fail() {
    echo "ci: error: $*" >&2
    exit 1
}

export DEBIAN_FRONTEND=noninteractive
apt-get install -y --no-install-recommends "$deb"

files="/usr/bin/clai
/usr/share/bash-completion/completions/clai
/usr/share/zsh/vendor-completions/_clai
/usr/share/fish/vendor_completions.d/clai.fish"
for f in $files; do
    [ -e "$f" ] || fail "$f is not installed"
done
# Slim images leave out /usr/share/doc when installing, so the documents are
# checked in the package itself.
docs="./usr/share/doc/clai/README.md
./usr/share/doc/clai/changelog.gz
./usr/share/doc/clai/changelog.Debian.gz
./usr/share/doc/clai/copyright"
listing=$(dpkg-deb -c "$deb" | awk '{print $6}')
for f in $docs; do
    printf '%s\n' "$listing" | grep -qxF "$f" || fail "${f#.} is not in the package"
done

version=$(dpkg-deb -f "$deb" Version)
shown=$(clai --version)
upstream=$(echo "${version%-*}" | sed 's/~/-/')
case "$shown" in
    "clai $upstream"*) echo "ci: $shown" >&2 ;;
    *) fail "clai --version says '$shown', the package is $version" ;;
esac

grep -q 'COMPLETE="bash"' /usr/share/bash-completion/completions/clai \
    || fail "the bash completion does not call clai"
if grep -q '/target/' /usr/share/bash-completion/completions/clai; then
    fail "the bash completion names a build directory"
fi
if command -v bash >/dev/null 2>&1; then
    bash -c '. /usr/share/bash-completion/completions/clai && complete -p clai' >/dev/null \
        || fail "bash does not register the completion"
fi

apt-get remove -y --purge clai
for f in $files; do
    [ ! -e "$f" ] || fail "$f is left after removing the package"
done
echo "ci: $deb installs, runs and removes cleanly" >&2
