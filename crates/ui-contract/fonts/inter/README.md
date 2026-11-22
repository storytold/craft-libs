# Inter

The suite's interface face, under the SIL Open Font License 1.1 (`OFL.txt`; Inter has no Reserved Font Name).

- `Inter-Regular.ttf`, `Inter-SemiBold.ttf`: Inter 4.001 (git-9221beed3), subset to Latin, Greek and Cyrillic.
- `InterKeys-Regular.ttf`, `InterKeys-SemiBold.ttf`: the macOS key symbols (U+2318 ⌘, U+2325 ⌥, U+2303 ⌃, U+21E7 ⇧,
  U+232B ⌫, U+2326 ⌦, U+23CE ⏎, U+21A9 ↩, U+238B ⎋, U+21E5 ⇥, U+21EA ⇪; the subset has only ⇧ ↩ ⇥ ⇪, and the monospace
  family has no Inter), cut from the same Inter release as storytold/craft-fonts carries
  it (`fonts/inter/Inter-Regular.ttf` sha256 40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82, `Inter-SemiBold.ttf` 78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3) with this
  workspace's `craft_font_subset::cut_face`. They sit behind Inter in the default families, so every other character
  keeps Inter's subset glyphs and metrics.
