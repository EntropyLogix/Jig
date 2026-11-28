#!/bin/sh
set -eu

if [ $# -ne 2 ]; then
    echo "usage: packaging/homebrew/formula.sh VERSION SHA256SUMS" >&2
    exit 2
fi
version=$1
sums=$2
base="https://github.com/EntropyLogix/Billet/releases/download/v$version"

sum() {
    found=$(awk -v file="blt-$version-$1.tar.gz" '$2 == file { print $1 }' "$sums")
    if [ -z "$found" ]; then
        echo "formula.sh: $sums has no checksum for blt-$version-$1.tar.gz" >&2
        exit 1
    fi
    printf '%s' "$found"
}

asset() {
    checksum=$(sum "$1")
    printf '      url "%s/blt-%s-%s.tar.gz"\n      sha256 "%s"\n' "$base" "$version" "$1" "$checksum"
}

mac_arm=$(asset aarch64-apple-darwin)
mac_intel=$(asset x86_64-apple-darwin)
linux_arm=$(asset aarch64-unknown-linux-musl)
linux_intel=$(asset x86_64-unknown-linux-musl)

cat <<EOF
class Blt < Formula
  desc "File-based issue tracker and project memory for people and coding agents"
  homepage "https://github.com/EntropyLogix/Billet"
  version "$version"
  license "MIT"

  on_macos do
    on_arm do
$mac_arm
    end
    on_intel do
$mac_intel
    end
  end

  on_linux do
    on_arm do
$linux_arm
    end
    on_intel do
$linux_intel
    end
  end

  def install
    bin.install "blt"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/blt --version")
  end
end
EOF
