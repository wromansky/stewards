#!/bin/sh
# Installs stewards from a GitHub release: sh install.sh [v<version>]  (default: the latest release).
# Installs under ${STEWARDS_PREFIX:-$HOME/.local}: bin/, lib/stewards/ (the BAML library), share/.
# STEWARDS_PREFIX must be an absolute path, and is required when running as root.
set -eu

repo=wromansky/stewards

fail() {
    echo "install.sh: $*" >&2
    exit 1
}

# https only, also across redirects.
fetch() {
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL "$@"
}

# Writes <source> to a new private file beside <destination> and renames it over it, so a running
# program never sees a half-written file and a planted name or symlink is not written through.
install_file() {
    new=$(mktemp "$2.XXXXXX") || fail "could not create a file beside $2"
    cp "$1" "$new" && chmod 755 "$new" && mv -f "$new" "$2" || {
        rm -f "$new"
        fail "could not install $2"
    }
}

main() {
    if [ -n "${STEWARDS_PREFIX:-}" ]; then
        prefix=$STEWARDS_PREFIX
    else
        [ "$(id -u)" -ne 0 ] || fail "refusing to install as root into \$HOME/.local; set STEWARDS_PREFIX (for example /usr/local)"
        prefix=${HOME:-}/.local
    fi
    case $prefix in
    /*) ;;
    *) fail "the install prefix must be an absolute path, got '$prefix'" ;;
    esac

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
        url=$(fetch -I -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest") ||
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
    trap 'rm -rf "$tmp"' EXIT
    trap 'exit 1' HUP INT TERM

    echo "downloading $name.tar.gz"
    fetch -o "$tmp/$name.tar.gz" "$base/$name.tar.gz" || fail "could not download $base/$name.tar.gz"
    fetch -o "$tmp/SHA256SUMS" "$base/SHA256SUMS" || fail "could not download $base/SHA256SUMS"

    expected=$(awk -v f="$name.tar.gz" '$2 == f { print $1 }' "$tmp/SHA256SUMS")
    [ -n "$expected" ] || fail "SHA256SUMS has no entry for $name.tar.gz"
    [ "$(sha256 "$tmp/$name.tar.gz")" = "$expected" ] || fail "checksum mismatch for $name.tar.gz; nothing installed"

    tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
    src=$tmp/$name
    [ -x "$src/bin/stewards" ] || fail "the archive has no bin/stewards"
    [ -d "$src/licenses" ] || fail "the archive has no licenses/"

    mkdir -p "$prefix/bin" "$prefix/lib/stewards" "$prefix/share/licenses/stewards"
    install_file "$src/bin/stewards" "$prefix/bin/stewards"
    for lib in "$src"/lib/stewards/*; do
        install_file "$lib" "$prefix/lib/stewards/${lib##*/}"
    done
    cp -R "$src/share/." "$prefix/share/"
    cp -R "$src/licenses/." "$prefix/share/licenses/stewards/"
    cp "$src"/LICENSE "$src"/THIRD-PARTY-LICENSES.html "$prefix/share/licenses/stewards/"

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
licenses: $prefix/share/licenses/stewards/
then run: stewards guide
MSG
}

main "$@"
