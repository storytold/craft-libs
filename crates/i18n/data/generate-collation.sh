#!/bin/sh
# Regenerates the collation data `collate` embeds, a postcard blob for the locales craft-i18n
# ships (its `locales/` directory) plus English, Persian and Hindi: each locale's default order,
# and Chinese pinyin and stroke orders.
# Needs icu4x-datagen at the version of the icu_collator dependency:
#   cargo install icu4x-datagen --version 2.3.0 --locked
set -eu
cd "$(dirname "$0")"
icu4x-datagen --format blob --overwrite --out collation.postcard --deduplication maximal \
  --cldr-tag 48.2.1 --icuexport-tag release-78.1rc \
  --attribute-filter 'collator=/^(stroke|pinyin)?$/' \
  --markers CollationRootV1 CollationTailoringV1 CollationDiacriticsV1 CollationJamoV1 \
    CollationMetadataV1 CollationReorderingV1 CollationSpecialPrimariesV1 \
  --locales en ar bg cs da de el es et fa fi fr hi hr hu id it ja ka kk ko lt nl pl pt-BR ru sl \
    sr sr-Latn sv te tr uk vi zh-Hans zh-Hant
