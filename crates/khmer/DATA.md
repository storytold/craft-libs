# Explicit Khmer data builds

`craft-khmer` builds offline without data by default. Data embedding is currently
**disabled**, pending inherited frequency-source notices, maintainer Embeddable
sign-off and the policy decision for trained models and converted configuration.
See [PROVENANCE.md](PROVENANCE.md). Any supplied `CRAFT_KHMER_DIR` fails the build;
there is no bypass. Run ordinary tests with both `CRAFT_KHMER_DIR` and
`CRAFT_KHMER_REQUIRED` unset.

The proposed staging integration is [craft-i18n #122](https://github.com/storytold/craft-i18n/pull/122),
also unmerged. It pins the six raw sources and the SBBIC/SIL notices, and preserves
the supplied SEAlang notice. `language-packs stage --locale km` currently rejects
the Download-only pack before fetching or writing it. For source inspection use
the pinned upstream URLs in that manifest or its verified cache.

After provenance resolution and maintainer approval in both repositories, stage
an Embeddable pack from the companion data checkout into a fresh directory:

```sh
cargo run --bin language-packs -- stage --locale km --out /absolute/khmer-pack \
  --cache /absolute/language-data-cache --no-download
```

That future command requires a new output directory whose parent exists. Staging
validates every raw source and writes unchanged files and all recorded notices.
Omit `--no-download` to populate the cache. This is a proposed developer build
helper, not an available app download mechanism. These instructions describe the
code in this PR; they become applicable only after both companions land and the
classification is approved.

The build script never downloads. Every supplied raw file and the upstream MIT
notice must match `source-pins.json`, pinned to Aksara revision
`31740e9d51d38171ae6119fc2d18a4a18fbfb2f1`. JSON object order is preserved;
weights use round-trip f64 parsing, preserving integer versus float rendering.
The pinned TypeScript affix configuration is parsed as data, never executed.
Each derived canonical text is SHA-checked against the former WordCraft canonical
output before Brotli compression (quality 11, window 22). Originals, compressed
bytes and CRC/length descriptors are generated only in `OUT_DIR`.

After a future maintainer-approved Embeddable classification, the script may emit
`cfg(craft_khmer_data)` only when all supplied sources validate.
Without a directory, it emits six empty assets. `CRAFT_KHMER_REQUIRED=1` rejects
an absent directory; any explicitly supplied incomplete or altered directory
fails regardless of that flag. Cargo reruns on source or environment changes.

Only hashes and provenance are committed here. Raw Khmer data remains outside
repositories. Runtime installation of these custom source kinds is pending
craft-libs#223, so craft-i18n keeps the pack hidden from published indexes.

The existing proofing workflow has a separate Khmer job that fetches the same pinned raw
sources and licence, verifies their SHA256 hashes, and runs all `craft-khmer`
tests plus `craft-proof`'s Khmer integration tests with `CRAFT_KHMER_REQUIRED=1`.
It does not rely on the optional pack being present in the general proofing job.

That explicit-data job will reject the pending Download-only pack until the
maintainer-approved classification changes; it must not be reported as a green
release gate while this provenance decision remains open.
