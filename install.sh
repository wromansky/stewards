#!/bin/sh
# Installs stewards from a GitHub release: sh install.sh [v<version>]  (default: the latest release).
# Installs under ${STEWARDS_PREFIX:-$HOME/.local}: bin/, lib/stewards/ (the BAML library), share/.
set -eu

repo=wromansky/stewards
prefix=${STEWARDS_PREFIX:-$HOME/.local}

fail() {
    echo "install.sh: $*" >&2
    exit 1
}

command -v curl >/dev/null 2>&1 || fail "curl is required"

case "$(uname -s)-$(uname -m)" in
Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
Darwin-arm64) target=aarch64-apple-darwin ;;
Darwin-x86_64) target=x86_64-apple-darwin ;;
*) fail "no release for $(uname -s) $(uname -m); try: cargo install stewards" ;;
esac
# The Linux archives are built against glibc.
case "$target" in
*-linux-gnu) ls /lib/ld-musl-* >/dev/null 2>&1 && fail "this is a musl system (Alpine); try: cargo install stewards" ;;
esac

tag=${1:-}
if [ -z "$tag" ]; then
    url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest") ||
        fail "could not find the latest release"
    tag=${url##*/}
fi
# The tag goes into URLs and file names: only a plain version is accepted.
case "$tag" in
v[0-9]*) ;;
[0-9]*) tag=v$tag ;;
*) fail "expected a version like v0.1.0, got '$tag'" ;;
esac
case "$tag" in
*[!0-9A-Za-z.+-]* | *..*) fail "expected a version like v0.1.0, got '$tag'" ;;
esac

if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
    fail "sha256sum or shasum is required to verify the download"
fi

name=stewards-${tag#v}-$target
base=https://github.com/$repo/releases/download/$tag
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

echo "downloading $name.tar.gz"
curl -fsSL -o "$tmp/$name.tar.gz" "$base/$name.tar.gz" || fail "could not download $base/$name.tar.gz"
curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS" || fail "could not download $base/SHA256SUMS"

expected=$(awk -v f="$name.tar.gz" '$2 == f { print $1 }' "$tmp/SHA256SUMS")
[ -n "$expected" ] || fail "SHA256SUMS has no entry for $name.tar.gz"
[ "$(sha256 "$tmp/$name.tar.gz")" = "$expected" ] || fail "checksum mismatch for $name.tar.gz; nothing installed"

tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
src=$tmp/$name
[ -x "$src/bin/stewards" ] || fail "the archive has no bin/stewards"

mkdir -p "$prefix/bin" "$prefix/lib/stewards" "$prefix/share"
cp "$src/bin/stewards" "$prefix/bin/stewards.new" && mv -f "$prefix/bin/stewards.new" "$prefix/bin/stewards"
cp "$src"/lib/stewards/* "$prefix/lib/stewards/"
cp -R "$src/share/." "$prefix/share/"

echo "installed stewards $tag in $prefix"
case ":$PATH:" in
*":$prefix/bin:"*) ;;
*) echo "next: add it to your PATH: export PATH=\"$prefix/bin:\$PATH\"" ;;
esac
cat <<MSG
completions and the man page are installed under $prefix/share; enable them for your shell:
  bash: loaded by bash-completion when the prefix is ~/.local; otherwise: source $prefix/share/bash-completion/completions/stewards
  zsh:  fpath+=($prefix/share/zsh/site-functions) before compinit in ~/.zshrc
  fish: set -p fish_complete_path $prefix/share/fish/vendor_completions.d (in config.fish)
man page: $prefix/share/man/man1/stewards.1 (man finds it when $prefix/bin is on PATH)
then run: stewards guide
MSG
