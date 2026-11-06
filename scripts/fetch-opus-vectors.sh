#!/usr/bin/env bash
# Fetch the Opus RFC 8251 conformance vectors (opus_testvectors-rfc8251, 75 MB) into DIR
# (default target/fixtures/opus/vectors), checked against their SHA-256. craft-audio's Opus
# conformance test reads them from there; they are test data, never committed.
#
# The archive is fetched in parallel byte ranges, and a range whose connection slows to a crawl is
# dropped and fetched again: opus-codec.org throttles some connections to a few KB/s.
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

# Small ranges, eight at a time: a range whose connection drops below 50 KB/s for 15 s is
# abandoned and fetched again, which costs little for a 2 MB range.
chunk=2000000
ranges=()
for ((start = 0; start < len; start += chunk)); do
    end=$((start + chunk - 1))
    ((end < len)) || end=$((len - 1))
    ranges+=("$start-$end")
done
fetch() { # fetch RANGE PART
    curl -fsS --retry 20 --retry-delay 1 --retry-all-errors --speed-limit 50000 --speed-time 15 -r "$1" -o "$2" "$URL"
}
export -f fetch
export URL
for i in "${!ranges[@]}"; do
    printf '%s %s/part.%04d\n' "${ranges[$i]}" "$tmp" "$i"
done | xargs -P 8 -n 2 bash -c 'fetch "$0" "$1"' || true
# Fetch again any range that came back short, one at a time.
for attempt in 1 2 3 4 5; do
    short=0
    for i in "${!ranges[@]}"; do
        range=${ranges[$i]}
        want=$((${range#*-} - ${range%-*} + 1))
        part=$(printf '%s/part.%04d' "$tmp" "$i")
        if [ "$(wc -c < "$part" 2> /dev/null || echo 0)" -ne "$want" ]; then
            short=1
            fetch "$range" "$part" || true
        fi
    done
    [ "$short" -eq 0 ] && break
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
