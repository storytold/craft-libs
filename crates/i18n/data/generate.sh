#!/usr/bin/env bash
# Regenerates the number, plural (cardinal and ordinal) and date data in this directory from Unicode
# CLDR, for every CLDR locale: 11 KB of numbers and plural rules and 427 KB of dates. Lookups fall
# back at run time, so a locale whose data matches its parent's is left out.
#
# Needs `cargo install icu4x-datagen --version 2.4.0 --locked` (the ICU4X minor version in Cargo.toml,
# which the script checks) and python3. Downloads CLDR's JSON release once into $CACHE. The releases
# the data came from are written to source.txt: the ICU4X minor version and CLDR release a build of
# this crate holds, and the one other tables built from the same CLDR (language names, quotation
# marks) take, so they cannot disagree with the data here.
set -euo pipefail

# ICU4X 2.4's own tested CLDR release (`SourceDataProvider::TESTED_CLDR_TAG`).
ICU4X=2.4.0
CLDR=49.0.0
HERE=$(cd "$(dirname "$0")" && pwd)
CACHE=${CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/craft-i18n-data}
mkdir -p "$CACHE"

[ "$(icu4x-datagen --version | awk '{print $2}')" = "$ICU4X" ] || {
    echo "icu4x-datagen $ICU4X is needed (cargo install icu4x-datagen --version $ICU4X --locked)" >&2
    exit 1
}

TMPDIR=$CACHE icu4x-datagen --format blob --overwrite \
    --cldr-tag "$CLDR" \
    --markers DecimalSymbolsV1 DecimalDigitsV1 PluralsCardinalV1 PluralsOrdinalV1 \
    --locales full --deduplication maximal \
    --out "$HERE/numbers.postcard"

ZIP=$CACHE/icu4x-source-cache/github.com/unicode-org/cldr-json/releases/download/$CLDR/cldr-$CLDR-json-full.zip
# Gregorian, Buddhist and Persian dates and times, month and weekday names, week data and the
# regional default calendar (CalendarPreferredV1). The markers are those the date formatters, name
# patterns and WeekInformation load; digits come from the decimal markers.
TMPDIR=$CACHE icu4x-datagen --format blob --overwrite \
    --cldr-tag "$CLDR" \
    --markers CalendarWeekV1 CalendarPreferredV1 DatetimeNamesDayperiodV1 \
    DatetimeNamesMonthGregorianV1 DatetimeNamesMonthBuddhistV1 DatetimeNamesMonthPersianV1 \
    DatetimeNamesWeekdayV1 DatetimeNamesYearGregorianV1 DatetimeNamesYearBuddhistV1 \
    DatetimeNamesYearPersianV1 DatetimePatternsDateGregorianV1 DatetimePatternsDateBuddhistV1 \
    DatetimePatternsDatePersianV1 DatetimePatternsGlueV1 DatetimePatternsTimeV1 DecimalDigitsV1 DecimalSymbolsV1 \
    --locales full --deduplication maximal \
    --out "$HERE/dates.postcard"

python3 -I "$HERE/locales.py" "$ZIP" "$CLDR" > "$HERE/locales.rs.tmp"
mv "$HERE/locales.rs.tmp" "$HERE/locales.rs"

printf 'icu4x-datagen %s\ncldr %s\n' "$ICU4X" "$CLDR" > "$HERE/source.txt"
