# Explicit Khmer data builds

`craft-khmer` builds offline without data by default. For a Khmer language build,
stage the hidden `km` sources from the craft-i18n manifest using its existing
`language-packs` tool:

```sh
cargo run --bin language-packs -- stage --locale km --out /absolute/khmer-pack \
  --cache /absolute/language-data-cache --no-download
CRAFT_KHMER_DIR=/absolute/khmer-pack CRAFT_KHMER_REQUIRED=1 cargo test -p craft-khmer
```

Use a fresh output directory whose parent already exists. The stage command refuses existing destinations, permits hidden packs, validates every downloaded/cached source,
and writes unchanged raw files and licences. Omit `--no-download` when populating
the cache. This is a developer build helper, not an app download mechanism.

The build script never downloads. Every supplied raw file and the upstream MIT
notice must match `source-pins.json`, pinned to Aksara revision
`31740e9d51d38171ae6119fc2d18a4a18fbfb2f1`. JSON object order is preserved;
weights use round-trip f64 parsing, preserving integer versus float rendering.
The pinned TypeScript affix configuration is parsed as data, never executed.
Each derived canonical text is SHA-checked against the former WordCraft canonical
output before Brotli compression (quality 11, window 22). Originals, compressed
bytes and CRC/length descriptors are generated only in `OUT_DIR`.

The script emits `cfg(craft_khmer_data)` only when all supplied sources validate.
Without a directory, it emits six empty assets. `CRAFT_KHMER_REQUIRED=1` rejects
an absent directory; any explicitly supplied incomplete or altered directory
fails regardless of that flag. Cargo reruns on source or environment changes.

Only hashes and provenance are committed here. Raw Khmer data remains outside
repositories. Runtime installation of these custom source kinds is pending
craft-libs#223, so craft-i18n keeps the pack hidden from published indexes.
