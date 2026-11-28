#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
Installs blt, the Billet command, from a GitHub release.

Usage: install.sh [--version X.Y.Z] [--dir DIR]

  --version X.Y.Z   install that release instead of the latest one
  --dir DIR         install into DIR instead of ~/.local/bin

BLT_RELEASES_URL replaces https://github.com/EntropyLogix/Billet/releases,
for example to install from a mirror.
EOF
}

fail() {
    echo "install.sh: $1" >&2
    exit 1
}

releases=${BLT_RELEASES_URL:-https://github.com/EntropyLogix/Billet/releases}
version=
dir="$HOME/.local/bin"
while [ $# -gt 0 ]; do
    case $1 in
        --version)
            [ $# -ge 2 ] || fail "--version needs a value"
            version=${2#v}
            shift 2
            ;;
        --dir)
            [ $# -ge 2 ] || fail "--dir needs a value"
            dir=$2
            shift 2
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
done

for tool in curl tar; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is required but not installed"
done
if command -v sha256sum >/dev/null 2>&1; then
    digest() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
    digest() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
    fail "sha256sum or shasum is required but not installed"
fi

case "$(uname -s) $(uname -m)" in
    "Darwin arm64") target=aarch64-apple-darwin ;;
    "Darwin x86_64") target=x86_64-apple-darwin ;;
    "Linux x86_64" | "Linux amd64") target=x86_64-unknown-linux-musl ;;
    "Linux aarch64" | "Linux arm64") target=aarch64-unknown-linux-musl ;;
    *) fail "no release for $(uname -s) $(uname -m); build from source with cargo install" ;;
esac

if [ -z "$version" ]; then
    latest=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$releases/latest") ||
        fail "cannot find the latest release at $releases"
    version=${latest##*/v}
    [ "$version" != "$latest" ] || fail "cannot find the latest release at $releases"
fi

archive="blt-$version-$target.tar.gz"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

curl -fsSL -o "$work/SHA256SUMS" "$releases/download/v$version/SHA256SUMS" ||
    fail "cannot download SHA256SUMS for $version"
curl -fsSL -o "$work/$archive" "$releases/download/v$version/$archive" ||
    fail "cannot download $archive"
expected=$(awk -v file="$archive" '$2 == file { print $1 }' "$work/SHA256SUMS")
[ -n "$expected" ] || fail "SHA256SUMS has no entry for $archive"
[ "$(digest "$work/$archive")" = "$expected" ] || fail "checksum mismatch for $archive; nothing installed"

tar -C "$work" -xzf "$work/$archive"
binary="$work/blt-$version-$target/blt"
[ -f "$binary" ] || fail "$archive does not contain blt"

mkdir -p "$dir"
cp "$binary" "$dir/.blt.$$"
chmod 755 "$dir/.blt.$$"
mv "$dir/.blt.$$" "$dir/blt"
echo "installed blt $version to $dir/blt"
case ":$PATH:" in
    *":$dir:"*) ;;
    *) echo "add $dir to PATH to run blt" ;;
esac
