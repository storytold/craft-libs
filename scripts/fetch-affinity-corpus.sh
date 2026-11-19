#!/usr/bin/env bash
# Fetch the public Affinity documents craft-affinity's corpus tests read into DIR (default
# target/fixtures/affinity), each from its upstream repository at a pinned commit and checked
# against the SHA-256 in crates/affinity/tests/support/corpus.tsv. They are test data under
# CC0, MIT or Apache-2.0 (the licence of each is in that file), never committed.
#
# Run the tests on them with AFFINITY_CORPUS=DIR cargo test -p craft-affinity --all-features.
set -euo pipefail

cd "$(dirname "$0")/.."
DEST=${1:-target/fixtures/affinity}
mkdir -p "$DEST"
while IFS=$'\t' read -r sha name _licence url; do
    out="$DEST/$name"
    if ! echo "$sha  $out" | sha256sum -c --status 2>/dev/null; then
        curl -fsSL --proto '=https' --retry 3 -o "$out" "$url"
        echo "$sha  $out" | sha256sum -c --status || { rm -f "$out"; echo "$name: SHA-256 does not match" >&2; exit 1; }
    fi
done < crates/affinity/tests/support/corpus.tsv
echo "Affinity corpus verified in $DEST"
