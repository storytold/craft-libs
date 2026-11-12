#!/usr/bin/env bash
# Replaces the vendored HarfBuzz source with the given release.
#
# Usage: scripts/update-harfbuzz.sh <version> <sha256>
#
# <sha256> is the digest of harfbuzz-<version>.tar.xz as listed on the
# release page (https://github.com/harfbuzz/harfbuzz/releases). The script
# downloads the tarball, checks the digest, copies src/ (minus tests,
# generators and build files) and COPYING into
# crates/harfbuzz-subset-sys/harfbuzz/, and records the release in that
# crate's README.
set -euo pipefail

if [ $# -ne 2 ]; then
    echo "usage: $0 <version> <sha256>" >&2
    exit 2
fi
version=$1
sha256=$2

root=$(cd "$(dirname "$0")/.." && pwd)
crate=$root/crates/harfbuzz-subset-sys
dest=$crate/harfbuzz
url=https://github.com/harfbuzz/harfbuzz/releases/download/$version/harfbuzz-$version.tar.xz

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL -o "$tmp/harfbuzz.tar.xz" "$url"
if command -v sha256sum >/dev/null; then
    actual=$(sha256sum "$tmp/harfbuzz.tar.xz" | cut -d' ' -f1)
else
    actual=$(shasum -a 256 "$tmp/harfbuzz.tar.xz" | cut -d' ' -f1)
fi
if [ "$actual" != "$sha256" ]; then
    echo "sha256 mismatch: expected $sha256, got $actual" >&2
    exit 1
fi

tar -xJf "$tmp/harfbuzz.tar.xz" -C "$tmp"
src=$tmp/harfbuzz-$version/src

# Not part of the harfbuzz-subset.cc build: the Rust and WebAssembly
# shapers, generator inputs and scripts, unit tests, meson files, and the
# other amalgamations (linking one of them next to harfbuzz-subset.cc
# would duplicate every symbol).
rm -rf "$src/rust" "$src/wasm" "$src/ms-use"
find "$src" -type f \( \
    -name 'test-*.cc' -o -name test.cc -o -name main.cc -o -name failing-alloc.c \
    -o -name harfbuzz.cc -o -name 'harfbuzz-world.cc*' \
    -o -name '*.py' -o -name meson.build -o -name '*.in' -o -name '*.make' -o -name '*.txt' \
    \) -exec rm -f {} +

rm -rf "$dest/src"
mkdir -p "$dest"
cp -R "$src" "$dest/src"
cp "$tmp/harfbuzz-$version/COPYING" "$dest/COPYING"

readme=$crate/README.md
sed -e "s/^| Version | .* |\$/| Version | $version |/" \
    -e "s#^| URL | .* |\$#| URL | $url |#" \
    -e "s/^| sha256 | .* |\$/| sha256 | \`$sha256\` |/" \
    "$readme" >"$tmp/README.md"
cat "$tmp/README.md" >"$readme"

echo "Vendored HarfBuzz $version ($(find "$dest/src" -type f | wc -l | tr -d ' ') files)."
