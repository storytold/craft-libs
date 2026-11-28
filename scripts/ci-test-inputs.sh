#!/usr/bin/env bash
# The test inputs the Workspace gate and the nightly full run fetch before their tests: the
# PhotoCraft test corpora (PSD, PSD tools, ATN, EXR, HEIF, PngSuite) at a pinned PhotoCraft commit,
# and the licensed PDF test fonts. Exports CRAFT_PSD_CORPUS and CRAFT_PDF_TEST_FONT_DIR through
# $GITHUB_ENV. Run from the repository root.
#
# A corpus already in .test-inputs/photocraft/corpus (restored from the CI cache, which is keyed on
# this file) is used as it is, without the PhotoCraft checkout and its xtask build.
set -euo pipefail

# The PhotoCraft commit whose `cargo xtask corpus` pins the corpora. Changing it changes this file,
# which starts a new corpus cache entry.
photocraft=74330157cb601573ae4523a95bb9132822fc3918
fetcher=.test-inputs/photocraft

if [[ -z "$(ls -A "$fetcher/corpus" 2>/dev/null)" ]]; then
  rm -rf "$fetcher"
  mkdir -p "$fetcher"
  git -C "$fetcher" init --quiet
  git -C "$fetcher" remote add origin https://github.com/storytold/PhotoCraft.git
  git -C "$fetcher" fetch --quiet --depth=1 origin "$photocraft"
  git -C "$fetcher" checkout --quiet --detach FETCH_HEAD
  (cd "$fetcher" && cargo xtask corpus --psd --psd-tools --atn --exr --heif --pngsuite)
fi
mkdir -p corpus
cp -a "$fetcher/corpus/." corpus/

python scripts/test-fetch-pdf-test-fonts.py
python scripts/fetch-pdf-test-fonts.py "$RUNNER_TEMP/craft-pdf-test-fonts"

printf 'CRAFT_PSD_CORPUS=%s\n' "$GITHUB_WORKSPACE/$fetcher/corpus" >> "$GITHUB_ENV"
printf 'CRAFT_PDF_TEST_FONT_DIR=%s\n' "$RUNNER_TEMP/craft-pdf-test-fonts" >> "$GITHUB_ENV"
