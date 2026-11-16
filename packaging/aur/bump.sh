#!/bin/sh
# Points PKGBUILD and .SRCINFO at a released crate: sh bump.sh <version> <sha256 of the .crate>.
# Only pkgver, pkgrel, and the crate's sha256sums entry change, so .SRCINFO gets the same edits
# instead of makepkg --printsrcinfo (which refuses root and needs Arch).
set -eu
cd "$(dirname "$0")"
version=${1:?usage: bump.sh <version> <sha256>}
sha=${2:?usage: bump.sh <version> <sha256>}
case "$version" in
"" | *[!0-9.]* | .* | *. | *..*) echo "bump.sh: not a plain version: $version" >&2; exit 1 ;;
esac
case "$sha" in
*[!0-9a-f]*) echo "bump.sh: not lowercase hex: $sha" >&2; exit 1 ;;
esac
[ "${#sha}" -eq 64 ] || { echo "bump.sh: a sha256 is 64 hex digits" >&2; exit 1; }

old=$(sed -n 's/^pkgver=//p' PKGBUILD)
oldsha=$(sed -n "s/^sha256sums=('\\([^']*\\)'\$/\\1/p" PKGBUILD)
[ -n "$old" ] && [ -n "$oldsha" ] || { echo "bump.sh: cannot read pkgver or sha256sums from PKGBUILD" >&2; exit 1; }

sed -i.bak -e "s/^pkgver=.*/pkgver=$version/" -e 's/^pkgrel=.*/pkgrel=1/' \
    -e "s/^sha256sums=('[^']*'/sha256sums=('$sha'/" PKGBUILD
sed -i.bak -e "s/^\\(	pkgver = \\).*/\\1$version/" -e 's/^\(	pkgrel = \).*/\11/' \
    -e "s/stewards-$old\\./stewards-$version./g" \
    -e "s/^\\(	sha256sums = \\)$oldsha\$/\\1$sha/" .SRCINFO
rm -f PKGBUILD.bak .SRCINFO.bak
