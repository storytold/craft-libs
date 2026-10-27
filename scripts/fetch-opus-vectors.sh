#!/usr/bin/env bash
# Fetch the Opus RFC 8251 conformance vectors (opus_testvectors-rfc8251, 75 MB) into DIR
# (default target/fixtures/opus/vectors), checked against their SHA-256. craft-audio's Opus
# conformance test reads them from there; they are test data, never committed.
#
# The archive is fetched in parallel byte ranges: a single connection to opus-codec.org can stall.
set -euo pipefail

URL=https://opus-codec.org/static/testvectors/opus_testvectors-rfc8251.tar.gz
SHA256=6b26a22f9ba87b2b836906a9bb7afec5f8e54d49553b1200382520ee6fedfa55
DEST=${1:-target/fixtures/opus/vectors}

if [ -f "$DEST/testvector01.bit" ] && [ -f "$DEST/testvector12m.dec" ]; then
    echo "Opus vectors already in $DEST"
    exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

len=$(curl -fsSIL "$URL" | tr -d '\r' | awk 'tolower($1) == "content-length:" { n = $2 } END { print n }')
[ -n "$len" ] || { echo "could not read the archive size" >&2; exit 1; }

chunk=8000000
pids=()
i=0
for ((start = 0; start < len; start += chunk)); do
    end=$((start + chunk - 1))
    ((end < len)) || end=$((len - 1))
    curl -fsS --retry 5 --retry-all-errors -r "$start-$end" -o "$(printf '%s/part.%03d' "$tmp" "$i")" "$URL" &
    pids+=($!)
    i=$((i + 1))
done
for pid in "${pids[@]}"; do
    wait "$pid"
done
cat "$tmp"/part.* > "$tmp/vectors.tar.gz"

if command -v sha256sum > /dev/null; then
    echo "$SHA256  $tmp/vectors.tar.gz" | sha256sum -c -
else
    echo "$SHA256  $tmp/vectors.tar.gz" | shasum -a 256 -c -
fi

mkdir -p "$DEST"
tar xzf "$tmp/vectors.tar.gz" -C "$DEST" --strip-components=1
echo "Opus vectors in $DEST"
