#!/usr/bin/env bash
# Regenerates the number and plural data in this directory from Unicode CLDR, for every CLDR
# locale: 10 KB, of which CLDR's modern-coverage locales alone would be 7 KB. Lookups fall back
# at run time, so a locale whose data matches its parent's is left out.
#
# Needs `cargo install icu4x-datagen --version 2.3.0` (the ICU4X minor version in Cargo.toml)
# and python3. Downloads CLDR's JSON release once into $CACHE.
set -euo pipefail

CLDR=48.2.1
HERE=$(cd "$(dirname "$0")" && pwd)
CACHE=${CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/craft-i18n-data}
mkdir -p "$CACHE"

TMPDIR=$CACHE icu4x-datagen --format blob --overwrite \
    --cldr-tag "$CLDR" \
    --markers DecimalSymbolsV1 DecimalDigitsV1 PluralsCardinalV1 \
    --locales full --deduplication maximal \
    --out "$HERE/numbers.postcard"

ZIP=$CACHE/icu4x-source-cache/github.com/unicode-org/cldr-json/releases/download/$CLDR/cldr-$CLDR-json-full.zip
python3 -I "$HERE/locales.py" "$ZIP" "$CLDR" > "$HERE/locales.rs.tmp"
mv "$HERE/locales.rs.tmp" "$HERE/locales.rs"
